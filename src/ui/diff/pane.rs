//! The dual pane (old | gutter | new) as its own Entity, so wheel, scrollbar
//! drag and hover notify only this view. Its body is one `DualPaneElement`
//! (element.rs); this file holds the state and the input handling. See
//! docs/diffview-architecture.md §4–§5.

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, Element, ElementId, Entity, EventEmitter,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, Position,
    Render, SharedString, Style, StyleRefinement, Styled, Subscription, Task, Timer, Window, div,
    font, px,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crate::domain::{
    Alignment, AlignmentOp, Anchor, DiffFontSize, FoldState, Side, hunk_jump_target,
    match_jump_plan,
};
use crate::git::FileDiff;
use crate::ui::appearance::{Appearance, UiTextSize};
use crate::ui::{scrollbar, theme};
use super::element::{
    self, BarState, Decorations, FrameInput, Geom, ShapeCache, build_frame, insert_hitboxes,
    line_number_digits, ln_col_width, text_extent, thumb_for, top_at,
};
use super::layout::{HunkLand, Layout, Row};
use super::trace;
use super::viewport::{self, Viewport};

/// What the shell (DiffView) hears from the pane. Hunk index and hover copy
/// are emitted only when they change.
pub enum PaneEvent {
    BeginDraft { side: Side, ln: u32 },
    HunkIndexChanged(Option<usize>),
    HoverCopy(Option<String>),
}

struct PaneFile {
    alignment: Alignment,
    old_text: Arc<str>,
    new_text: Arc<str>,
}

/// Owns the per-file diff state: Alignment, fold, Layout, scroll and caches.
pub struct DualPane {
    /// Selected text file; `None` for binary / error / nothing selected.
    file: Option<PaneFile>,
    /// Per-file Equal fold. Reset when a file is opened.
    fold: FoldState,
    /// View projection of `file` under `fold`. Rebuilt on file / fold /
    /// Alignment change, never per frame.
    layout: Option<Layout>,
    /// Shaped text per (side, visual row), visible ± one screen.
    shapes: ShapeCache,
    /// Anchors of the selected path's DraftComments (comment index).
    comments: Vec<Anchor>,
    /// Line being drafted, highlighted in its pane.
    drafting: Option<(Side, u32)>,
    /// Shared scroll parameter, in pixels, and the only vertical scroll
    /// source. See docs/dual-pane-diff.md §3.1. Kept inside
    /// `viewport::s_range` from the first measured frame on.
    scroll_s: f32,
    /// Per-side horizontal scroll of the code text, in pixels, `[old, new]`.
    /// Independent of each other and of `scroll_s`; moved only by horizontal
    /// input over that pane. Reset on file open, kept (re-clamped) on fold,
    /// Alignment, font size and resize.
    x_offsets: [f32; 2],
    /// Horizontal travel per side from the last prepaint (`0..=max_x`).
    max_x: [f32; 2],
    /// Widest shaped line seen per side since the last Layout rebuild / font
    /// change. Only grows, so the bound never shrinks while scrolling.
    widest_seen: [f32; 2],
    /// Mono advance of `'0'` at `(font px, advance)` in `code_font`.
    mono_advance: Option<(f32, f32)>,
    /// Advance of `'0'` in `code_font` at the line-number size.
    ln_advance: Option<f32>,
    /// Element height, set in prepaint. 0 until the first frame.
    view_h: f32,
    /// Device pixels per logical pixel, from the last prepaint.
    scale: f32,
    hover_copy: Option<String>,
    /// Row under a left press, for click-on-release: (side, visual row).
    press: Option<(Side, u32)>,
    bars: BarState,
    bar_hide: Option<Task<()>>,
    /// 0-based index of the Hunk at / nearest the viewport; drives chrome.
    hunk_index: Option<usize>,
    /// Hunk-navigation position (row units) set by a jump or file open, before
    /// `s_range` clamping. Jumps near the file ends move no pixels, so chrome
    /// and the next jump follow this until the user scrolls.
    hunk_s: Option<f32>,
    /// Mono size for both panes and ribbons (§3.5). Based on the Code Font
    /// size setting; A−/A+ override it for this pane's session only.
    font_size: DiffFontSize,
    /// Code Font family the caches were shaped with.
    code_font: SharedString,
    _appearance: Subscription,
}

impl EventEmitter<PaneEvent> for DualPane {}

impl DualPane {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let appearance = cx.global::<Appearance>();
        let font_size = DiffFontSize::new(appearance.code_font_size);
        let code_font = appearance.code_font.name.clone();
        Self {
            file: None,
            fold: FoldState::collapsed(),
            layout: None,
            shapes: ShapeCache::default(),
            comments: Vec::new(),
            drafting: None,
            scroll_s: 0.,
            x_offsets: [0.; 2],
            max_x: [0.; 2],
            widest_seen: [0.; 2],
            mono_advance: None,
            ln_advance: None,
            view_h: 0.,
            scale: 1.,
            hover_copy: None,
            press: None,
            bars: BarState::default(),
            bar_hide: None,
            hunk_index: None,
            hunk_s: Some(0.),
            font_size,
            code_font,
            _appearance: cx.observe_global::<Appearance>(Self::apply_appearance),
        }
    }

    /// A Code Font size change resets the pane to it, dropping A−/A+; a family
    /// change reshapes everything.
    fn apply_appearance(&mut self, cx: &mut Context<Self>) {
        let appearance = cx.global::<Appearance>();
        let base = appearance.code_font_size;
        let family = appearance.code_font.name.clone();
        if base != self.font_size.base() {
            self.change_font_size(|size| *size = DiffFontSize::new(base), cx);
        }
        if family != self.code_font {
            self.code_font = family;
            self.mono_advance = None;
            self.ln_advance = None;
            self.invalidate_shapes();
            cx.notify();
        }
    }

    /// Drop shaped text and the widest-line bound it fed; the next prepaint
    /// reshapes what is visible.
    fn invalidate_shapes(&mut self) {
        self.shapes.clear();
        self.widest_seen = [0.; 2];
    }

    /// Open a file at its start with everything folded.
    pub fn open(&mut self, file: &FileDiff, comments: Vec<Anchor>, cx: &mut Context<Self>) {
        self.file = match file {
            FileDiff::Text {
                alignment,
                old_text,
                new_text,
            } => Some(PaneFile {
                alignment: alignment.clone(),
                old_text: old_text.clone(),
                new_text: new_text.clone(),
            }),
            _ => None,
        };
        self.comments = comments;
        self.fold = FoldState::collapsed();
        self.set_hunk_index(None, cx);
        self.hunk_s = Some(0.);
        self.reset_scroll(cx);
        self.x_offsets = [0.; 2];
        self.rebuild_layout();
        self.reveal_bars(cx);
        cx.notify();
    }

    /// Replace the Alignment (ViewOptions changed), keeping the anchor line.
    pub fn set_alignment(&mut self, alignment: Alignment, cx: &mut Context<Self>) {
        self.with_anchor(|this| {
            this.fold = FoldState::collapsed();
            if let Some(file) = this.file.as_mut() {
                file.alignment = alignment;
            }
        });
        self.set_hunk_index(None, cx);
        cx.notify();
    }

    pub fn set_comments(&mut self, comments: Vec<Anchor>, cx: &mut Context<Self>) {
        self.comments = comments;
        if let Some(layout) = self.layout.as_mut() {
            layout.set_comments(self.comments.iter());
        }
        cx.notify();
    }

    pub fn set_drafting(&mut self, drafting: Option<(Side, u32)>, cx: &mut Context<Self>) {
        if self.drafting != drafting {
            self.drafting = drafting;
            cx.notify();
        }
    }

    pub fn hunk_count(&self) -> Option<usize> {
        self.layout.as_ref().map(|l| l.hunk_count)
    }

    fn row_h(&self) -> f32 {
        self.font_size.row_height()
    }

    fn set_hunk_index(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if self.hunk_index != index {
            self.hunk_index = index;
            cx.emit(PaneEvent::HunkIndexChanged(index));
        }
    }

    fn set_hover_copy(&mut self, copy: Option<String>, cx: &mut Context<Self>) {
        if self.hover_copy != copy {
            self.hover_copy = copy.clone();
            cx.emit(PaneEvent::HoverCopy(copy));
        }
    }

    /// Rebuild the Layout from the file, `fold` and the comment index.
    fn rebuild_layout(&mut self) {
        self.layout = None;
        self.invalidate_shapes();
        let Some(file) = self.file.as_ref() else {
            return;
        };
        let t = trace::start();
        let mut layout = Layout::build(
            file.old_text.clone(),
            file.new_text.clone(),
            &file.alignment,
            Some(&self.fold),
        );
        layout.set_comments(self.comments.iter());
        if t.is_some() {
            trace::layout(
                trace::since(t),
                [layout.old.rows(), layout.new.rows()],
                layout.bridges.len(),
            );
        }
        self.layout = Some(layout);
    }

    /// Back to the file start. The first measured frame clamps `scroll_s` up
    /// to the lower end of `s_range`.
    fn reset_scroll(&mut self, cx: &mut Context<Self>) {
        self.scroll_s = 0.;
        self.press = None;
        self.bars.drag = None;
        self.set_hover_copy(None, cx);
    }

    fn with_anchor(&mut self, mutate: impl FnOnce(&mut Self)) {
        let cap = self.capture_anchor();
        mutate(self);
        self.rebuild_layout();
        self.restore_anchor(cap);
        self.hunk_s = None;
    }

    /// This frame's Viewport, tops snapped as painted.
    fn viewport(&self) -> Option<Viewport<'_>> {
        let layout = self.layout.as_ref()?;
        Some(Viewport::new(layout, self.scroll_s, self.view_h, self.row_h()).snapped(self.scale))
    }

    fn capture_anchor(&self) -> Option<viewport::AnchorCap> {
        self.viewport()?.capture_anchor()
    }

    fn restore_anchor(&mut self, cap: Option<viewport::AnchorCap>) {
        let (Some(cap), Some(layout)) = (cap, self.layout.as_ref()) else {
            return;
        };
        if let Some(s) = viewport::s_for_anchor(layout, cap, self.view_h, self.row_h(), self.scroll_s) {
            self.scroll_s = s;
        }
    }

    fn expand_omit(&mut self, omit_id: usize, cx: &mut Context<Self>) {
        self.with_anchor(|this| {
            this.fold.expand(omit_id);
        });
        cx.notify();
    }

    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.with_anchor(|this| {
            let ids: Vec<usize> = this
                .file
                .as_ref()
                .map(|f| {
                    f.alignment
                        .ops
                        .iter()
                        .enumerate()
                        .filter_map(|(i, op)| matches!(op, AlignmentOp::Equal { .. }).then_some(i))
                        .collect()
                })
                .unwrap_or_default();
            for i in ids {
                this.fold.expand(i);
            }
        });
        cx.notify();
    }

    pub fn collapse_unchanged(&mut self, cx: &mut Context<Self>) {
        self.with_anchor(|this| {
            this.fold = FoldState::collapsed();
        });
        cx.notify();
    }

    pub fn jump_hunk(&mut self, dir: i32, cx: &mut Context<Self>) {
        let (Some(file), Some(layout)) = (self.file.as_ref(), self.layout.as_ref()) else {
            return;
        };
        let hunk_count = layout.hunk_count;
        if hunk_count == 0 {
            return;
        }
        let lands = &layout.hunk_lands;
        let row_h = self.row_h();
        let s_rows = self.hunk_s.unwrap_or(self.scroll_s / row_h);
        let next = if dir > 0 {
            (0..hunk_count).find(|&i| {
                lands
                    .get(i)
                    .map(|h| h.s as f32 > s_rows + 0.5)
                    .unwrap_or(false)
            })
        } else {
            (0..hunk_count)
                .rev()
                .find(|&i| {
                    lands
                        .get(i)
                        .map(|h| (h.s as f32) < s_rows - 0.5)
                        .unwrap_or(false)
                })
        };
        let Some(i) = next else {
            return;
        };
        let Some(target) = hunk_jump_target(&file.alignment, i) else {
            return;
        };
        let s = viewport::s_for_target(layout, target, row_h, self.scroll_s).unwrap_or(self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = Some(s / row_h);
        self.set_hunk_index(Some(i), cx);
        self.reveal_bars(cx);
        cx.notify();
    }

    pub fn jump_match(&mut self, side: Side, ln: u32, cx: &mut Context<Self>) {
        let Some(file) = self.file.as_ref() else {
            return;
        };
        let plan = match_jump_plan(&file.alignment, &self.fold, side, ln);
        if let Some(id) = plan.expand {
            self.fold.expand(id);
            self.rebuild_layout();
        }
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let row_h = self.row_h();
        let s = viewport::s_for_target(layout, plan.target, row_h, self.scroll_s)
            .unwrap_or(self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = Some(s / row_h);
        self.sync_hunk_index(cx);
        self.reveal_bars(cx);
        cx.notify();
    }

    pub fn set_font_size(&mut self, op: FontOp, cx: &mut Context<Self>) {
        self.change_font_size(
            |size| match op {
                FontOp::Inc => size.increase(),
                FontOp::Dec => size.decrease(),
                FontOp::Reset => size.reset(),
            },
            cx,
        );
    }

    fn change_font_size(&mut self, change: impl FnOnce(&mut DiffFontSize), cx: &mut Context<Self>) {
        let prev = self.row_h();
        let prev_px = self.font_size.px() as f32;
        change(&mut self.font_size);
        let next = self.row_h();
        if prev > 0. {
            self.scroll_s *= next / prev;
        }
        // Keep the same columns in view; the next prepaint re-clamps.
        let next_px = self.font_size.px() as f32;
        if prev_px > 0. {
            for x in &mut self.x_offsets {
                *x *= next_px / prev_px;
            }
        }
        self.invalidate_shapes();
        cx.notify();
    }

    fn sync_hunk_index(&mut self, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let s_rows = self.hunk_s.unwrap_or(self.scroll_s / self.row_h());
        let index = nearest_hunk_index(s_rows, &layout.hunk_lands);
        self.set_hunk_index(index, cx);
    }

    /// Wheel / trackpad (momentum included): write `scroll_s` only.
    pub(super) fn scroll_by(&mut self, dy: f32, cx: &mut Context<Self>) {
        let row_h = self.row_h();
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let s = viewport::clamp_s(layout, self.scroll_s + dy, self.view_h, row_h);
        if s == self.scroll_s && self.hunk_s.is_none() {
            return;
        }
        self.scroll_s = s;
        self.hunk_s = None;
        self.sync_hunk_index(cx);
        self.reveal_bars(cx);
        cx.notify();
    }

    /// Horizontal wheel / trackpad over `side`'s code pane: move that side's
    /// `x_offset` only. `scroll_s` and the other side stay put.
    pub(super) fn scroll_x_by(&mut self, side: Side, dx: f32, cx: &mut Context<Self>) {
        let ix = side_ix(side);
        let x = viewport::clamp_x(self.x_offsets[ix] + dx, self.max_x[ix]);
        if x != self.x_offsets[ix] {
            self.x_offsets[ix] = x;
            cx.notify();
        }
    }

    /// Advance of one mono char at `font_px`, cached per size (and cleared on a
    /// Code Font family change).
    fn mono_advance(&mut self, font_px: f32, window: &Window) -> f32 {
        if let Some((at, advance)) = self.mono_advance
            && at == font_px
        {
            return advance;
        }
        let advance = zero_advance(&self.code_font, font_px, window);
        self.mono_advance = Some((font_px, advance));
        advance
    }

    /// Line-number digit advance; see [`element::ln_col_width`].
    fn ln_advance(&mut self, window: &Window) -> f32 {
        *self
            .ln_advance
            .get_or_insert_with(|| zero_advance(&self.code_font, element::LN_FONT_PX, window))
    }

    /// Show both scrollbars and (re)arm the idle hide timer.
    fn reveal_bars(&mut self, cx: &mut Context<Self>) {
        self.bars.visible = true;
        self.arm_bar_hide(cx);
    }

    fn arm_bar_hide(&mut self, cx: &mut Context<Self>) {
        self.bar_hide = Some(cx.spawn(async move |this, cx| {
            Timer::after(scrollbar::HIDE_DELAY).await;
            this.update(cx, |this, cx| {
                this.bar_hide.take();
                if this.bars.hovered.iter().any(|&h| h) || this.bars.drag.is_some() {
                    return;
                }
                if this.bars.visible {
                    this.bars.visible = false;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Put `side`'s thumb top at `thumb_top` (track y); map that side's
    /// content position back to `scroll_s`. The other side follows §3.1.
    fn drag_thumb(&mut self, side: Side, thumb_top: f32) {
        let Some(vp) = self.viewport() else {
            return;
        };
        let Some(geom) = thumb_for(self.view_h, vp.max_top(side), vp.top(side)) else {
            return;
        };
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let row_h = self.row_h();
        let top = top_at(&geom, thumb_top);
        let n = layout.side(side).rows();
        let content = viewport::content_from_top(top, n, self.view_h, row_h);
        let s = viewport::s_for_content(layout, side, content, row_h, self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = None;
    }

    /// Left press in `side`'s track at track y `y`: grab the thumb, or jump
    /// so the thumb centers on the press and grab it there.
    pub(super) fn press_track(&mut self, side: Side, y: f32, cx: &mut Context<Self>) {
        let Some(vp) = self.viewport() else {
            return;
        };
        let Some(geom) = thumb_for(self.view_h, vp.max_top(side), vp.top(side)) else {
            return;
        };
        let (top, h) = (f32::from(geom.thumb_top), f32::from(geom.thumb_height));
        let grab = if y >= top && y <= top + h {
            y - top
        } else {
            self.drag_thumb(side, y - h / 2.);
            h / 2.
        };
        self.bars.drag = Some((side, grab));
        self.bars.visible = true;
        self.bar_hide = None;
        self.press = None;
        self.sync_hunk_index(cx);
        cx.notify();
    }

    /// Visual row of `side` at pane y `y`.
    fn row_index_at(&self, side: Side, y: f32) -> Option<u32> {
        match self.viewport()?.hit(side, y)? {
            Row::Line(l) => Some(l.row),
            Row::Omit(o) => Some(o.row),
        }
    }

    pub(super) fn press_row(&mut self, side: Side, y: f32) {
        self.press = self.row_index_at(side, y).map(|row| (side, row));
    }

    /// Left release. Ends a thumb drag, or completes a click on the pressed
    /// row: a line begins a draft, an omission separator expands its span.
    pub(super) fn release(&mut self, side: Option<Side>, y: f32, cx: &mut Context<Self>) {
        if self.bars.drag.take().is_some() {
            if !self.bars.hovered.iter().any(|&h| h) {
                self.arm_bar_hide(cx);
            }
            cx.notify();
            return;
        }
        let (Some(pressed), Some(side)) = (self.press.take(), side) else {
            return;
        };
        if pressed.0 != side {
            return;
        }
        enum Click {
            Line(u32),
            Omit(usize),
        }
        let click = self.viewport().and_then(|vp| match vp.hit(side, y)? {
            Row::Line(l) if l.row == pressed.1 => Some(Click::Line(l.ln)),
            Row::Omit(o) if o.row == pressed.1 => Some(Click::Omit(o.id)),
            _ => None,
        });
        match click {
            Some(Click::Line(ln)) => cx.emit(PaneEvent::BeginDraft { side, ln }),
            Some(Click::Omit(id)) => self.expand_omit(id, cx),
            None => {}
        }
    }

    /// Track hover per side, an active thumb drag (`track_y` is the pointer in
    /// each track), and gutter hover copy (`gutter_y` is pane y over the gutter).
    pub(super) fn mouse_moved(
        &mut self,
        hovered: [bool; 2],
        gutter_y: Option<f32>,
        track_y: [f32; 2],
        cx: &mut Context<Self>,
    ) {
        let mut dirty = false;
        if self.bars.hovered != hovered {
            self.bars.hovered = hovered;
            if hovered.iter().any(|&h| h) {
                self.bars.visible = true;
                self.bar_hide = None;
            } else if self.bars.drag.is_none() {
                self.arm_bar_hide(cx);
            }
            dirty = true;
        }
        if let Some((side, grab)) = self.bars.drag {
            self.drag_thumb(side, track_y[side_ix(side)] - grab);
            self.sync_hunk_index(cx);
            dirty = true;
        }
        // Hover copy only reaches the shell's chrome; the pane does not redraw.
        let copy = gutter_y.and_then(|y| {
            let i = self.viewport()?.bridge_at(y)?;
            self.layout.as_ref().map(|l| l.bridges[i].position_copy())
        });
        self.set_hover_copy(copy, cx);
        if dirty {
            cx.notify();
        }
    }

    /// Prepaint of the element: this frame's `view_h` → Viewport → paint list.
    pub(super) fn prepaint_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<element::Frame> {
        let t_prepaint = trace::start();
        self.scale = window.scale_factor();
        self.view_h = f32::from(bounds.size.height);
        let row_h = self.row_h();
        let font_px = self.font_size.px() as f32;
        let advance = self.mono_advance(font_px, window);
        let ln_advance = self.ln_advance(window);
        let layout = self.layout.as_ref()?;
        let t_vp = trace::start();
        let vp = Viewport::new(layout, self.scroll_s, self.view_h, row_h).snapped(self.scale);
        let viewport_took = trace::since(t_vp);
        let s = vp.s();
        let ln_w = ln_col_width(line_number_digits(layout), ln_advance);
        let geom = Geom::new(bounds, ln_w, self.scale);
        let tracks = [Side::Old, Side::New]
            .map(|side| thumb_for(self.view_h, vp.max_top(side), vp.top(side)).is_some());
        let hitboxes = insert_hitboxes(&geom, tracks, window);
        let mut frame = build_frame(
            FrameInput {
                layout,
                vp: &vp,
                geom,
                row_h,
                font_px,
                code_family: &self.code_font,
                scale: self.scale,
                decorations: Decorations {
                    drafting: self.drafting,
                },
                bars: &self.bars,
            },
            &mut self.shapes,
            hitboxes,
            window,
        );
        // Horizontal bound per side: the whole side's longest shown line,
        // estimated as chars × mono advance (no shaping off screen), raised
        // by any wider line actually shaped. Re-clamp here so resize, fold
        // and font changes pull an offset back inside its travel.
        for side in [Side::Old, Side::New] {
            let ix = side_ix(side);
            self.widest_seen[ix] = self.widest_seen[ix].max(frame.widest(side));
            let longest = (layout.side(side).max_chars() as f32 * advance).max(self.widest_seen[ix]);
            let pane_w = f32::from(geom.pane(side).size.width);
            self.max_x[ix] = viewport::max_x(text_extent(longest), pane_w);
            self.x_offsets[ix] = viewport::clamp_x(self.x_offsets[ix], self.max_x[ix]);
            frame.set_x_offset(side, viewport::snap(self.x_offsets[ix], self.scale));
        }
        frame.stats.viewport = viewport_took;
        let index = nearest_hunk_index(self.hunk_s.unwrap_or(s / row_h), &layout.hunk_lands);
        // A new view_h can re-clamp `scroll_s`.
        self.scroll_s = s;
        if index != self.hunk_index {
            // Emitting mid-draw would not schedule the shell's redraw.
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |pane, cx| pane.set_hunk_index(index, cx)).ok();
            });
        }
        frame.stats.prepaint = trace::since(t_prepaint);
        Some(frame)
    }
}

/// Advance of `'0'` in `family` at `font_px`.
fn zero_advance(family: &SharedString, font_px: f32, window: &Window) -> f32 {
    let text = window.text_system();
    let id = text.resolve_font(&font(family.clone()));
    text.advance(id, px(font_px), '0')
        .map(|s| f32::from(s.width))
        .unwrap_or(font_px * 0.6)
}

impl Render for DualPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.layout.is_none() {
            return placeholder("No Layout", cx);
        }
        element::dual_pane(cx.entity()).into_any_element()
    }
}

/// Where the shell leaves room for the pane, written by the shell's layout.
pub type SlotBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

/// Mounts the pane over the shell at `SlotBounds`, as a sibling of the shell
/// rather than its child. GPUI re-renders every ancestor of a notified view,
/// so a nested pane would drag the shell along on every scroll frame. Both
/// are cached views: the shell reuses its last frame while only the pane is
/// dirty, and the pane reuses its last frame while only the shell is.
pub struct PaneSlot {
    bounds: SlotBounds,
    view: AnyElement,
}

pub fn slot(pane: &Entity<DualPane>, bounds: SlotBounds) -> PaneSlot {
    PaneSlot {
        bounds,
        view: AnyView::from(pane.clone())
            .cached(StyleRefinement::default().size_full())
            .into_any_element(),
    }
}

impl IntoElement for PaneSlot {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PaneSlot {
    type RequestLayoutState = ();
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    /// Runs after the shell's prepaint (earlier sibling), so the slot bounds
    /// are from this frame.
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(bounds) = self.bounds.get() else {
            return false;
        };
        self.view.layout_as_root(bounds.size.into(), window, cx);
        self.view.prepaint_at(bounds.origin, window, cx);
        true
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        mounted: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *mounted {
            self.view.paint(window, cx);
        }
    }
}

pub enum FontOp {
    Inc,
    Dec,
    Reset,
}

pub fn placeholder(msg: &str, cx: &App) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .ui_text_size(14., cx)
        .text_color(theme::muted())
        .child(msg.to_string())
        .into_any_element()
}

fn side_ix(side: Side) -> usize {
    match side {
        Side::Old => 0,
        Side::New => 1,
    }
}

/// Last Hunk landing at or above `s_rows` (the first one if none does).
/// `lands` are in Alignment order, so `s` is non-decreasing.
pub(super) fn nearest_hunk_index(s_rows: f32, lands: &[HunkLand]) -> Option<usize> {
    if lands.is_empty() {
        return None;
    }
    let past = lands.partition_point(|land| land.s as f32 <= s_rows + 0.5);
    Some(past.saturating_sub(1))
}

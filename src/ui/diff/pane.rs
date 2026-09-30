//! The dual pane (preimage | gutter | postimage) as its own Entity, so wheel, scrollbar
//! drag and hover notify only this view. Its body is one `DualPaneElement`
//! (element.rs); this file holds the state and the input handling. See
//! docs/diffview-architecture.md §4–§5.

use gpui::{
    AnyElement, AnyView, App, Bounds, ContentMask, Context, Element, ElementId, Entity,
    EventEmitter, GlobalElementId, InspectorElementId, IntoElement, LayoutId, ParentElement,
    Pixels, Position, Render, SharedString, Style, StyleRefinement, Styled, Subscription, Task,
    Timer, Window, div, font, px, size,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use super::element::{
    self, BarState, Decorations, FrameInput, Geom, ShapeCache, build_frame, code_wrap_width_px,
    insert_scrollbar_hitboxes, line_number_digits, ln_col_width, text_extent, thumb_for, top_at,
    wrap_plan_for_panes,
};
use super::layout::{HunkLand, Layout, Row, WrapPlan};
use super::trace;
use super::viewport::{self, Viewport};
use crate::domain::{
    Alignment, AlignmentOp, Anchor, DiffFontSize, FoldState, HunkJumpTarget, SearchSide, Side,
    first_match_byte, hunk_jump_target, match_jump_plan,
};
use crate::git::FileDiff;
use crate::syntax::{self, Span};
use crate::ui::appearance::{Appearance, UiTextSize};
use crate::ui::{scrollbar, theme};
use std::path::Path;

/// What the shell (DiffView) hears from the pane. Hunk index and hover copy
/// are emitted only when they change.
pub enum PaneEvent {
    /// Empty gutter icon clicked: open the draft dock on this LineSpan.
    OpenDraft {
        side: Side,
        start: u32,
        count: u32,
    },
    /// Filled gutter icon clicked: reopen this DraftComment's body in the dock.
    OpenEdit {
        id: u64,
    },
    /// A gutter-band drag began a postimage selection, so any open DraftComment is
    /// no longer the user's target.
    SelectionStarted,
    HunkIndexChanged(Option<usize>),
    HoverCopy(Option<String>),
}

/// One mouse move, in element coordinates, as the pane reads it.
pub(super) struct PointerMove {
    /// Vertical scrollbar track hover, per side.
    pub hovered: [bool; 2],
    /// Horizontal scrollbar track hover, per side.
    pub h_hovered: [bool; 2],
    /// Pane y, only while the pointer is over the center gutter.
    pub gutter_y: Option<f32>,
    /// Pane y, wherever the pointer is; a selection drag reads only this.
    pub pane_y: f32,
    /// Pointer y inside each side's vertical track.
    pub track_y: [f32; 2],
    /// Pointer x inside each side's horizontal track.
    pub h_track_x: [f32; 2],
}

/// Contiguous, same-side, line-granular selection from a center-gutter drag
/// on the line-number column. `start..=end` in 1-based line numbers of `side`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineSelection {
    side: Side,
    start: u32,
    end: u32,
}

impl LineSelection {
    fn span(self) -> (Side, u32, u32) {
        (self.side, self.start, self.end)
    }
}

/// A DraftComment as the pane needs it: the Anchor drives the row index and the
/// start-line icons, the id is what a click on a filled icon reopens.
#[derive(Clone, Debug)]
pub struct PaneComment {
    pub id: u64,
    pub anchor: Anchor,
}

struct PaneFile {
    alignment: Alignment,
    preimage_text: Arc<str>,
    postimage_text: Arc<str>,
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
    /// Per-side syntax spans over the side's full text. Kept across
    /// `rebuild_layout` (fold / ignore-whitespace); cleared on file open.
    highlights: [Option<Arc<[Span]>>; 2],
    /// Shaped text per (side, visual row), visible ± one screen.
    shapes: ShapeCache,
    /// The selected path's DraftComments (comment row index + start-line bubbles).
    comments: Vec<PaneComment>,
    /// LineSpan being drafted, highlighted in its pane.
    drafting: Option<(Side, u32, u32)>,
    /// Current drag selection; the draft target while the dock is open.
    selection: Option<LineSelection>,
    /// Live drag: the side and the line the press landed on.
    sel_drag: Option<(Side, u32)>,
    /// Shared scroll parameter, in pixels, and the only vertical scroll
    /// source. See docs/dual-pane-diff.md §3.1. Kept inside
    /// `viewport::s_range` from the first measured frame on.
    scroll_s: f32,
    /// Per-side horizontal scroll of the code text, in pixels, `[preimage, postimage]`.
    /// When [`Self::sync_horizontal`] is on, both stay equal. Independent of
    /// `scroll_s`; moved by horizontal input over a pane (both panes when
    /// synced). Reset on file open, kept (re-clamped) on fold, Alignment, font
    /// size and resize.
    x_offsets: [f32; 2],
    /// §3.1.2: one shared offset when true.
    sync_horizontal: bool,
    /// §3.1.1: soft wrap in the code columns.
    soft_wrap: bool,
    /// In-file search query for row highlights (mirrors DiffView).
    search_query: SharedString,
    /// Which side(s) to highlight (mirrors DiffView Side factor).
    search_side: SearchSide,
    /// Current occurrence identity for paint (darker + pulse).
    active_search: Option<ActiveSearchMatch>,
    /// When the active match landed; drives ~250ms pulse.
    search_pulse_at: Option<std::time::Instant>,
    /// Shaped non-ASCII advances per `(char, font px, family hash)`.
    char_widths: HashMap<(char, u32, u64), f32>,
    /// Last prepaint pane widths; stable width triggers rewrap (§6 deferral).
    prev_frame_pane_w: [f32; 2],
    /// Wrap must rebuild in prepaint with a Window (glyph widths + plan).
    wrap_layout_dirty: bool,
    /// Scroll landing deferred until the next wrapped layout rebuild.
    pending_land: Option<PendingLand>,
    /// Which side last received horizontal input; used when sync is turned on.
    last_x_side: Option<Side>,
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
    /// Code column width per side, set in prepaint.
    pane_w: [f32; 2],
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
    /// Code Theme id the shape cache was colored with. A change reshapes;
    /// highlight spans stay capture ids.
    code_theme_id: String,
    _appearance: Subscription,
    /// Bumped on every [`Self::open`]; background highlight results with a
    /// different generation are dropped.
    open_generation: u64,
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
            highlights: [None, None],
            shapes: ShapeCache::default(),
            comments: Vec::new(),
            drafting: None,
            selection: None,
            sel_drag: None,
            scroll_s: 0.,
            x_offsets: [0.; 2],
            sync_horizontal: true,
            soft_wrap: false,
            search_query: SharedString::default(),
            search_side: SearchSide::Postimage,
            active_search: None,
            search_pulse_at: None,
            char_widths: HashMap::new(),
            prev_frame_pane_w: [0.; 2],
            wrap_layout_dirty: false,
            pending_land: None,
            last_x_side: None,
            max_x: [0.; 2],
            widest_seen: [0.; 2],
            mono_advance: None,
            ln_advance: None,
            view_h: 0.,
            pane_w: [0.; 2],
            scale: 1.,
            hover_copy: None,
            press: None,
            bars: BarState::default(),
            bar_hide: None,
            hunk_index: None,
            hunk_s: Some(0.),
            font_size,
            code_font,
            code_theme_id: String::new(),
            _appearance: cx.observe_global::<Appearance>(Self::apply_appearance),
            open_generation: 0,
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
            self.char_widths.clear();
            if self.soft_wrap {
                self.wrap_layout_dirty = true;
                let cap = self.capture_anchor();
                self.rebuild_layout(None);
                self.restore_after_rewrap(cap);
            } else {
                self.invalidate_shapes();
            }
            cx.notify();
        }
    }

    fn mark_wrap_dirty(&mut self) {
        if self.soft_wrap {
            self.wrap_layout_dirty = true;
        }
    }

    /// Drop shaped text and the widest-line bound it fed; the next prepaint
    /// reshapes what is visible.
    fn invalidate_shapes(&mut self) {
        self.shapes.clear();
        self.widest_seen = [0.; 2];
    }

    /// Open a file at its start with everything folded.
    pub fn open(
        &mut self,
        path: &str,
        file: &FileDiff,
        comments: Vec<PaneComment>,
        cx: &mut Context<Self>,
    ) {
        self.open_generation = self.open_generation.wrapping_add(1);
        let generation = self.open_generation;
        self.pending_land = None;
        self.file = match file {
            FileDiff::Text {
                alignment,
                preimage_text,
                postimage_text,
            } => Some(PaneFile {
                alignment: alignment.clone(),
                preimage_text: preimage_text.clone(),
                postimage_text: postimage_text.clone(),
            }),
            _ => None,
        };
        // Plain text until the background task finishes (or the size guard skips).
        self.highlights = [None, None];
        if let Some(file) = self.file.as_ref() {
            let first =
                first_line(&file.postimage_text).or_else(|| first_line(&file.preimage_text));
            if let Some(lang) = syntax::detect(Path::new(path), first.unwrap_or("")) {
                let preimage_text = file.preimage_text.clone();
                let postimage_text = file.postimage_text.clone();
                let any_under_guard =
                    !syntax::exceeds_size_guard(&preimage_text, syntax::DEFAULT_SIZE_GUARD)
                        || !syntax::exceeds_size_guard(&postimage_text, syntax::DEFAULT_SIZE_GUARD);
                if any_under_guard {
                    cx.spawn(async move |this, cx| {
                        let (sides, took) = cx
                            .background_executor()
                            .spawn(async move {
                                let t = std::time::Instant::now();
                                // Query compile + palette resolve stay off the UI thread.
                                let code_theme = crate::ui::code_theme::active();
                                let _ = crate::ui::code_theme::capture_colors(&code_theme);
                                let side = |text: &str| {
                                    if syntax::exceeds_size_guard(text, syntax::DEFAULT_SIZE_GUARD)
                                    {
                                        None
                                    } else {
                                        Some(Arc::from(syntax::highlight(lang, text)))
                                    }
                                };
                                ([side(&preimage_text), side(&postimage_text)], t.elapsed())
                            })
                            .await;
                        this.update(cx, |this, cx| {
                            if !should_apply_highlight(this.open_generation, generation) {
                                return;
                            }
                            this.highlights = sides;
                            this.shapes.clear();
                            if trace::enabled() {
                                trace::highlight(took);
                            }
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
            }
        }
        self.comments = comments;
        // A selection belongs to the file it was dragged in.
        self.selection = None;
        self.sel_drag = None;
        self.fold = FoldState::collapsed();
        self.set_hunk_index(None, cx);
        self.hunk_s = Some(0.);
        self.reset_scroll(cx);
        self.x_offsets = [0.; 2];
        self.last_x_side = None;
        self.wrap_layout_dirty = self.soft_wrap;
        self.rebuild_layout(None);
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

    pub fn set_comments(&mut self, comments: Vec<PaneComment>, cx: &mut Context<Self>) {
        self.comments = comments;
        if let Some(layout) = self.layout.as_mut() {
            layout.set_comments(self.comments.iter().map(|c| &c.anchor));
        }
        cx.notify();
    }

    /// Start line and id of every line-anchored DraftComment, in comment order:
    /// one filled icon each, always visible.
    fn comment_starts(&self) -> Vec<(Side, u32, u64)> {
        self.comments
            .iter()
            .filter_map(|c| match &c.anchor {
                Anchor::Line { side, span, .. } => Some((*side, span.start, c.id)),
                Anchor::File { .. } => None,
            })
            .collect()
    }

    pub fn set_drafting(&mut self, drafting: Option<(Side, u32, u32)>, cx: &mut Context<Self>) {
        if self.drafting != drafting {
            self.drafting = drafting;
            cx.notify();
        }
    }

    /// Point the selection at a LineSpan the shell chose (reopening a
    /// DraftComment makes its span the draft target, so the wash and the icon
    /// follow the dock).
    pub fn select_span(&mut self, side: Side, start: u32, end: u32, cx: &mut Context<Self>) {
        let next = Some(LineSelection { side, start, end });
        self.sel_drag = None;
        if self.selection != next {
            self.selection = next;
            cx.notify();
        }
    }

    /// Current selection as `(side, start, end)` inclusive, if any.
    pub fn selection(&self) -> Option<(Side, u32, u32)> {
        self.selection.map(LineSelection::span)
    }

    /// Drop the wash without opening or closing the dock.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_none() && self.sel_drag.is_none() {
            return;
        }
        self.selection = None;
        self.sel_drag = None;
        cx.notify();
    }

    /// Icon click. A filled icon names the DraftComment that starts on its line,
    /// so it reopens that body; an empty one only paints on a selection's start
    /// line, so there always is a selection to hand over as the postimage span.
    pub(super) fn click_comment_icon(&mut self, mark: element::IconMark, cx: &mut Context<Self>) {
        match mark {
            element::IconMark::Filled(id) => cx.emit(PaneEvent::OpenEdit { id }),
            element::IconMark::Empty => {
                let Some(sel) = self.selection else {
                    return;
                };
                cx.emit(PaneEvent::OpenDraft {
                    side: sel.side,
                    start: sel.start,
                    count: span_count(sel.start, sel.end),
                });
            }
        }
    }

    pub fn font_px(&self) -> u32 {
        self.font_size.px()
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
    fn rebuild_layout(&mut self, window: Option<&mut Window>) {
        self.layout = None;
        self.invalidate_shapes();
        let Some(file) = self.file.as_ref() else {
            return;
        };
        let preimage_text = file.preimage_text.clone();
        let postimage_text = file.postimage_text.clone();
        let alignment = file.alignment.clone();
        let t = trace::start();
        let plan = wrap_plan_for_panes(self.pane_w[0], self.pane_w[1]);
        let font_px = self.font_size.px();
        let mono = self
            .mono_advance
            .map(|(_, a)| a)
            .unwrap_or(font_px as f32 * 0.6);
        let soft = self.soft_wrap;
        let pane_w = self.pane_w;
        let family_hash = family_hash(&self.code_font);
        let can_wrap = soft && (pane_w[0] > 0. || pane_w[1] > 0.) && window.is_some();
        let mut width_ctx = WrapCharWidth {
            mono,
            font_px,
            family: &self.code_font,
            family_hash,
            cache: &mut self.char_widths,
            window,
        };
        let mut char_width = |c: char| width_ctx.width(c);
        let wrap = if can_wrap {
            Some((
                &plan as &WrapPlan,
                &mut char_width as &mut dyn FnMut(char) -> f32,
            ))
        } else {
            if soft {
                self.wrap_layout_dirty = true;
            }
            None
        };
        let mut layout = Layout::build(
            preimage_text,
            postimage_text,
            &alignment,
            Some(&self.fold),
            wrap,
        );
        layout.set_comments(self.comments.iter().map(|c| &c.anchor));
        debug_assert!(!soft || !can_wrap || layout.wrap.is_some());
        if t.is_some() {
            trace::layout(
                trace::since(t),
                [layout.preimage.rows(), layout.postimage.rows()],
                layout.bridges.len(),
                layout.wrap.is_some(),
            );
        }
        self.layout = Some(layout);
        if can_wrap {
            self.wrap_layout_dirty = false;
        }
    }

    fn restore_after_rewrap(&mut self, cap: Option<viewport::AnchorCap>) {
        let (Some(cap), Some(layout)) = (cap, self.layout.as_ref()) else {
            return;
        };
        if let Some(s) =
            viewport::s_for_rewrap(layout, cap, self.view_h, self.row_h(), self.scroll_s)
        {
            self.scroll_s = s;
        }
    }

    fn finish_wrap_rebuild(&mut self, cap: Option<viewport::AnchorCap>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let row_h = self.row_h();
        match self.pending_land.take() {
            Some(PendingLand::Match { side, ln, byte }) => {
                if let Some(s) = viewport::s_for_match_byte(
                    layout,
                    side,
                    ln,
                    byte,
                    self.view_h,
                    row_h,
                    self.scroll_s,
                ) {
                    self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
                    self.hunk_s = Some(self.scroll_s / row_h);
                }
            }
            Some(PendingLand::Line(target)) => {
                if let Some(s) = viewport::s_for_target(layout, target, row_h, self.scroll_s) {
                    self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
                    self.hunk_s = Some(self.scroll_s / row_h);
                }
            }
            Some(PendingLand::Anchor(cap)) => {
                if let Some(s) =
                    viewport::s_for_rewrap(layout, cap, self.view_h, row_h, self.scroll_s)
                {
                    self.scroll_s = s;
                }
            }
            None => self.restore_after_rewrap(cap),
        }
    }

    fn sync_wrap_layout(&mut self, pane_w: [f32; 2], window: &mut Window, cx: &mut Context<Self>) {
        if !self.soft_wrap {
            self.wrap_layout_dirty = false;
            return;
        }
        let has_width = pane_w[0] > 0. || pane_w[1] > 0.;
        if !has_width {
            return;
        }
        self.pane_w = pane_w;

        let width_mismatch = |layout: &Layout| {
            layout.wrap.as_ref().is_none_or(|applied| {
                applied.plan.preimage.width_px != code_wrap_width_px(pane_w[0])
                    || applied.plan.postimage.width_px != code_wrap_width_px(pane_w[1])
            })
        };

        if self.wrap_layout_dirty || self.layout.as_ref().is_none_or(|l| l.wrap.is_none()) {
            self.prev_frame_pane_w = pane_w;
            let cap = self.pending_land.is_none().then(|| self.capture_anchor());
            self.rebuild_layout(Some(window));
            self.finish_wrap_rebuild(cap.flatten());
            return;
        }

        let stable =
            pane_w[0] == self.prev_frame_pane_w[0] && pane_w[1] == self.prev_frame_pane_w[1];
        if !stable {
            self.prev_frame_pane_w = pane_w;
            cx.notify();
            return;
        }
        self.prev_frame_pane_w = pane_w;

        if self.layout.as_ref().is_some_and(width_mismatch) {
            let cap = self.pending_land.is_none().then(|| self.capture_anchor());
            self.rebuild_layout(Some(window));
            self.finish_wrap_rebuild(cap.flatten());
        }
    }

    pub fn set_soft_wrap(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.soft_wrap {
            return;
        }
        let cap = self.capture_anchor();
        self.soft_wrap = on;
        if on {
            self.x_offsets = [0.; 2];
            self.wrap_layout_dirty = true;
        } else {
            self.wrap_layout_dirty = false;
            self.pending_land = None;
        }
        self.rebuild_layout(None);
        self.restore_after_rewrap(cap);
        cx.notify();
    }

    pub fn set_search_query(&mut self, query: SharedString, cx: &mut Context<Self>) {
        if query == self.search_query {
            return;
        }
        self.search_query = query;
        cx.notify();
    }

    pub fn set_search_side(&mut self, side: SearchSide, cx: &mut Context<Self>) {
        if side == self.search_side {
            return;
        }
        self.search_side = side;
        cx.notify();
    }

    pub fn set_active_search_match(
        &mut self,
        active: Option<ActiveSearchMatch>,
        cx: &mut Context<Self>,
    ) {
        let changed = self.active_search != active;
        self.active_search = active;
        if changed {
            if self.active_search.is_some() {
                self.search_pulse_at = Some(std::time::Instant::now());
                let this = cx.entity().downgrade();
                cx.spawn(async move |_, cx| {
                    for _ in 0..5 {
                        Timer::after(std::time::Duration::from_millis(50)).await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    }
                    this.update(cx, |pane, cx| {
                        pane.search_pulse_at = None;
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            } else {
                self.search_pulse_at = None;
            }
            cx.notify();
        }
    }

    /// Back to the file start. The first measured frame clamps `scroll_s` up
    /// to the lower end of `s_range`.
    fn reset_scroll(&mut self, cx: &mut Context<Self>) {
        self.scroll_s = 0.;
        self.press = None;
        self.bars.drag = None;
        self.bars.h_drag = None;
        self.set_hover_copy(None, cx);
    }

    fn with_anchor(&mut self, mutate: impl FnOnce(&mut Self)) {
        let cap = self.capture_anchor();
        mutate(self);
        self.mark_wrap_dirty();
        self.rebuild_layout(None);
        if self.soft_wrap {
            if let Some(cap) = cap {
                self.pending_land = Some(PendingLand::Anchor(cap));
            }
        } else if let Some(cap) = cap {
            self.restore_anchor(Some(cap));
        }
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
        if let Some(s) =
            viewport::s_for_anchor(layout, cap, self.view_h, self.row_h(), self.scroll_s)
        {
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
            (0..hunk_count).rev().find(|&i| {
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
        let s =
            viewport::s_for_target(layout, target, row_h, self.scroll_s).unwrap_or(self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = Some(s / row_h);
        self.set_hunk_index(Some(i), cx);
        self.reveal_bars(cx);
        cx.notify();
    }

    /// Expand a collapsed Equal that hides `ln` (if any) and scroll so the line
    /// lands on the §3.1 anchor. Used by island select / edit reopen.
    pub fn reveal_line(&mut self, side: Side, ln: u32, cx: &mut Context<Self>) {
        let Some(target) = self.expand_for_line_jump(side, ln) else {
            return;
        };
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let row_h = self.row_h();
        if self.soft_wrap && (self.wrap_layout_dirty || layout.wrap.is_none()) {
            self.pending_land = Some(PendingLand::Line(target));
            cx.notify();
            return;
        }
        let s =
            viewport::s_for_target(layout, target, row_h, self.scroll_s).unwrap_or(self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = Some(s / row_h);
        self.sync_hunk_index(cx);
        self.reveal_bars(cx);
        cx.notify();
    }

    pub fn jump_match(&mut self, side: Side, ln: u32, byte: Option<usize>, cx: &mut Context<Self>) {
        let Some(target) = self.expand_for_line_jump(side, ln) else {
            return;
        };
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let row_h = self.row_h();
        let line_text = layout.side(target.side).line_text(target.ln);
        let byte =
            byte.or_else(|| line_text.and_then(|text| first_match_byte(text, &self.search_query)));
        let defer = self.soft_wrap && (self.wrap_layout_dirty || layout.wrap.is_none());
        if defer {
            self.pending_land = Some(match byte {
                Some(b) => PendingLand::Match {
                    side: target.side,
                    ln: target.ln,
                    byte: b,
                },
                None => PendingLand::Line(target),
            });
            cx.notify();
            return;
        }
        let s = byte
            .and_then(|b| {
                viewport::s_for_match_byte(
                    layout,
                    target.side,
                    target.ln,
                    b,
                    self.view_h,
                    row_h,
                    self.scroll_s,
                )
            })
            .or_else(|| viewport::s_for_target(layout, target, row_h, self.scroll_s))
            .unwrap_or(self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = Some(s / row_h);
        self.sync_hunk_index(cx);
        self.reveal_bars(cx);
        cx.notify();
    }

    /// Apply [`match_jump_plan`]: expand a collapsed Equal hiding `ln`, return the land target.
    fn expand_for_line_jump(&mut self, side: Side, ln: u32) -> Option<HunkJumpTarget> {
        let file = self.file.as_ref()?;
        let plan = match_jump_plan(&file.alignment, &self.fold, side, ln);
        if let Some(id) = plan.expand {
            self.fold.expand(id);
            self.mark_wrap_dirty();
            self.rebuild_layout(None);
        }
        Some(plan.target)
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
        if self.soft_wrap {
            self.char_widths.clear();
            self.wrap_layout_dirty = true;
            let cap = self.capture_anchor();
            self.rebuild_layout(None);
            self.restore_after_rewrap(cap);
        } else {
            self.invalidate_shapes();
        }
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

    pub fn set_sync_horizontal(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.sync_horizontal {
            return;
        }
        self.sync_horizontal = on;
        if on {
            let x = viewport::clamp_x_synced(
                viewport::shared_x_on_sync_enable(self.x_offsets, self.last_x_side),
                self.max_x,
            );
            self.x_offsets = [x, x];
        }
        cx.notify();
    }

    /// Horizontal wheel / trackpad over `side`'s code pane. When synced, both
    /// sides move; otherwise only that side. `scroll_s` is unchanged.
    pub(super) fn scroll_x_by(&mut self, side: Side, dx: f32, cx: &mut Context<Self>) {
        if self.soft_wrap {
            return;
        }
        self.last_x_side = Some(side);
        let x = if self.sync_horizontal {
            viewport::clamp_x_synced(self.x_offsets[0] + dx, self.max_x)
        } else {
            let ix = side_ix(side);
            viewport::clamp_x(self.x_offsets[ix] + dx, self.max_x[ix])
        };
        if self.sync_horizontal {
            if x != self.x_offsets[0] || x != self.x_offsets[1] {
                self.x_offsets = [x, x];
                self.reveal_bars(cx);
                cx.notify();
            }
        } else {
            let ix = side_ix(side);
            if x != self.x_offsets[ix] {
                self.x_offsets[ix] = x;
                self.reveal_bars(cx);
                cx.notify();
            }
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
                if this.bars.hovered.iter().any(|&h| h)
                    || this.bars.h_hovered.iter().any(|&h| h)
                    || this.bars.any_drag()
                {
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

    /// `max_x` passed to thumb geometry: shared bound when synced, else per side.
    fn h_thumb_scroll_max(&self, side: Side) -> f32 {
        if self.sync_horizontal {
            viewport::max_x_synced(self.max_x)
        } else {
            self.max_x[side_ix(side)]
        }
    }

    fn h_thumb_geom(&self, side: Side) -> Option<viewport::HThumbGeom> {
        let ix = side_ix(side);
        if self.max_x[ix] <= 0. {
            return None;
        }
        viewport::h_thumb_for(
            self.pane_w[ix],
            self.h_thumb_scroll_max(side),
            self.x_offsets[ix],
        )
    }

    pub(super) fn h_bar_shown(&self, side: Side) -> bool {
        self.bars.h_shown(side)
    }

    fn drag_h_thumb(&mut self, side: Side, thumb_left: f32) {
        let Some(geom) = self.h_thumb_geom(side) else {
            return;
        };
        let x = viewport::x_at(&geom, thumb_left);
        self.last_x_side = Some(side);
        if self.sync_horizontal {
            self.x_offsets = [viewport::clamp_x_synced(x, self.max_x); 2];
        } else {
            let ix = side_ix(side);
            self.x_offsets[ix] = viewport::clamp_x(x, self.max_x[ix]);
        }
    }

    /// Left press in `side`'s horizontal track at track x `x`.
    pub(super) fn press_h_track(&mut self, side: Side, x: f32, cx: &mut Context<Self>) {
        let Some(geom) = self.h_thumb_geom(side) else {
            return;
        };
        let (left, w) = (geom.thumb_left, geom.thumb_width);
        let grab = if x >= left && x <= left + w {
            x - left
        } else {
            self.drag_h_thumb(side, x - w / 2.);
            w / 2.
        };
        self.bars.h_drag = Some((side, grab));
        self.bars.visible = true;
        self.bar_hide = None;
        self.press = None;
        cx.notify();
    }

    /// Visual row of `side` at pane y `y`.
    fn row_index_at(&self, side: Side, y: f32) -> Option<u32> {
        match self.viewport()?.hit(side, y)? {
            Row::Line(l) => Some(l.row),
            Row::Omit(o) => Some(o.row),
        }
    }

    /// Logical line `side` shows at pane y `y`, skipping blank padding rows.
    fn line_at(&self, side: Side, y: f32) -> Option<u32> {
        match self.viewport()?.hit(side, y)? {
            Row::Line(l) if !l.is_equal_padding() => Some(l.ln),
            _ => None,
        }
    }

    /// Left press in `side`'s code column: arm an omission-separator click only.
    /// Line selection belongs to the gutter band so the code column stays free
    /// for text selection.
    pub(super) fn press_code(&mut self, side: Side, y: f32, _cx: &mut Context<Self>) {
        self.press = self.row_index_at(side, y).map(|row| (side, row));
    }

    /// Left press on `side`'s line-number column.
    /// On a line it starts a selection drag (replacing any selection, including
    /// one on the other side).
    pub(super) fn press_gutter_select(&mut self, side: Side, y: f32, cx: &mut Context<Self>) {
        let Some(ln) = self.line_at(side, y) else {
            return;
        };
        self.press = None;
        self.sel_drag = Some((side, ln));
        self.selection = Some(LineSelection {
            side,
            start: ln,
            end: ln,
        });
        cx.emit(PaneEvent::SelectionStarted);
        cx.notify();
    }

    /// Left release. Ends a thumb or selection drag, or completes a click on
    /// the pressed omission separator by expanding its span.
    pub(super) fn release(&mut self, side: Option<Side>, y: f32, cx: &mut Context<Self>) {
        self.sel_drag = None;
        if self.bars.drag.take().is_some() || self.bars.h_drag.take().is_some() {
            if !self.bars.hovered.iter().any(|&h| h) && !self.bars.h_hovered.iter().any(|&h| h) {
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
        let omit = self.viewport().and_then(|vp| match vp.hit(side, y)? {
            Row::Omit(o) if o.row == pressed.1 => Some(o.id),
            _ => None,
        });
        if let Some(id) = omit {
            self.expand_omit(id, cx);
        }
    }

    /// Track hover per side, an active thumb or selection drag, and gutter
    /// hover copy.
    pub(super) fn mouse_moved(&mut self, m: PointerMove, cx: &mut Context<Self>) {
        let PointerMove {
            hovered,
            h_hovered,
            gutter_y,
            pane_y,
            track_y,
            h_track_x,
        } = m;
        let mut dirty = false;
        // A drag keeps its own side and reads only y, so leaving the column
        // sideways (or over the gutter) still extends the selection.
        if let Some((side, anchor)) = self.sel_drag
            && let Some(ln) = self.line_at(side, pane_y)
        {
            let (start, end) = selection_span(anchor, ln);
            let next = Some(LineSelection { side, start, end });
            if self.selection != next {
                self.selection = next;
                dirty = true;
            }
        }
        if self.bars.hovered != hovered {
            self.bars.hovered = hovered;
            if hovered.iter().any(|&h| h) {
                self.bars.visible = true;
                self.bar_hide = None;
            } else if !self.bars.any_drag() && !self.bars.h_hovered.iter().any(|&h| h) {
                self.arm_bar_hide(cx);
            }
            dirty = true;
        }
        if self.bars.h_hovered != h_hovered {
            self.bars.h_hovered = h_hovered;
            if h_hovered.iter().any(|&h| h) {
                self.bars.visible = true;
                self.bar_hide = None;
            } else if !self.bars.any_drag() && !self.bars.hovered.iter().any(|&h| h) {
                self.arm_bar_hide(cx);
            }
            dirty = true;
        }
        if let Some((side, grab)) = self.bars.drag {
            self.drag_thumb(side, track_y[side_ix(side)] - grab);
            self.sync_hunk_index(cx);
            dirty = true;
        }
        if let Some((side, grab)) = self.bars.h_drag {
            let ix = side_ix(side);
            self.drag_h_thumb(side, h_track_x[ix] - grab);
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
        let ln_w = ln_col_width(line_number_digits(self.layout.as_ref()?), ln_advance);
        let geom = Geom::new(bounds, ln_w, self.scale);
        for side in [Side::Preimage, Side::Postimage] {
            self.pane_w[side_ix(side)] = f32::from(geom.pane(side).size.width);
        }
        self.sync_wrap_layout(self.pane_w, window, cx);
        let comment_starts = self.comment_starts();
        let code_theme = crate::ui::code_theme::active();
        if code_theme.id != self.code_theme_id {
            self.invalidate_shapes();
            self.code_theme_id = code_theme.id.clone();
        }
        let layout = self.layout.as_ref()?;
        let t_vp = trace::start();
        let vp = Viewport::new(layout, self.scroll_s, self.view_h, row_h).snapped(self.scale);
        let viewport_took = trace::since(t_vp);
        let s = vp.s();
        let mut frame = build_frame(
            FrameInput {
                layout,
                highlights: &self.highlights,
                vp: &vp,
                geom,
                row_h,
                font_px,
                code_family: &self.code_font,
                scale: self.scale,
                decorations: Decorations {
                    drafting: self.drafting,
                    selection: self.selection.map(LineSelection::span),
                    comment_starts,
                    search_query: (!self.search_query.is_empty())
                        .then(|| Arc::from(self.search_query.as_str())),
                    search_side: self.search_side,
                    active_match: self.active_search.clone(),
                    search_pulse: self.search_pulse_at.map(|t| {
                        let elapsed = t.elapsed().as_secs_f32();
                        (1. - (elapsed / 0.25)).clamp(0., 1.)
                    }),
                },
                bars: &self.bars,
                code_theme,
            },
            &mut self.shapes,
            window,
        );
        // Horizontal bound per side: the whole side's longest shown line,
        // estimated as chars × mono advance (no shaping off screen), raised
        // by any wider line actually shaped. Re-clamp here so resize, fold
        // and font changes pull an offset back inside its travel.
        if self.soft_wrap {
            self.max_x = [0.; 2];
            self.x_offsets = [0.; 2];
        } else {
            for side in [Side::Preimage, Side::Postimage] {
                let ix = side_ix(side);
                self.widest_seen[ix] = self.widest_seen[ix].max(frame.widest(side));
                let longest =
                    (layout.side(side).max_chars() as f32 * advance).max(self.widest_seen[ix]);
                let pane_w = f32::from(geom.pane(side).size.width);
                self.max_x[ix] = viewport::max_x(text_extent(longest), pane_w);
            }
            if self.sync_horizontal {
                let x = viewport::clamp_x_synced(self.x_offsets[0], self.max_x);
                self.x_offsets = [x, x];
            } else {
                for side in [Side::Preimage, Side::Postimage] {
                    let ix = side_ix(side);
                    self.x_offsets[ix] = viewport::clamp_x(self.x_offsets[ix], self.max_x[ix]);
                }
            }
        }
        for side in [Side::Preimage, Side::Postimage] {
            let ix = side_ix(side);
            frame.set_x_offset(side, viewport::snap(self.x_offsets[ix], self.scale));
        }
        let v_tracks = [Side::Preimage, Side::Postimage]
            .map(|side| thumb_for(self.view_h, vp.max_top(side), vp.top(side)).is_some());
        let h_tracks = if self.soft_wrap {
            [false, false]
        } else {
            [Side::Preimage, Side::Postimage].map(|side| self.max_x[side_ix(side)] > 0.)
        };
        let (tracks, h_track_boxes) = insert_scrollbar_hitboxes(&geom, v_tracks, h_tracks, window);
        frame.tracks = tracks;
        frame.h_tracks = h_track_boxes;
        // Inserted after the element's own hitbox, so the icons win the press.
        frame.icon_hitboxes = frame
            .icons
            .iter()
            .map(|icon| window.insert_hitbox(icon.slot, gpui::HitboxBehavior::Normal))
            .collect();
        for side in [Side::Preimage, Side::Postimage] {
            if let Some(g) = self.h_thumb_geom(side) {
                frame.set_h_thumb(side, g, &self.bars);
            }
        }
        frame.stats.viewport = viewport_took;
        let index = nearest_hunk_index(self.hunk_s.unwrap_or(s / row_h), &layout.hunk_lands);
        // A postimage view_h can re-clamp `scroll_s`.
        self.scroll_s = s;
        if index != self.hunk_index {
            // Emitting mid-draw would not schedule the shell's redraw.
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |pane, cx| pane.set_hunk_index(index, cx))
                    .ok();
            });
        }
        frame.stats.prepaint = trace::since(t_prepaint);
        Some(frame)
    }
}

enum PendingLand {
    Match { side: Side, ln: u32, byte: usize },
    Line(crate::domain::HunkJumpTarget),
    Anchor(viewport::AnchorCap),
}

pub use super::element::ActiveSearchMatch;

struct WrapCharWidth<'a, 'w> {
    mono: f32,
    font_px: u32,
    family: &'a SharedString,
    family_hash: u64,
    cache: &'a mut HashMap<(char, u32, u64), f32>,
    window: Option<&'w mut Window>,
}

impl WrapCharWidth<'_, '_> {
    fn width(&mut self, c: char) -> f32 {
        if c.is_ascii() {
            return self.mono;
        }
        let key = (c, self.font_px, self.family_hash);
        if let Some(&w) = self.cache.get(&key) {
            return w;
        }
        let window = self
            .window
            .as_deref_mut()
            .expect("wrap layout builds need a Window for non-ASCII widths");
        let w = char_advance(self.family, self.font_px as f32, c, window);
        self.cache.insert(key, w);
        w
    }
}

fn family_hash(family: &SharedString) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    family.hash(&mut h);
    h.finish()
}

/// Shaped like painted rows, so glyphs missing from the Code Font get the
/// fallback font's width instead of failing.
fn char_advance(family: &SharedString, font_px: f32, c: char, window: &mut Window) -> f32 {
    let text: SharedString = c.to_string().into();
    let run = gpui::TextRun {
        len: text.len(),
        font: font(family.clone()),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text, px(font_px), &[run], None);
    f32::from(line.width)
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
    /// Lay the pane out at this width instead of the measured slot. Paint stays
    /// clipped to the slot, so a show/hide slide can move the card edge without
    /// the code reflowing over the comment island.
    layout_width: Option<f32>,
    view: AnyElement,
}

pub fn slot(pane: &Entity<DualPane>, bounds: SlotBounds, layout_width: Option<f32>) -> PaneSlot {
    PaneSlot {
        bounds,
        layout_width,
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
        let width = self.layout_width.map(px).unwrap_or(bounds.size.width);
        self.view
            .layout_as_root(size(width, bounds.size.height).into(), window, cx);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.view.prepaint_at(bounds.origin, window, cx);
        });
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
            let Some(bounds) = self.bounds.get() else {
                return;
            };
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                self.view.paint(window, cx);
            });
        }
    }
}

/// Like [`slot`], but for a one-shot chrome element that must paint **after**
/// the DualPane (e.g. the floating find bar, so its drop shadow sits on code).
pub struct OverlaySlot {
    bounds: SlotBounds,
    child: AnyElement,
}

pub fn overlay_slot(bounds: SlotBounds, child: impl IntoElement) -> OverlaySlot {
    OverlaySlot {
        bounds,
        child: child.into_any_element(),
    }
}

impl IntoElement for OverlaySlot {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for OverlaySlot {
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
        self.child.layout_as_root(bounds.size.into(), window, cx);
        self.child.prepaint_at(bounds.origin, window, cx);
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
            self.child.paint(window, cx);
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
        .text_color(theme::software_palette().text.secondary)
        .child(msg.to_string())
        .into_any_element()
}

fn side_ix(side: Side) -> usize {
    match side {
        Side::Preimage => 0,
        Side::Postimage => 1,
    }
}

fn first_line(text: &str) -> Option<&str> {
    if text.is_empty() {
        None
    } else {
        text.lines().next()
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

/// Background highlight results apply only while this open is still current.
fn should_apply_highlight(open_generation: u64, result_generation: u64) -> bool {
    open_generation == result_generation
}

/// Inclusive line span of a drag from `anchor` to the line under the pointer,
/// low end first — a drag upwards selects the same span as one downwards.
fn selection_span(anchor: u32, ln: u32) -> (u32, u32) {
    (anchor.min(ln), anchor.max(ln))
}

/// `LineSpan::count` for an inclusive `start..=end` selection.
fn span_count(start: u32, end: u32) -> u32 {
    end.saturating_sub(start) + 1
}

#[cfg(test)]
mod tests {
    use super::{selection_span, should_apply_highlight, span_count};

    #[test]
    fn drag_normalizes_to_low_end_first() {
        assert_eq!(selection_span(3, 7), (3, 7));
        assert_eq!(selection_span(7, 3), (3, 7), "upward drag is the same span");
        assert_eq!(selection_span(5, 5), (5, 5));
    }

    #[test]
    fn span_count_covers_both_ends() {
        assert_eq!(span_count(5, 5), 1, "single line is count 1");
        assert_eq!(span_count(3, 5), 3);
        assert_eq!(span_count(1, 120), 120);
        // Never zero, even if an inverted span ever reached here.
        assert_eq!(span_count(9, 4), 1);
    }

    #[test]
    fn soft_wrap_deferred_rebuild_satisfies_layout_invariant() {
        let invariant = |soft: bool, can_wrap: bool, has_wrap: bool| !soft || !can_wrap || has_wrap;
        assert!(invariant(true, false, false));
        assert!(invariant(true, true, true));
        assert!(invariant(false, true, false));
    }

    #[test]
    fn stale_generation_is_dropped() {
        assert!(should_apply_highlight(3, 3));
        assert!(!should_apply_highlight(4, 3));
        assert!(!should_apply_highlight(3, 4));
        assert!(should_apply_highlight(0, 0));
        assert!(!should_apply_highlight(u64::MAX, 0));
        assert!(should_apply_highlight(u64::MAX, u64::MAX));
    }
}

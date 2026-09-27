//! The dual pane (old | gutter | new) as its own Entity, so wheel, native
//! scroll and hover notify only this view. Still div-based; the custom Element
//! is issue 03. See docs/diffview-architecture.md §5.

use gpui::{
    AnyElement, AnyView, App, Bounds, ContentMask, Context, Div, Element, ElementId, Entity,
    EventEmitter, GlobalElementId, InspectorElementId, IntoElement, LayoutId, MouseMoveEvent,
    ParentElement, PathBuilder, Pixels, Position, Render, ScrollHandle, ScrollWheelEvent,
    SharedString, Stateful, StatefulInteractiveElement, Style, StyleRefinement, Styled, Window,
    canvas, div, fill, point, prelude::*, px, rgb, size,
};
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use crate::domain::{
    Alignment, AlignmentOp, Anchor, DiffFontSize, FoldState, Side, TokenPart, hunk_jump_target,
    match_jump_plan,
};
use crate::git::FileDiff;
use crate::ui::theme;
use super::layout::{HunkLand, Layout, LineKind, Row, SideLayout};
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
    /// Per-side row text for the div panes, indexed by visual row.
    row_text: [Vec<SharedString>; 2],
    /// Anchors of the selected path's DraftComments (comment index).
    comments: Vec<Anchor>,
    /// Line being drafted, highlighted in its pane.
    drafting: Option<(Side, u32)>,
    old_scroll: ScrollHandle,
    new_scroll: ScrollHandle,
    /// Shared scroll parameter, in pixels. See docs/dual-pane-diff.md §3.1.
    /// Kept inside `viewport::s_range` from the first measured frame on.
    scroll_s: f32,
    scroll_nudge: f32,
    applied_old: f32,
    applied_new: f32,
    /// Pane viewport height from the previous frame. 0 until the first layout.
    view_h: f32,
    /// Window y of the code panes' top edge (hover hit tests).
    pane_top_w: f32,
    /// Rows that get word marks this frame: visible ± one screen.
    marked_rows: [Range<usize>; 2],
    hover_copy: Option<String>,
    placed: Vec<PlacedBridge>,
    omit_links: Vec<(f32, f32, f32, f32)>,
    old_gaps: Vec<(f32, f32)>,
    new_gaps: Vec<(f32, f32)>,
    /// 0-based index of the Hunk at / nearest the viewport; drives chrome.
    hunk_index: Option<usize>,
    /// Hunk-navigation position (row units) set by a jump or file open, before
    /// `s_range` clamping. Jumps near the file ends move no pixels, so chrome
    /// and the next jump follow this until the user scrolls.
    hunk_s: Option<f32>,
    /// Session-level mono size for both panes and ribbons (§3.5).
    font_size: DiffFontSize,
}

impl EventEmitter<PaneEvent> for DualPane {}

impl DualPane {
    pub fn new() -> Self {
        Self {
            file: None,
            fold: FoldState::collapsed(),
            layout: None,
            row_text: [Vec::new(), Vec::new()],
            comments: Vec::new(),
            drafting: None,
            old_scroll: ScrollHandle::new(),
            new_scroll: ScrollHandle::new(),
            scroll_s: 0.,
            scroll_nudge: 0.,
            applied_old: 0.,
            applied_new: 0.,
            view_h: 0.,
            pane_top_w: 0.,
            marked_rows: [0..0, 0..0],
            hover_copy: None,
            placed: Vec::new(),
            omit_links: Vec::new(),
            old_gaps: Vec::new(),
            new_gaps: Vec::new(),
            hunk_index: None,
            hunk_s: Some(0.),
            font_size: DiffFontSize::default(),
        }
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
        self.rebuild_layout();
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
        self.row_text = [Vec::new(), Vec::new()];
        let Some(file) = self.file.as_ref() else {
            return;
        };
        let mut layout = Layout::build(
            file.old_text.clone(),
            file.new_text.clone(),
            &file.alignment,
            Some(&self.fold),
        );
        layout.set_comments(self.comments.iter());
        self.row_text = [row_texts(&layout.old), row_texts(&layout.new)];
        self.layout = Some(layout);
    }

    /// Back to the file start. The first measured frame clamps `scroll_s` up
    /// to the lower end of `s_range`.
    fn reset_scroll(&mut self, cx: &mut Context<Self>) {
        self.scroll_s = 0.;
        self.scroll_nudge = 0.;
        self.applied_old = 0.;
        self.applied_new = 0.;
        self.set_hover_copy(None, cx);
        self.placed.clear();
        self.omit_links.clear();
        self.old_gaps.clear();
        self.new_gaps.clear();
        self.old_scroll.set_offset(point(px(0.), px(0.)));
        self.new_scroll.set_offset(point(px(0.), px(0.)));
    }

    fn with_anchor(&mut self, mutate: impl FnOnce(&mut Self)) {
        let cap = self.capture_anchor();
        mutate(self);
        self.rebuild_layout();
        self.restore_anchor(cap);
        self.hunk_s = None;
    }

    fn viewport(&self) -> Option<Viewport<'_>> {
        let layout = self.layout.as_ref()?;
        Some(Viewport::new(layout, self.scroll_s, self.view_h, self.row_h()))
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
        cx.notify();
    }

    pub fn set_font_size(&mut self, op: FontOp, cx: &mut Context<Self>) {
        let prev = self.row_h();
        match op {
            FontOp::Inc => self.font_size.increase(),
            FontOp::Dec => self.font_size.decrease(),
            FontOp::Reset => self.font_size.reset(),
        }
        let next = self.row_h();
        if prev > 0. {
            self.scroll_s *= next / prev;
        }
        cx.notify();
    }

    fn on_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(window.line_height());
        // AppKit scrollingDeltaY is negative when the user scrolls down.
        self.scroll_nudge -= f32::from(delta.y);
        self.hunk_s = None;
        cx.notify();
    }

    /// Hover copy only reaches the shell's chrome; the pane does not redraw.
    fn on_gutter_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let y = f32::from(event.position.y) - self.pane_top_w;
        let copy = self.viewport().and_then(|vp| {
            let i = vp.bridge_at(y)?;
            self.layout
                .as_ref()
                .map(|layout| layout.bridges[i].position_copy())
        });
        self.set_hover_copy(copy, cx);
    }

    fn sync_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let row_h = self.row_h();
        if self.view_h < 1. {
            return;
        }
        let mut s = self.scroll_s;
        if self.scroll_nudge != 0. {
            s += self.scroll_nudge;
            self.scroll_nudge = 0.;
        } else {
            let old_top = -f32::from(self.old_scroll.offset().y);
            let new_top = -f32::from(self.new_scroll.offset().y);
            let old_delta = (old_top - self.applied_old).abs();
            let new_delta = (new_top - self.applied_new).abs();
            if old_delta > 0.5 || new_delta > 0.5 {
                let (side, top) = if old_delta >= new_delta {
                    (Side::Old, old_top)
                } else {
                    (Side::New, new_top)
                };
                let n = layout.side(side).rows();
                let content = viewport::content_from_top(top, n, self.view_h, row_h);
                s = viewport::s_for_content(layout, side, content, row_h, s);
                self.hunk_s = None;
            }
        }
        let vp = Viewport::new(layout, s, self.view_h, row_h);
        self.scroll_s = vp.s();
        let old_top = vp.top(Side::Old);
        let new_top = vp.top(Side::New);
        self.applied_old = old_top;
        self.applied_new = new_top;
        self.old_scroll.set_offset(point(px(0.), px(-old_top)));
        self.new_scroll.set_offset(point(px(0.), px(-new_top)));
        let hunk_index = nearest_hunk_index(
            self.hunk_s.unwrap_or(self.scroll_s / row_h),
            &layout.hunk_lands,
        );
        let screen = (self.view_h / row_h).ceil() as usize;
        self.marked_rows = [Side::Old, Side::New].map(|side| {
            let rows = vp.visible_rows(side);
            rows.start.saturating_sub(screen)..rows.end + screen
        });

        let old_pane = self.old_scroll.bounds();
        let new_pane = self.new_scroll.bounds();
        // Ribbon spans the whole gutter, from the left code edge to the right code edge,
        // so the pinch meets the row background on one side and the hairline on the other.
        let x_l = f32::from(old_pane.right());
        let x_r = f32::from(new_pane.left());
        let old_top_w = f32::from(old_pane.top());
        let new_top_w = f32::from(new_pane.top());
        self.pane_top_w = old_top_w;
        self.old_gaps = vp.gaps(Side::Old);
        self.new_gaps = vp.gaps(Side::New);
        self.placed.clear();
        self.omit_links.clear();
        if x_r - x_l >= 4. {
            self.placed.extend(vp.bridges().into_iter().map(|p| PlacedBridge {
                kind: p.kind,
                x_l,
                x_r,
                y_l0: old_top_w + p.y_l0,
                y_l1: old_top_w + p.y_l1,
                y_r0: new_top_w + p.y_r0,
                y_r1: new_top_w + p.y_r1,
            }));
            self.omit_links.extend(
                vp.omit_links()
                    .into_iter()
                    .map(|(y_l, y_r)| (x_l, old_top_w + y_l, x_r, new_top_w + y_r)),
            );
        }
        self.set_hunk_index(hunk_index, cx);
    }
}

impl Render for DualPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_scroll(cx);
        let Some(layout) = self.layout.as_ref() else {
            return placeholder("No Layout");
        };
        let vp = Viewport::new(layout, self.scroll_s, self.view_h, self.row_h());
        let waves = self.omit_links.clone();
        let ln_w = ln_col_width(line_number_digits(layout));
        div()
            .id("diff-panes")
            .relative()
            .size_full()
            .flex()
            .overflow_hidden()
            .bg(theme::white())
            .child(code_pane(true, layout, &vp, self, cx))
            .child(center_gutter(layout, self, cx))
            .child(code_pane(false, layout, &vp, self, cx))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        paint_omit_waves(window, bounds, &waves, ln_w);
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
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

const LN_FONT_PX: f32 = 10.;
/// Wider than Menlo/Consolas at 10px (~6px) so a digit is never clipped.
const LN_DIGIT_PX: f32 = 8.;
/// `pr_1` / `pl_1` on the column.
const LN_PAD: f32 = 4.;
const BRIDGE_COL: f32 = 24.;

fn line_number_digits(layout: &Layout) -> u32 {
    let mut digits = 0u32;
    let mut n = layout.max_line_number().max(1);
    while n > 0 {
        digits += 1;
        n /= 10;
    }
    digits.max(2)
}

fn ln_col_width(digits: u32) -> f32 {
    digits as f32 * LN_DIGIT_PX + LN_PAD
}

pub enum FontOp {
    Inc,
    Dec,
    Reset,
}

/// A visible bridge in window coordinates.
#[derive(Clone)]
struct PlacedBridge {
    kind: LineKind,
    x_l: f32,
    x_r: f32,
    y_l0: f32,
    y_l1: f32,
    y_r0: f32,
    y_r1: f32,
}

fn side_ix(side: Side) -> usize {
    match side {
        Side::Old => 0,
        Side::New => 1,
    }
}

pub fn placeholder(msg: &str) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme::muted())
        .child(msg.to_string())
        .into_any_element()
}

fn code_pane(
    left: bool,
    layout: &Layout,
    vp: &Viewport<'_>,
    view: &DualPane,
    cx: &mut Context<DualPane>,
) -> Div {
    let id = if left { "code-left" } else { "code-right" };
    let side = if left { Side::Old } else { Side::New };
    let rows = layout.side(side);
    let texts = &view.row_text[side_ix(side)];
    let marked_rows = view.marked_rows[side_ix(side)].clone();
    let empty = rows.is_empty();
    let gaps = if left {
        view.old_gaps.clone()
    } else {
        view.new_gaps.clone()
    };
    let seam_rows = vp.visible_seams(side).to_vec();
    let seam_color = if left { theme::add_bg() } else { theme::del_bg() };
    let seam_scroll = if left { view.applied_old } else { view.applied_new };
    let seam = vp.empty_seam(side);
    let handle = if left {
        view.old_scroll.clone()
    } else {
        view.new_scroll.clone()
    };
    let drafting_line = view
        .drafting
        .and_then(|(s, line)| (s == side).then_some(line));
    let row_h = view.row_h();
    let font_px = view.font_size.px() as f32;
    let pad = vp.content_pad(side);
    let measure = left.then(|| cx.entity().downgrade());

    div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .child(
            div()
                .id(id)
                .size_full()
                .overflow_y_scroll()
                // ponytail: GPUI draws the thumb on the trailing edge. The old pane's
                // outer-left bar would be a custom thumb; wheel coupling is what the bridges use.
                .track_scroll(&handle)
                .on_scroll_wheel(cx.listener(DualPane::on_wheel))
                .font_family(theme::MONO_FONT)
                .text_size(px(font_px))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .children(rows.iter_rows().enumerate().map(|(i, row)| {
                            let (line, omit_id) = match row {
                                Row::Line(l) => (Some(l), None),
                                Row::Omit(o) => (None, Some(o.id)),
                            };
                            let ln = line.map(|l| l.ln).unwrap_or(0);
                            let marked = line.is_some_and(|l| rows.has_comment(l.ln));
                            let drafting_here = line.is_some() && drafting_line == Some(ln);
                            let bg = if drafting_here {
                                rgb(0xdbe4ff)
                            } else {
                                kind_bg(row_kind(row))
                            };
                            let row_id = if left { ("row-l", i) } else { ("row-r", i) };
                            // Word marks only near the viewport; the LCS is memoized per block.
                            let parts = line
                                .filter(|_| marked_rows.contains(&i))
                                .and_then(|l| layout.marks(side, l));
                            let text = texts.get(i).cloned().unwrap_or_default();
                            div()
                                .id(row_id)
                                .relative()
                                .h(px(row_h))
                                .px_3()
                                .bg(bg)
                                .text_color(theme::text())
                                .overflow_hidden()
                                .when(marked, |d| {
                                    d.border_l_2().border_color(theme::accent())
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(id) = omit_id {
                                        this.expand_omit(id, cx);
                                    } else {
                                        cx.emit(PaneEvent::BeginDraft { side, ln });
                                    }
                                }))
                                .child(render_row_text(text, parts))
                                .when(rows.is_seam(i), |row| {
                                    row.child(seam_hairline(seam_color))
                                })
                        }))
                        .when(pad > 0., |col| col.child(div().h(px(pad)).w_full())),
                ),
        )
        .child(
            canvas(
                move |bounds, _, cx| {
                    if let Some(measure) = &measure {
                        let h = f32::from(bounds.size.height);
                        measure
                            .update(cx, |this, cx| {
                                if (this.view_h - h).abs() > 0.5 {
                                    this.view_h = h;
                                    cx.notify();
                                }
                            })
                            .ok();
                    }
                },
                move |bounds, _, window, _| {
                    paint_gaps(window, bounds, &gaps, empty, seam);
                    for idx in &seam_rows {
                        let y = *idx as f32 * row_h - seam_scroll;
                        let rect = Bounds {
                            origin: point(bounds.left(), bounds.top() + px(y)),
                            size: size(bounds.size.width, px(2.)),
                        };
                        window.paint_quad(fill(rect, seam_color));
                    }
                },
            )
            .absolute()
            .size_full(),
        )
}

fn center_gutter(layout: &Layout, view: &DualPane, cx: &mut Context<DualPane>) -> Stateful<Div> {
    let placed = view.placed.clone();
    let ln_w = ln_col_width(line_number_digits(layout));
    let row_h = view.row_h();
    div()
        .id("gutter")
        .relative()
        .overflow_hidden()
        .w(px(ln_w * 2. + BRIDGE_COL))
        .h_full()
        .flex_none()
        .bg(theme::white())
        .on_scroll_wheel(cx.listener(DualPane::on_wheel))
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !hovered {
                this.set_hover_copy(None, cx);
            }
        }))
        .on_mouse_move(cx.listener(DualPane::on_gutter_move))
        .child(ln_col(true, &layout.old, view.applied_old, row_h, ln_w, false))
        .child(ln_col(false, &layout.new, view.applied_new, row_h, ln_w, false))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        paint_bridges(window, &placed, ln_w);
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .child(ln_col(true, &layout.old, view.applied_old, row_h, ln_w, true))
        .child(ln_col(false, &layout.new, view.applied_new, row_h, ln_w, true))
}

fn ln_col(
    left: bool,
    rows: &SideLayout,
    scroll_top: f32,
    row_h: f32,
    col_w: f32,
    labels_only: bool,
) -> Stateful<Div> {
    let id = if labels_only {
        if left { "ln-left-text" } else { "ln-right-text" }
    } else if left {
        "ln-left"
    } else {
        "ln-right"
    };
    let seam_color = if left { theme::add_bg() } else { theme::del_bg() };
    div()
        .id(id)
        .absolute()
        .top(px(-scroll_top))
        .w(px(col_w))
        .overflow_hidden()
        .whitespace_nowrap()
        .when(left, |col| col.left(px(0.)))
        .when(!left, |col| col.right(px(0.)))
        .font_family(theme::line_number_font())
        .text_size(px(LN_FONT_PX))
        .text_color(theme::faint())
        .when(left, |col| col.text_right().pr_1())
        .when(!left, |col| col.pl_1())
        .children(rows.iter_rows().enumerate().map(|(i, row)| {
            let ln_id = if left {
                (if labels_only { "ln-lt" } else { "ln-l" }, i)
            } else {
                (if labels_only { "ln-rt" } else { "ln-r" }, i)
            };
            let label = match row {
                Row::Line(line) => line.ln.to_string(),
                Row::Omit(_) => String::new(),
            };
            let is_seam = !labels_only && rows.is_seam(i);
            div()
                .id(ln_id)
                .relative()
                .w_full()
                .h(px(row_h))
                .whitespace_nowrap()
                .overflow_hidden()
                .when(!labels_only, |cell| cell.bg(kind_bg(row_kind(row))))
                .child(label)
                .when(is_seam, |cell| cell.child(seam_hairline(seam_color)))
        }))
}


fn seam_hairline(color: gpui::Rgba) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(2.))
        .bg(color)
}

fn nearest_hunk_index(s_rows: f32, lands: &[HunkLand]) -> Option<usize> {
    if lands.is_empty() {
        return None;
    }
    let mut idx = 0;
    for (i, land) in lands.iter().enumerate() {
        if land.s as f32 <= s_rows + 0.5 {
            idx = i;
        } else {
            break;
        }
    }
    Some(idx)
}

/// Row text per visual row, shared so a frame clones a pointer, not the line.
fn row_texts(side: &SideLayout) -> Vec<SharedString> {
    side.iter_rows()
        .map(|row| match row {
            Row::Line(line) => SharedString::from(side.text(line).to_string()),
            Row::Omit(_) => SharedString::default(),
        })
        .collect()
}

fn render_row_text(text: SharedString, parts: Option<&[TokenPart]>) -> gpui::AnyElement {
    match parts {
        Some(parts) if parts.iter().any(|p| p.changed) => div()
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .children(parts.iter().map(|p| {
                if p.changed {
                    div()
                        .bg(theme::mod_chg())
                        .rounded(px(2.))
                        .child(p.text.clone())
                        .into_any_element()
                } else {
                    div().child(p.text.clone()).into_any_element()
                }
            }))
            .into_any_element(),
        _ => div().child(text).into_any_element(),
    }
}

fn kind_bg(kind: Option<LineKind>) -> gpui::Rgba {
    match kind {
        Some(LineKind::Replace) => theme::mod_bg(),
        Some(LineKind::Insert) => theme::add_bg(),
        Some(LineKind::Delete) => theme::del_bg(),
        Some(LineKind::Equal) | None => theme::white(),
    }
}

fn row_kind(row: Row<'_>) -> Option<LineKind> {
    match row {
        Row::Line(line) => Some(line.kind),
        Row::Omit(_) => None,
    }
}

fn paint_bridges(window: &mut Window, placed: &[PlacedBridge], ln_w: f32) {
    for bridge in placed {
        let parallel = (bridge.y_l0 - bridge.y_r0).abs() < 1. && (bridge.y_l1 - bridge.y_r1).abs() < 1.;
        let mut path = PathBuilder::fill();
        if parallel {
            path.move_to(point(px(bridge.x_l), px(bridge.y_l0)));
            path.line_to(point(px(bridge.x_r), px(bridge.y_r0)));
            path.line_to(point(px(bridge.x_r), px(bridge.y_r1)));
            path.line_to(point(px(bridge.x_l), px(bridge.y_l1)));
        } else {
            pinch_bezier(&mut path, bridge, ln_w);
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, kind_bg(Some(bridge.kind)));
        }
    }
}

/// Full row on the long side, cubic Bézier through the center gutter, then a 2px
/// hairline across the short side's line-number column so it meets the code hairline.
fn pinch_bezier(path: &mut PathBuilder, bridge: &PlacedBridge, ln_w: f32) {
    let x_l = bridge.x_l;
    let x_r = bridge.x_r;
    let mid_l = x_l + ln_w;
    let mid_r = x_r - ln_w;
    let mw = (mid_r - mid_l).max(8.);
    match bridge.kind {
        LineKind::Delete => {
            let y0 = bridge.y_l0;
            let y1 = bridge.y_l1;
            let top = bridge.y_r0;
            let bot = bridge.y_r0 + 2.;
            path.move_to(point(px(x_l), px(y0)));
            path.line_to(point(px(mid_l), px(y0)));
            path.cubic_bezier_to(
                point(px(mid_r), px(top)),
                point(px(mid_l + mw * 0.45), px(y0)),
                point(px(mid_r - mw * 0.45), px(top)),
            );
            path.line_to(point(px(x_r), px(top)));
            path.line_to(point(px(x_r), px(bot)));
            path.line_to(point(px(mid_r), px(bot)));
            path.cubic_bezier_to(
                point(px(mid_l), px(y1)),
                point(px(mid_r - mw * 0.45), px(bot)),
                point(px(mid_l + mw * 0.45), px(y1)),
            );
            path.line_to(point(px(x_l), px(y1)));
        }
        LineKind::Insert => {
            let top = bridge.y_l0;
            let bot = bridge.y_l0 + 2.;
            let y0 = bridge.y_r0;
            let y1 = bridge.y_r1;
            path.move_to(point(px(x_l), px(top)));
            path.line_to(point(px(mid_l), px(top)));
            path.cubic_bezier_to(
                point(px(mid_r), px(y0)),
                point(px(mid_l + mw * 0.45), px(top)),
                point(px(mid_r - mw * 0.45), px(y0)),
            );
            path.line_to(point(px(x_r), px(y0)));
            path.line_to(point(px(x_r), px(y1)));
            path.line_to(point(px(mid_r), px(y1)));
            path.cubic_bezier_to(
                point(px(mid_l), px(bot)),
                point(px(mid_r - mw * 0.45), px(y1)),
                point(px(mid_l + mw * 0.45), px(bot)),
            );
            path.line_to(point(px(x_l), px(bot)));
        }
        _ => {
            // Hold the full block through each line-number column. The cubic
            // only runs between the columns, from the top of the left block
            // to the top of the right block (and bottom to bottom).
            path.move_to(point(px(x_l), px(bridge.y_l0)));
            path.line_to(point(px(mid_l), px(bridge.y_l0)));
            path.cubic_bezier_to(
                point(px(mid_r), px(bridge.y_r0)),
                point(px(mid_l + mw * 0.45), px(bridge.y_l0)),
                point(px(mid_r - mw * 0.45), px(bridge.y_r0)),
            );
            path.line_to(point(px(x_r), px(bridge.y_r0)));
            path.line_to(point(px(x_r), px(bridge.y_r1)));
            path.line_to(point(px(mid_r), px(bridge.y_r1)));
            path.cubic_bezier_to(
                point(px(mid_l), px(bridge.y_l1)),
                point(px(mid_r - mw * 0.45), px(bridge.y_r1)),
                point(px(mid_l + mw * 0.45), px(bridge.y_l1)),
            );
            path.line_to(point(px(x_l), px(bridge.y_l1)));
        }
    }
}

fn paint_omit_waves(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    folds: &[(f32, f32, f32, f32)],
    ln_w: f32,
) {
    if folds.is_empty() {
        return;
    }
    let x0 = f32::from(bounds.left());
    let x1 = f32::from(bounds.right());
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for &(gutter_l, y_l, gutter_r, y_r) in folds {
            // Bend only in the gap between the line-number columns. A slope
            // across the digits cuts through them when the folds are far apart.
            let path = joined_wave(
                x0,
                x1,
                y_l,
                gutter_l + ln_w,
                gutter_r - ln_w,
                y_r,
            );
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(0xb5b5b5));
            }
        }
    });
}

const WAVE_PERIOD: f32 = 16.;
const WAVE_AMP: f32 = 3.5;
const WAVE_STEP: f32 = 2.;

fn wave_y(x: f32, base: f32, crest_at: Option<f32>) -> f32 {
    let phase = match crest_at {
        Some(lock) => (x - lock) / WAVE_PERIOD * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2,
        None => x / WAVE_PERIOD * std::f32::consts::TAU,
    };
    base + phase.sin() * WAVE_AMP
}

fn trace_wave(
    path: &mut PathBuilder,
    x0: f32,
    x1: f32,
    base: f32,
    crest_at: Option<f32>,
    first_move: bool,
) {
    if x1 < x0 {
        return;
    }
    let mut x = x0;
    let mut moved = !first_move;
    loop {
        let xx = x.min(x1);
        let p = point(px(xx), px(wave_y(xx, base, crest_at)));
        if moved {
            path.line_to(p);
        } else {
            path.move_to(p);
            moved = true;
        }
        if xx >= x1 - 0.01 {
            break;
        }
        x += WAVE_STEP;
    }
}

/// One stroke. Each side is a horizontal sine through its code and line numbers.
/// A height change is a cubic Bézier in the gap between the line-number columns.
/// Each sine meets that curve at a crest, so both tangents are horizontal, the
/// same way the change ribbons leave a flat edge.
fn joined_wave(x0: f32, x1: f32, y_l: f32, gap_l: f32, gap_r: f32, y_r: f32) -> PathBuilder {
    let mut path = PathBuilder::stroke(px(1.25));
    if x1 - x0 < 2. {
        return path;
    }
    let gap_l = gap_l.clamp(x0, x1);
    let gap_r = (gap_l + 8.).max(gap_r).min(x1);
    if (y_r - y_l).abs() < 0.5 {
        trace_wave(&mut path, x0, x1, y_l, None, true);
        return path;
    }
    trace_wave(&mut path, x0, gap_l, y_l, Some(gap_l), true);
    let y0 = y_l + WAVE_AMP;
    let y1 = y_r + WAVE_AMP;
    let dx = (gap_r - gap_l) * 0.45;
    path.cubic_bezier_to(
        point(px(gap_r), px(y1)),
        point(px(gap_l + dx), px(y0)),
        point(px(gap_r - dx), px(y1)),
    );
    if gap_r < x1 {
        trace_wave(&mut path, gap_r, x1, y_r, Some(gap_r), false);
    }
    path
}

fn paint_gaps(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    gaps: &[(f32, f32)],
    empty: bool,
    seam: f32,
) {
    for &(y0, y1) in gaps {
        if y1 - y0 < 0.5 {
            continue;
        }
        let rect = Bounds {
            origin: point(bounds.left(), bounds.top() + px(y0)),
            size: size(bounds.size.width, px(y1 - y0)),
        };
        window.paint_quad(fill(rect, theme::gap_bg()));
        window.with_content_mask(Some(ContentMask { bounds: rect }), |window| {
            let width = f32::from(rect.size.width);
            let left = f32::from(rect.left());
            let top = f32::from(rect.top());
            let bottom = f32::from(rect.bottom());
            let mut y = top - width;
            while y < bottom {
                let mut stroke = PathBuilder::stroke(px(1.));
                stroke.move_to(point(px(left), px(y)));
                stroke.line_to(point(px(left + width), px(y + width)));
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, theme::faint());
                }
                y += 7.;
            }
        });
    }
    if empty {
        let y = bounds.top() + px(seam);
        let mut stroke = PathBuilder::stroke(px(2.));
        stroke.move_to(point(bounds.left() + px(8.), y));
        stroke.line_to(point(bounds.right() - px(8.), y));
        if let Ok(path) = stroke.build() {
            window.paint_path(path, rgb(0x8aa0b8));
        }
    }
}

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, Div, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseMoveEvent, ParentElement, PathBuilder,
    Render, ScrollHandle, ScrollWheelEvent, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, WindowControlArea, canvas, div, fill, point, prelude::*, px, rgb, size, svg,
};
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use crate::domain::{
    ChangedPath, Comparison, DiffFontSize, FoldState, PathStatus, Review, SearchMatch,
    SearchScope, Side, TokenPart, ViewOptions, hunk_jump_target, match_jump_plan, search_file,
};

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
use crate::export;
use crate::git::{self, FileDiff};
use crate::window_geometry_store;
use super::diff::layout::{HunkLand, Layout, LineKind, Row, SideLayout};
use super::diff::viewport::{self, Viewport};
use super::file_tree::{self, TreeRow};
#[cfg(target_os = "macos")]
use super::mac_column_vibrancy::ColumnVibrancy;
use super::scrollbar;
use super::splitter::{self, Axis, ResizeState};
use super::theme;
use super::window_geometry;

/// Own snapshot for the Diff window — not a live shared model with main.
#[derive(Clone, Debug)]
pub struct DiffSnapshot {
    pub comparison: Comparison,
    pub changed_paths: Vec<ChangedPath>,
    pub selected_path: String,
    pub file: FileDiff,
}

struct Drafting {
    side: Side,
    line: u32,
    body: String,
}

pub struct DiffView {
    focus: FocusHandle,
    tree_collapsed: bool,
    tree_width: f32,
    tree_resize_state: Rc<ResizeState>,
    pub snapshot: Option<DiffSnapshot>,
    review: Option<Review>,
    drafting: Option<Drafting>,
    export_status: Option<String>,
    /// Ephemeral; paths in set are collapsed. Default empty = all expanded.
    collapsed_dirs: HashSet<String>,
    /// Reset collapsed_dirs when this no longer matches current ChangedPath list.
    tree_path_fingerprint: Vec<String>,
    /// View projection of the selected text file under `fold`. Rebuilt on
    /// file / fold / ViewOptions change, never per frame.
    layout: Option<Layout>,
    /// Per-side row text for the div panes, indexed by visual row.
    row_text: [Vec<SharedString>; 2],
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
    #[cfg(target_os = "macos")]
    tree_vibrancy: Option<ColumnVibrancy>,
    /// Per-file Equal fold. Reset when the selected path changes.
    fold: FoldState,
    /// 0-based index of the Hunk at / nearest the viewport; drives chrome.
    hunk_index: Option<usize>,
    /// Hunk-navigation position (row units) set by a jump or file open, before
    /// `s_range` clamping. Jumps near the file ends move no pixels, so chrome
    /// and the next jump follow this until the user scrolls.
    hunk_s: Option<f32>,
    /// Diff-computation knobs; does not change Comparison identity.
    view_options: ViewOptions,
    /// Session-level mono size for both panes and ribbons (§3.5).
    font_size: DiffFontSize,
    /// In-file search query; empty = no hits.
    search_query: String,
    search_scope: SearchScope,
    /// When true, keystrokes edit `search_query` (like the draft bar).
    searching: bool,
    bounds_sub: Option<gpui::Subscription>,
}

enum FontOp {
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

impl DiffView {
    pub fn with_snapshot(snapshot: DiffSnapshot, cx: &mut Context<Self>) -> Self {
        let review = Review::new(snapshot.comparison.clone());
        let mut this = Self {
            focus: cx.focus_handle(),
            tree_collapsed: false,
            tree_width: f32::from(theme::DIFF_TREE_WIDTH),
            tree_resize_state: Rc::new(ResizeState::default()),
            snapshot: Some(snapshot),
            review: Some(review),
            drafting: None,
            export_status: None,
            collapsed_dirs: HashSet::new(),
            tree_path_fingerprint: Vec::new(),
            layout: None,
            row_text: [Vec::new(), Vec::new()],
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
            #[cfg(target_os = "macos")]
            tree_vibrancy: None,
            fold: FoldState::collapsed(),
            hunk_index: None,
            hunk_s: Some(0.),
            view_options: ViewOptions::default(),
            font_size: DiffFontSize::default(),
            search_query: String::new(),
            search_scope: SearchScope::Both,
            searching: false,
            bounds_sub: None,
        };
        this.rebuild_layout();
        this
    }

    fn row_h(&self) -> f32 {
        self.font_size.row_height()
    }

    /// Rebuild the Layout from the selected file, `fold` and the comment index.
    fn rebuild_layout(&mut self) {
        self.layout = None;
        self.row_text = [Vec::new(), Vec::new()];
        let Some(FileDiff::Text {
            alignment,
            old_text,
            new_text,
        }) = self.snapshot.as_ref().map(|s| &s.file)
        else {
            return;
        };
        let layout = Layout::build(
            old_text.clone(),
            new_text.clone(),
            alignment,
            Some(&self.fold),
        );
        self.row_text = [row_texts(&layout.old), row_texts(&layout.new)];
        self.layout = Some(layout);
        self.refresh_comments();
    }

    fn refresh_comments(&mut self) {
        let (Some(layout), Some(snap)) = (self.layout.as_mut(), self.snapshot.as_ref()) else {
            return;
        };
        match &self.review {
            Some(review) => layout.set_comments(
                review
                    .comments_for_path(&snap.selected_path)
                    .map(|c| &c.anchor),
            ),
            None => layout.set_comments(std::iter::empty()),
        }
    }

    /// Recompute Alignment under the current ViewOptions. Caller rebuilds the Layout.
    fn recompute_alignment(&mut self) {
        let opts = self.view_options.clone();
        let Some(snap) = self.snapshot.as_mut() else {
            return;
        };
        let FileDiff::Text {
            alignment,
            old_text,
            new_text,
        } = &mut snap.file
        else {
            return;
        };
        *alignment = git::compute_alignment(old_text, new_text, &opts);
        self.hunk_index = None;
        self.hunk_s = None;
    }

    fn toggle_ignore_whitespace(&mut self, cx: &mut Context<Self>) {
        self.view_options.ignore_whitespace = !self.view_options.ignore_whitespace;
        self.with_anchor(|this| {
            this.fold = FoldState::collapsed();
            this.recompute_alignment();
        });
        cx.notify();
    }

    /// Back to the file start. The first measured frame clamps `scroll_s` up
    /// to the lower end of `s_range`.
    fn reset_scroll(&mut self) {
        self.scroll_s = 0.;
        self.scroll_nudge = 0.;
        self.applied_old = 0.;
        self.applied_new = 0.;
        self.hover_copy = None;
        self.placed.clear();
        self.omit_links.clear();
        self.old_gaps.clear();
        self.new_gaps.clear();
        self.old_scroll.set_offset(point(px(0.), px(0.)));
        self.new_scroll.set_offset(point(px(0.), px(0.)));
    }

    fn reset_fold(&mut self) {
        self.fold = FoldState::collapsed();
        self.hunk_index = None;
        self.hunk_s = Some(0.);
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

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.with_anchor(|this| {
            let ids: Vec<usize> = this
                .snapshot
                .as_ref()
                .and_then(|s| match &s.file {
                    FileDiff::Text { alignment, .. } => Some(
                        alignment
                            .ops
                            .iter()
                            .enumerate()
                            .filter_map(|(i, op)| {
                                matches!(op, crate::domain::AlignmentOp::Equal { .. }).then_some(i)
                            })
                            .collect(),
                    ),
                    _ => None,
                })
                .unwrap_or_default();
            for i in ids {
                this.fold.expand(i);
            }
        });
        cx.notify();
    }

    fn collapse_unchanged(&mut self, cx: &mut Context<Self>) {
        self.with_anchor(|this| {
            this.fold = FoldState::collapsed();
        });
        cx.notify();
    }

    fn jump_hunk(&mut self, dir: i32, cx: &mut Context<Self>) {
        let (Some(FileDiff::Text { alignment, .. }), Some(layout)) =
            (self.snapshot.as_ref().map(|s| &s.file), self.layout.as_ref())
        else {
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
        let Some(target) = hunk_jump_target(alignment, i) else {
            return;
        };
        let s = viewport::s_for_target(layout, target, row_h, self.scroll_s).unwrap_or(self.scroll_s);
        self.scroll_s = viewport::clamp_s(layout, s, self.view_h, row_h);
        self.hunk_s = Some(s / row_h);
        self.hunk_index = Some(i);
        cx.notify();
    }

    fn jump_match(&mut self, side: Side, ln: u32, cx: &mut Context<Self>) {
        let Some(FileDiff::Text { alignment, .. }) = self.snapshot.as_ref().map(|s| &s.file) else {
            return;
        };
        let plan = match_jump_plan(alignment, &self.fold, side, ln);
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

    fn set_font_size(&mut self, op: FontOp, cx: &mut Context<Self>) {
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

    fn cycle_search_scope(&mut self, cx: &mut Context<Self>) {
        self.search_scope = match self.search_scope {
            SearchScope::Both => SearchScope::Old,
            SearchScope::Old => SearchScope::New,
            SearchScope::New => SearchScope::Both,
        };
        cx.notify();
    }

    fn current_matches(&self) -> Vec<SearchMatch> {
        let Some(FileDiff::Text {
            old_text, new_text, ..
        }) = self.snapshot.as_ref().map(|s| &s.file)
        else {
            return Vec::new();
        };
        search_file(old_text, new_text, &self.search_query, self.search_scope)
    }

    fn tree_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |requested, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                let available = f32::from(window.viewport_size().width);
                let width = splitter::clamp_diff_tree_width(requested, available);
                if this.tree_width != width {
                    this.tree_width = width;
                    cx.notify();
                }
            })
            .ok();
        })
    }

    fn sync_collapsed_dirs(&mut self) {
        let next = self
            .snapshot
            .as_ref()
            .map(|s| s.changed_paths.iter().map(|p| p.path.clone()).collect())
            .unwrap_or_default();
        if next != self.tree_path_fingerprint {
            self.tree_path_fingerprint = next;
            self.collapsed_dirs.clear();
        }
    }

    /// If Comparison matches, retarget path (+ file content); else replace snapshot.
    pub fn apply_snapshot(&mut self, incoming: DiffSnapshot) {
        match &mut self.snapshot {
            Some(current) if current.comparison == incoming.comparison => {
                current.selected_path = incoming.selected_path;
                current.changed_paths = incoming.changed_paths;
                current.file = incoming.file;
            }
            _ => {
                self.review = Some(Review::new(incoming.comparison.clone()));
                self.drafting = None;
                self.export_status = None;
                self.snapshot = Some(incoming);
            }
        }
        self.reset_fold();
        self.reset_scroll();
        self.recompute_alignment();
        self.hunk_s = Some(0.);
        self.rebuild_layout();
    }

    fn select_path(&mut self, path: String) {
        let opts = self.view_options.clone();
        let Some(snap) = &mut self.snapshot else {
            return;
        };
        if snap.selected_path == path {
            return;
        }
        let status = snap
            .changed_paths
            .iter()
            .find(|p| p.path == path)
            .map(|p| p.status)
            .unwrap_or(PathStatus::Modify);
        let file = git::file_diff(&snap.comparison, &path, status, &opts);
        snap.selected_path = path.clone();
        snap.file = file;
        self.drafting = None;
        self.reset_fold();
        self.reset_scroll();
        self.rebuild_layout();
        window_geometry_store::note_diff_selected_path(path);
        window_geometry_store::flush();
    }

    fn begin_draft(&mut self, side: Side, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.drafting = Some(Drafting {
            side,
            line,
            body: String::new(),
        });
        window.focus(&self.focus);
        cx.notify();
    }

    fn commit_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.drafting.take() else {
            return;
        };
        let body = draft.body.trim().to_string();
        if body.is_empty() {
            cx.notify();
            return;
        }
        let path = self
            .snapshot
            .as_ref()
            .map(|s| s.selected_path.clone())
            .unwrap_or_default();
        if let Some(review) = &mut self.review {
            review.add_line_comment(path, draft.side, draft.line, body);
        }
        self.refresh_comments();
        cx.notify();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.searching {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.searching = false;
                    cx.notify();
                }
                "enter" => {
                    if let Some(m) = self.current_matches().into_iter().next() {
                        self.jump_match(m.side, m.ln, cx);
                    }
                }
                "backspace" => {
                    self.search_query.pop();
                    cx.notify();
                }
                "tab" => self.cycle_search_scope(cx),
                _ => {
                    let mods = &event.keystroke.modifiers;
                    if mods.control || mods.alt || mods.platform || mods.function {
                        return;
                    }
                    if let Some(ch) = &event.keystroke.key_char {
                        self.search_query.push_str(ch);
                        cx.notify();
                    }
                }
            }
            return;
        }
        if self.drafting.is_none() {
            match event.keystroke.key.as_str() {
                "]" => self.jump_hunk(1, cx),
                "[" => self.jump_hunk(-1, cx),
                "/" => {
                    // Focus is already on DiffView; open search mode.
                    self.searching = true;
                    self.drafting = None;
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => self.commit_draft(cx),
            "escape" => {
                self.drafting = None;
                cx.notify();
            }
            "backspace" => {
                if let Some(d) = &mut self.drafting {
                    d.body.pop();
                }
                cx.notify();
            }
            _ => {
                let mods = &event.keystroke.modifiers;
                if mods.control || mods.alt || mods.platform || mods.function {
                    return;
                }
                if let Some(ch) = &event.keystroke.key_char {
                    if let Some(d) = &mut self.drafting {
                        d.body.push_str(ch);
                    }
                    cx.notify();
                }
            }
        }
    }


    fn export_to_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(review) = &self.review else {
            return;
        };
        let text = export::export_review(review);
        if text.is_empty() {
            self.export_status = Some("No DraftComments to export".into());
        } else {
            let n = review.comments.len();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.export_status = Some(format!(
                "Copied {n} comment{}",
                if n == 1 { "" } else { "s" }
            ));
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

    fn on_gutter_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let y = f32::from(event.position.y) - self.pane_top_w;
        let copy = self.viewport().and_then(|vp| {
            let i = vp.bridge_at(y)?;
            self.layout
                .as_ref()
                .map(|layout| layout.bridges[i].position_copy())
        });
        if self.hover_copy != copy {
            self.hover_copy = copy;
            cx.notify();
        }
    }

    fn sync_scroll(&mut self) {
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
        self.hunk_index = nearest_hunk_index(
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
        if x_r - x_l < 4. {
            return;
        }
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
}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.bounds_sub.is_none() {
            self.bounds_sub = Some(cx.observe_window_bounds(window, |_, window, cx| {
                window_geometry_store::set_diff_bounds(window_geometry::stored_from_window(window));
                window_geometry::debounce_flush(cx);
            }));
        }
        self.sync_collapsed_dirs();
        self.sync_scroll();
        let tree_w = if self.tree_collapsed {
            px(0.)
        } else {
            px(self.tree_width)
        };
        let show_tree_split = !self.tree_collapsed;

        #[cfg(target_os = "macos")]
        {
            let column_w = if self.tree_collapsed {
                0.
            } else {
                self.tree_width
            };
            ColumnVibrancy::ensure_synced(&mut self.tree_vibrancy, window, column_w);
        }

        div()
            .id("diff")
            .size_full()
            .flex()
            .overflow_hidden()
            .when(cfg!(not(target_os = "macos")), |d| d.bg(theme::white()))
            .font_family(theme::UI_FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.handle_key(event, cx);
            }))
            .child(render_tree_pane(self, tree_w, cx))
            .when(show_tree_split, |d| {
                d.child(splitter::handle(
                    "diff-tree-resize-handle",
                    Axis::HorizontalLeading,
                    self.tree_resize_handler(cx),
                    self.tree_resize_state.clone(),
                ))
            })
            .child(render_dual_pane(self, cx))
    }
}

fn render_tree_pane(
    view: &DiffView,
    width: gpui::Pixels,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let paths = view
        .snapshot
        .as_ref()
        .map(|s| s.changed_paths.clone())
        .unwrap_or_default();
    let selected = view
        .snapshot
        .as_ref()
        .map(|s| s.selected_path.clone())
        .unwrap_or_default();
    let rows = file_tree::flatten(&paths, &view.collapsed_dirs);

    div()
        .id("diff-tree")
        .h_full()
        .w(width)
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(theme::sidebar())
        .child(
            div()
                .h(theme::CHROME_HEIGHT)
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .pl(px(12.))
                .pr_2()
                .children(traffic_lights_space())
                .child(toggle_button("diff-tree-toggle", false, cx))
                .child(
                    div()
                        .id("diff-drag-tree")
                        .h_full()
                        .flex_1()
                        .window_control_area(WindowControlArea::Drag),
                ),
        )
        .child({
            let (scroll, sb) = scrollbar::vertical("diff-tree-sb", cx);
            scrollbar::overlay_flex(
            div()
                .id("diff-tree-body")
                .size_full()
                .px_1()
                .pt_1()
                .track_scroll(&scroll)
                .overflow_y_scroll()
                .children(rows.into_iter().enumerate().map(|(i, row)| match row {
                    TreeRow::Dir { depth, name, path } => {
                        let collapsed = view.collapsed_dirs.contains(&path);
                        let toggle_path = path.clone();
                        div()
                            .id(("ddir", i))
                            .mx_1()
                            .h(px(22.))
                            .pl(px(6. + depth as f32 * 12.))
                            .rounded_lg()
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(0xf6f8fb)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.collapsed_dirs.remove(&toggle_path) {
                                    this.collapsed_dirs.insert(toggle_path.clone());
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(16.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        svg()
                                            .size(px(16.))
                                            .path(if collapsed {
                                                "folder.svg"
                                            } else {
                                                "folder_open.svg"
                                            })
                                            .text_color(theme::muted()),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(16.))
                                    .flex()
                                    .items_center()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme::muted())
                                    .child(name),
                            )
                    }
                    TreeRow::File { depth, path } => {
                        let path_click = path.path.clone();
                        let active = selected == path.path;
                        let name = path.file_name().to_string();
                        let status = path.status;
                        let add = path.additions;
                        let del = path.deletions;
                        div()
                            .id(("dfile", i))
                            .mx_1()
                            .h(px(22.))
                            .pl(px(6. + depth as f32 * 12.))
                            .pr_1()
                            .rounded_lg()
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .when(active, |d| d.bg(theme::range()))
                            .hover(move |d| {
                                if active {
                                    d.bg(theme::range())
                                } else {
                                    d.bg(rgb(0xf6f8fb))
                                }
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_path(path_click.clone());
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(16.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .font_family(theme::MONO_FONT)
                                    .text_xs()
                                    .text_color(match status {
                                        PathStatus::Add => rgb(0x1a7f4b),
                                        PathStatus::Delete => rgb(0xb42318),
                                        PathStatus::Modify => rgb(0x9a6700),
                                    })
                                    .child(status.letter()),
                            )
                            .child(
                                div()
                                    .h(px(16.))
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .items_center()
                                    .text_xs()
                                    .text_color(theme::text())
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(name),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(16.))
                                    .flex()
                                    .items_center()
                                    .font_family(theme::MONO_FONT)
                                    .text_xs()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_color(rgb(0x1a7f4b))
                                            .child(format!("+{add}")),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(0xb42318))
                                            .child(format!("−{del}")),
                                    ),
                            )
                    }
                })),
            sb,
            )
        })
}

fn render_dual_pane(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let (path, subtitle) = match &view.snapshot {
        Some(s) => {
            let n = view
                .review
                .as_ref()
                .map(|r| r.comments_for_path(&s.selected_path).count())
                .unwrap_or(0);
            let mut sub = match &s.file {
                FileDiff::Text { .. } => {
                    let hunk_count = view.layout.as_ref().map(|l| l.hunk_count).unwrap_or(0);
                    let hunk_part = if hunk_count == 0 {
                        "0 differences".into()
                    } else {
                        let n = view.hunk_index.unwrap_or(0) + 1;
                        format!("hunk {n} of {hunk_count}")
                    };
                    format!(
                        "{hunk_part} · {n} comment{}",
                        if n == 1 { "" } else { "s" }
                    )
                }
                FileDiff::Binary => "binary file".into(),
                FileDiff::Error(e) => e.clone(),
            };
            if let Some(copy) = &view.hover_copy {
                sub = format!("{sub} · {copy}");
            }
            (s.selected_path.clone(), sub)
        }
        None => ("—".into(), "No file selected".into()),
    };

    div()
        .id("diff-main")
        .h_full()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .bg(theme::white())
        .child(
            div()
                .h(theme::CHROME_HEIGHT)
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .when(view.tree_collapsed, |row| {
                    row.pl(px(12.))
                        .pr_3()
                        .children(traffic_lights_space())
                        .child(toggle_button("diff-tree-toggle-collapsed", true, cx))
                })
                .when(!view.tree_collapsed, |row| row.px_3())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .font_family(theme::MONO_FONT)
                        .text_xs()
                        .text_color(theme::muted())
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(path),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(subtitle),
                )
                .child(chrome_button("prev-hunk", "↑", cx, |this, cx| {
                    this.jump_hunk(-1, cx);
                }))
                .child(chrome_button("next-hunk", "↓", cx, |this, cx| {
                    this.jump_hunk(1, cx);
                }))
                .child(chrome_button("expand-all", "Expand", cx, |this, cx| {
                    this.expand_all(cx);
                }))
                .child(chrome_button("collapse-eq", "Collapse", cx, |this, cx| {
                    this.collapse_unchanged(cx);
                }))
                .child(chrome_toggle(
                    "ignore-ws",
                    "Ignore WS",
                    view.view_options.ignore_whitespace,
                    cx,
                    |this, cx| this.toggle_ignore_whitespace(cx),
                ))
                .child(chrome_button("font-dec", "A−", cx, |this, cx| {
                    this.set_font_size(FontOp::Dec, cx);
                }))
                .child(chrome_button("font-reset", "A", cx, |this, cx| {
                    this.set_font_size(FontOp::Reset, cx);
                }))
                .child(chrome_button("font-inc", "A+", cx, |this, cx| {
                    this.set_font_size(FontOp::Inc, cx);
                }))
                .child(chrome_toggle(
                    "find",
                    "Find",
                    view.searching,
                    cx,
                    |this, cx| {
                        if this.searching {
                            this.searching = false;
                            cx.notify();
                        } else {
                            this.searching = true;
                            this.drafting = None;
                            cx.notify();
                        }
                    },
                ))
                .child(export_button(cx))
                .children(view.export_status.as_ref().map(|status| {
                    div()
                        .text_xs()
                        .text_color(theme::accent())
                        .child(status.clone())
                }))
                .child(
                    div()
                        .id("diff-drag-main")
                        .w(px(40.))
                        .h_full()
                        .window_control_area(WindowControlArea::Drag),
                ),
        )
        .child(render_search_bar(view, cx))
        .child(render_body(view, cx))
        .child(render_comments(view, cx))
        .child(render_draft_bar(view))
}

fn render_search_bar(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    if !view.searching && view.search_query.is_empty() {
        return div().into_any_element();
    }
    let matches = view.current_matches();
    let scope_label = match view.search_scope {
        SearchScope::Old => "Old",
        SearchScope::New => "New",
        SearchScope::Both => "Both",
    };
    let query_display = if view.searching {
        format!("{}▌", view.search_query)
    } else {
        view.search_query.clone()
    };
    let hint = if view.searching {
        "Find — type, Tab scope, Enter first hit, Esc close"
    } else {
        "Find"
    };

    div()
        .flex_none()
        .border_b_1()
        .border_color(theme::line())
        .bg(rgb(0xfafbfd))
        .px_3()
        .py_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(hint.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .font_family(theme::MONO_FONT)
                        .text_xs()
                        .text_color(theme::text())
                        .child(query_display),
                )
                .child(chrome_toggle(
                    "search-scope",
                    scope_label,
                    true,
                    cx,
                    |this, cx| this.cycle_search_scope(cx),
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(format!("{} hit{}", matches.len(), if matches.len() == 1 { "" } else { "s" })),
                ),
        )
        .when(!matches.is_empty(), |bar| {
            bar.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(matches.into_iter().enumerate().map(|(i, m)| {
                        let side = m.side;
                        let ln = m.ln;
                        let label = format!("{} {ln}", m.side.label());
                        div()
                            .id(("hit", i))
                            .h(theme::TOGGLE_SIZE)
                            .px_2()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(theme::muted())
                            .hover(|button| button.bg(theme::hover()))
                            .active(|button| button.bg(rgb(0xdfe3e9)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.jump_match(side, ln, cx);
                            }))
                            .child(label)
                    })),
            )
        })
        .into_any_element()
}

fn render_body(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    match (view.snapshot.as_ref().map(|s| &s.file), view.layout.as_ref()) {
        (Some(FileDiff::Text { .. }), Some(layout)) => {
            let vp = Viewport::new(layout, view.scroll_s, view.view_h, view.row_h());
            let waves = view.omit_links.clone();
            let ln_w = ln_col_width(line_number_digits(layout));
            div()
                .id("diff-panes")
                .relative()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .overflow_hidden()
                .child(code_pane(true, layout, &vp, view, cx))
                .child(center_gutter(layout, view, cx))
                .child(code_pane(false, layout, &vp, view, cx))
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
        (Some(FileDiff::Text { .. }), None) => placeholder("No Layout"),
        (Some(FileDiff::Binary), _) => placeholder("Binary file — no Alignment"),
        (Some(FileDiff::Error(msg)), _) => placeholder(msg),
        (None, _) => placeholder("Open Diff from the main window"),
    }
}

fn render_comments(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let path = view
        .snapshot
        .as_ref()
        .map(|s| s.selected_path.clone())
        .unwrap_or_default();
    let comments: Vec<_> = view
        .review
        .as_ref()
        .map(|r| r.comments_for_path(&path).cloned().collect())
        .unwrap_or_default();

    if comments.is_empty() {
        return div().into_any_element();
    }

    let (scroll, sb) = scrollbar::vertical("diff-comments-sb", cx);
    div()
        .flex_none()
        .max_h(px(160.))
        .border_t_1()
        .border_color(theme::line())
        .bg(rgb(0xfafbfd))
        .child(scrollbar::overlay_max(
            px(160.),
            div()
                .id("diff-comments-scroll")
                .w_full()
                .max_h(px(160.))
                .px_3()
                .py_2()
                .track_scroll(&scroll)
                .overflow_y_scroll()
                .children(comments.into_iter().map(|c| {
            let label = match &c.anchor {
                crate::domain::Anchor::Line { side, span, .. } => {
                    format!("{} L{} · ", side.label(), span.start)
                }
                crate::domain::Anchor::File { .. } => "file · ".into(),
            };
            div()
                .id(("cmt", c.id as usize))
                .text_xs()
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            div()
                                .font_family(theme::MONO_FONT)
                                .text_color(theme::faint())
                                .child(label),
                        )
                        .child(div().text_color(theme::text()).child(c.body)),
                )
        })),
            sb,
        ))
        .into_any_element()
}

fn render_draft_bar(view: &DiffView) -> impl IntoElement {
    let Some(draft) = &view.drafting else {
        return div().into_any_element();
    };
    let hint = format!(
        "DraftComment · {} L{} — type, Enter to save, Esc to cancel",
        draft.side.label(),
        draft.line
    );
    div()
        .flex_none()
        .border_t_1()
        .border_color(theme::accent())
        .bg(theme::range())
        .px_3()
        .py_2()
        .child(
            div()
                .text_xs()
                .text_color(theme::muted())
                .child(hint),
        )
        .child(
            div()
                .mt_1()
                .font_family(theme::MONO_FONT)
                .text_sm()
                .text_color(theme::text())
                .child(format!("{}▌", draft.body)),
        )
        .into_any_element()
}

fn placeholder(msg: &str) -> gpui::AnyElement {
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
    view: &DiffView,
    cx: &mut Context<DiffView>,
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
        .as_ref()
        .and_then(|d| (d.side == side).then_some(d.line));
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
                .on_scroll_wheel(cx.listener(DiffView::on_wheel))
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
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(id) = omit_id {
                                        this.expand_omit(id, cx);
                                    } else {
                                        this.begin_draft(side, ln, window, cx);
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

fn center_gutter(layout: &Layout, view: &DiffView, cx: &mut Context<DiffView>) -> Stateful<Div> {
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
        .on_scroll_wheel(cx.listener(DiffView::on_wheel))
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !hovered && this.hover_copy.take().is_some() {
                cx.notify();
            }
        }))
        .on_mouse_move(cx.listener(DiffView::on_gutter_move))
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

fn chrome_button(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<DiffView>,
    on_click: impl Fn(&mut DiffView, &mut Context<DiffView>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(theme::TOGGLE_SIZE)
        .px_2()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_xs()
        .text_color(theme::muted())
        .hover(|button| button.bg(theme::hover()))
        .active(|button| button.bg(rgb(0xdfe3e9)))
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(label)
}

fn chrome_toggle(
    id: &'static str,
    label: &'static str,
    pressed: bool,
    cx: &mut Context<DiffView>,
    on_click: impl Fn(&mut DiffView, &mut Context<DiffView>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(theme::TOGGLE_SIZE)
        .px_2()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_xs()
        .when(pressed, |d| d.bg(theme::range()).text_color(theme::accent()))
        .when(!pressed, |d| d.text_color(theme::muted()))
        .hover(|button| button.bg(theme::hover()))
        .active(|button| button.bg(rgb(0xdfe3e9)))
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(label)
}

fn export_button(cx: &mut Context<DiffView>) -> impl IntoElement {
    div()
        .id("export")
        .w(theme::TOGGLE_SIZE)
        .h(theme::TOGGLE_SIZE)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_color(theme::muted())
        .tooltip(|_, cx| cx.new(|_| ExportTooltip).into())
        .hover(|button| button.bg(theme::hover()))
        .active(|button| button.bg(rgb(0xdfe3e9)))
        .on_click(cx.listener(|this, _, _, cx| {
            this.export_to_clipboard(cx);
        }))
        .child(
            svg()
                .size_4()
                .path("export.svg")
                .text_color(theme::muted()),
        )
}

struct ExportTooltip;

impl Render for ExportTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x273142))
            .text_xs()
            .text_color(theme::white())
            .child("Export")
    }
}

fn toggle_button(
    id: &'static str,
    collapsed: bool,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    div()
        .id(id)
        .w(theme::TOGGLE_SIZE)
        .h(theme::TOGGLE_SIZE)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(collapsed, |button| button.bg(theme::range()))
        .cursor_pointer()
        .hover(|button| button.bg(theme::hover()))
        .active(|button| button.bg(rgb(0xdfe3e9)))
        .on_click(cx.listener(|this, _, _, cx| {
            this.tree_collapsed = !this.tree_collapsed;
            cx.notify();
        }))
        .child(
            svg()
                .size_4()
                .path("sidebar_title.svg")
                .text_color(theme::muted()),
        )
}

#[cfg(target_os = "macos")]
fn traffic_lights_space() -> Option<Div> {
    Some(
        div()
            .w(px(theme::TRAFFIC_LIGHTS_WIDTH))
            .h_full()
            .flex_none(),
    )
}

#[cfg(not(target_os = "macos"))]
fn traffic_lights_space() -> Option<Div> {
    None
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

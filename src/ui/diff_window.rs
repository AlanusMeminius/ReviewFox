use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, Div, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseMoveEvent, ParentElement, PathBuilder,
    Render, ScrollHandle, ScrollWheelEvent, StatefulInteractiveElement, Styled, Window,
    WindowControlArea, canvas, div, fill, point, prelude::*, px, rgb, size, svg,
};
use std::collections::HashSet;
use std::rc::Rc;

use crate::domain::{
    Bridge, ChangedPath, Comparison, DisplayRow, DisplayRows, FoldState, HunkJumpTarget,
    PathStatus, Review, RowKind, ScrollKnot, Side, TokenPart, ViewOptions, display_rows_folded,
    hunk_jump_target, replace_marks,
};

const LN_COL: f32 = 32.;
const BRIDGE_COL: f32 = 24.;
use crate::export;
use crate::git::{self, FileDiff};
use super::file_tree::{self, TreeRow};
use super::splitter::{self, Axis, ResizeState};
use super::theme;

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
    old_scroll: ScrollHandle,
    new_scroll: ScrollHandle,
    /// Shared scroll parameter, in pixels. See docs/dual-pane-diff.md §3.1.
    scroll_s: f32,
    scroll_nudge: f32,
    applied_old: f32,
    applied_new: f32,
    /// Pane viewport height from the previous frame. 0 until the first layout.
    view_h: f32,
    hover_copy: Option<String>,
    hover_bands: Vec<HoverBand>,
    placed: Vec<PlacedBridge>,
    omit_links: Vec<(f32, f32, f32, f32)>,
    old_gaps: Vec<(f32, f32)>,
    new_gaps: Vec<(f32, f32)>,
    /// Per-file Equal fold. Reset when the selected path changes.
    fold: FoldState,
    /// 0-based index of the Hunk at / nearest the viewport; drives chrome.
    hunk_index: Option<usize>,
    /// Diff-computation knobs; does not change Comparison identity.
    view_options: ViewOptions,
}

struct HoverBand {
    top: f32,
    bottom: f32,
    copy: String,
}

struct AnchorCap {
    side: Side,
    ln: u32,
    view_y: f32,
}

#[derive(Clone)]
struct PlacedBridge {
    kind: RowKind,
    points: Vec<(f32, f32)>,
}

impl DiffView {
    pub fn with_snapshot(snapshot: DiffSnapshot, cx: &mut Context<Self>) -> Self {
        let review = Review::new(snapshot.comparison.clone());
        Self {
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
            old_scroll: ScrollHandle::new(),
            new_scroll: ScrollHandle::new(),
            scroll_s: 0.,
            scroll_nudge: 0.,
            applied_old: 0.,
            applied_new: 0.,
            view_h: 0.,
            hover_copy: None,
            hover_bands: Vec::new(),
            placed: Vec::new(),
            omit_links: Vec::new(),
            old_gaps: Vec::new(),
            new_gaps: Vec::new(),
            fold: FoldState::collapsed(),
            hunk_index: None,
            view_options: ViewOptions::default(),
        }
    }

    fn recompute_alignment(&mut self) {
        let opts = self.view_options.clone();
        let fold = self.fold.clone();
        let Some(snap) = self.snapshot.as_mut() else {
            return;
        };
        let FileDiff::Text {
            display,
            hunk_count,
            alignment,
            old_text,
            new_text,
        } = &mut snap.file
        else {
            return;
        };
        *alignment = git::compute_alignment(old_text, new_text, &opts);
        *hunk_count = alignment.hunks().len();
        *display = display_rows_folded(old_text, new_text, alignment, &fold);
        self.hunk_index = None;
    }

    fn toggle_ignore_whitespace(&mut self, cx: &mut Context<Self>) {
        self.view_options.ignore_whitespace = !self.view_options.ignore_whitespace;
        self.with_anchor(|this| {
            this.fold = FoldState::collapsed();
            this.recompute_alignment();
        });
        cx.notify();
    }

    fn reset_scroll(&mut self) {
        self.scroll_s = 0.;
        self.scroll_nudge = 0.;
        self.applied_old = 0.;
        self.applied_new = 0.;
        self.hover_copy = None;
        self.hover_bands.clear();
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
    }

    fn reproject_fold(&mut self) {
        let Some(snap) = self.snapshot.as_mut() else {
            return;
        };
        let FileDiff::Text {
            display,
            alignment,
            old_text,
            new_text,
            ..
        } = &mut snap.file
        else {
            return;
        };
        *display = display_rows_folded(old_text, new_text, alignment, &self.fold);
    }

    fn with_anchor(&mut self, mutate: impl FnOnce(&mut Self)) {
        let cap = self.capture_anchor();
        mutate(self);
        self.reproject_fold();
        self.restore_anchor(cap);
    }

    fn capture_anchor(&self) -> Option<AnchorCap> {
        let snap = self.snapshot.as_ref()?;
        let FileDiff::Text { display, .. } = &snap.file else {
            return None;
        };
        let row_h = f32::from(theme::ROW_HEIGHT);
        let anchor = if self.view_h < 1. {
            0.
        } else {
            self.view_h / 3.
        };
        let old_top = -f32::from(self.old_scroll.offset().y);
        let new_top = -f32::from(self.new_scroll.offset().y);
        let old_hit = row_at(&display.old_rows, old_top, anchor, row_h);
        let new_hit = row_at(&display.new_rows, new_top, anchor, row_h);
        let hold_above = |side: Side, rows: &[DisplayRow], hit_i: usize, scroll: f32| {
            for p in (0..hit_i).rev() {
                if !matches!(rows[p].kind, RowKind::Omit { .. }) {
                    return Some(AnchorCap {
                        side,
                        ln: rows[p].ln,
                        view_y: p as f32 * row_h - scroll,
                    });
                }
            }
            None
        };
        if let Some(i) = old_hit {
            if matches!(display.old_rows[i].kind, RowKind::Omit { .. }) {
                return hold_above(Side::Old, &display.old_rows, i, old_top);
            }
        }
        if let Some(i) = new_hit {
            if matches!(display.new_rows[i].kind, RowKind::Omit { .. }) {
                return hold_above(Side::New, &display.new_rows, i, new_top);
            }
        }
        if let Some(i) = old_hit {
            return Some(AnchorCap {
                side: Side::Old,
                ln: display.old_rows[i].ln,
                view_y: i as f32 * row_h - old_top,
            });
        }
        if let Some(i) = new_hit {
            return Some(AnchorCap {
                side: Side::New,
                ln: display.new_rows[i].ln,
                view_y: i as f32 * row_h - new_top,
            });
        }
        None
    }

    fn restore_anchor(&mut self, cap: Option<AnchorCap>) {
        let Some(cap) = cap else {
            return;
        };
        let Some(FileDiff::Text { display, .. }) = self.snapshot.as_ref().map(|s| &s.file) else {
            return;
        };
        let row_h = f32::from(theme::ROW_HEIGHT);
        let rows = if cap.side == Side::Old {
            &display.old_rows
        } else {
            &display.new_rows
        };
        let Some(i) = rows
            .iter()
            .position(|r| r.ln == cap.ln && !matches!(r.kind, RowKind::Omit { .. }))
        else {
            return;
        };
        let anchor = if self.view_h < 1. {
            0.
        } else {
            self.view_h / 3.
        };
        let want = anchor + i as f32 * row_h - cap.view_y;
        let end = display.knots.last().map(|k| k.s as f32 * row_h).unwrap_or(0.);
        self.scroll_s = s_from(
            cap.side == Side::Old,
            want,
            &display.knots,
            row_h,
            self.scroll_s,
        )
        .clamp(0., end);
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
        let Some(FileDiff::Text {
            display,
            alignment,
            ..
        }) = self.snapshot.as_ref().map(|s| &s.file)
        else {
            return;
        };
        let hunk_count = alignment.hunks().len();
        if hunk_count == 0 {
            return;
        }
        let lands = display.hunk_lands.clone();
        let knots = display.knots.clone();
        let old_rows = display.old_rows.clone();
        let new_rows = display.new_rows.clone();
        let row_h = f32::from(theme::ROW_HEIGHT);
        let s_rows = self.scroll_s / row_h;
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
        let end = knots.last().map(|k| k.s as f32 * row_h).unwrap_or(0.);
        self.scroll_s = scroll_s_for_target(target, &old_rows, &new_rows, &knots, row_h, self.scroll_s)
            .clamp(0., end);
        self.hunk_index = Some(i);
        cx.notify();
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
                self.reset_fold();
                self.reset_scroll();
                self.recompute_alignment();
            }
            _ => {
                self.review = Some(Review::new(incoming.comparison.clone()));
                self.drafting = None;
                self.export_status = None;
                self.snapshot = Some(incoming);
                self.reset_fold();
                self.reset_scroll();
                self.recompute_alignment();
            }
        }
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
        snap.selected_path = path;
        snap.file = file;
        self.drafting = None;
        self.reset_fold();
        self.reset_scroll();
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
        cx.notify();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.drafting.is_none() {
            match event.keystroke.key.as_str() {
                "]" => self.jump_hunk(1, cx),
                "[" => self.jump_hunk(-1, cx),
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

    fn has_comment(&self, side: Side, line: u32) -> bool {
        let Some(path) = self.snapshot.as_ref().map(|s| s.selected_path.as_str()) else {
            return false;
        };
        let Some(review) = &self.review else {
            return false;
        };
        review
            .comments_for_path(path)
            .any(|c| c.anchor.line_on(side, line))
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
        cx.notify();
    }

    fn sync_scroll(&mut self) {
        let Some(FileDiff::Text { display, .. }) = self.snapshot.as_ref().map(|s| &s.file) else {
            return;
        };
        let knots = display.knots.clone();
        let bridges = display.bridges.clone();
        let old_rows = display.old_rows.clone();
        let new_rows = display.new_rows.clone();
        let hunk_lands = display.hunk_lands.clone();
        let old_n = old_rows.len();
        let new_n = new_rows.len();
        let row_h = f32::from(theme::ROW_HEIGHT);
        if self.view_h < 1. {
            return;
        }
        let anchor = self.view_h / 3.;
        let end = knots.last().map(|k| k.s as f32 * row_h).unwrap_or(0.);
        if self.scroll_nudge != 0. {
            self.scroll_s = (self.scroll_s + self.scroll_nudge).clamp(0., end);
            self.scroll_nudge = 0.;
        } else {
            let old_top = -f32::from(self.old_scroll.offset().y);
            let new_top = -f32::from(self.new_scroll.offset().y);
            let old_delta = (old_top - self.applied_old).abs();
            let new_delta = (new_top - self.applied_new).abs();
            if old_delta > 0.5 || new_delta > 0.5 {
                let from_old = old_delta >= new_delta;
                let (top, n) = if from_old {
                    (old_top, old_n)
                } else {
                    (new_top, new_n)
                };
                let content = content_from_scroll(top, n, anchor, row_h);
                self.scroll_s =
                    s_from(from_old, content, &knots, row_h, self.scroll_s).clamp(0., end);
            }
        }
        let (old_y, new_y) = interp(self.scroll_s, &knots, row_h);
        let old_top = track(old_y, old_n, anchor, row_h);
        let new_top = track(new_y, new_n, anchor, row_h);
        self.applied_old = old_top;
        self.applied_new = new_top;
        self.old_scroll.set_offset(point(px(0.), px(-old_top)));
        self.new_scroll.set_offset(point(px(0.), px(-new_top)));
        self.hunk_index = nearest_hunk_index(self.scroll_s / row_h, &hunk_lands);

        let old_pane = self.old_scroll.bounds();
        let new_pane = self.new_scroll.bounds();
        let x_l = f32::from(old_pane.right()) + LN_COL;
        let x_r = f32::from(new_pane.left()) - LN_COL;
        let old_top_w = f32::from(old_pane.top());
        let new_top_w = f32::from(new_pane.top());
        self.old_gaps = gap_intervals(true, &bridges, old_n, new_n, old_top, new_top, self.view_h, row_h);
        self.new_gaps = gap_intervals(false, &bridges, old_n, new_n, old_top, new_top, self.view_h, row_h);
        self.placed.clear();
        self.omit_links.clear();
        self.hover_bands.clear();
        if x_r - x_l < 4. {
            return;
        }
        for bridge in &bridges {
            let Some(placed) = place_bridge(
                bridge,
                old_n,
                new_n,
                old_top,
                new_top,
                old_top_w,
                new_top_w,
                self.view_h,
                row_h,
                x_l,
                x_r,
            ) else {
                continue;
            };
            let mut y0 = f32::MAX;
            let mut y1 = f32::MIN;
            for (_, y) in &placed.points {
                y0 = y0.min(*y);
                y1 = y1.max(*y);
            }
            self.hover_bands.push(HoverBand {
                top: y0,
                bottom: y1,
                copy: bridge.position_copy(),
            });
            self.placed.push(placed);
        }
        for (i, row) in old_rows.iter().enumerate() {
            let RowKind::Omit { id, .. } = row.kind else {
                continue;
            };
            let Some(j) = new_rows.iter().position(|r| match r.kind {
                RowKind::Omit { id: nid, .. } => nid == id,
                _ => false,
            }) else {
                continue;
            };
            let y_l = old_top_w + i as f32 * row_h + row_h / 2. - old_top;
            let y_r = new_top_w + j as f32 * row_h + row_h / 2. - new_top;
            self.omit_links.push((x_l, y_l, x_r, y_r));
        }
    }

}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_collapsed_dirs();
        self.sync_scroll();
        let tree_w = if self.tree_collapsed {
            px(0.)
        } else {
            px(self.tree_width)
        };
        let show_tree_split = !self.tree_collapsed;

        div()
            .id("diff")
            .size_full()
            .flex()
            .overflow_hidden()
            .bg(theme::white())
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
        .child(
            div()
                .id("diff-tree-body")
                .flex_1()
                .min_h(px(0.))
                .px_1()
                .pt_1()
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
        )
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
                FileDiff::Text { hunk_count, .. } => {
                    let hunk_part = if *hunk_count == 0 {
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
        .child(render_body(view, cx))
        .child(render_comments(view))
        .child(render_draft_bar(view))
}

fn render_body(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    match view.snapshot.as_ref().map(|s| &s.file) {
        Some(FileDiff::Text { display, .. }) => div()
            .id("diff-panes")
            .flex_1()
            .min_h(px(0.))
            .flex()
            .overflow_hidden()
            .child(code_pane(true, display, view, cx))
            .child(center_gutter(display, view, cx))
            .child(code_pane(false, display, view, cx))
            .into_any_element(),
        Some(FileDiff::Binary) => placeholder("Binary file — no Alignment"),
        Some(FileDiff::Error(msg)) => placeholder(msg),
        None => placeholder("Open Diff from the main window"),
    }
}

fn render_comments(view: &DiffView) -> impl IntoElement {
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

    div()
        .flex_none()
        .max_h(px(160.))
        .overflow_hidden()
        .border_t_1()
        .border_color(theme::line())
        .bg(rgb(0xfafbfd))
        .px_3()
        .py_2()
        .gap_1()
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
        }))
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
    display: &DisplayRows,
    view: &DiffView,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let id = if left { "code-left" } else { "code-right" };
    let side = if left { Side::Old } else { Side::New };
    let rows = if left {
        display.old_rows.clone()
    } else {
        display.new_rows.clone()
    };
    let empty = rows.is_empty();
    let other_n = if left {
        display.new_rows.len()
    } else {
        display.old_rows.len()
    };
    let gaps = if left {
        view.old_gaps.clone()
    } else {
        view.new_gaps.clone()
    };
    let seam = empty_seam(other_n, view.view_h, f32::from(theme::ROW_HEIGHT));
    let handle = if left {
        view.old_scroll.clone()
    } else {
        view.new_scroll.clone()
    };
    let drafting_line = view
        .drafting
        .as_ref()
        .and_then(|d| (d.side == side).then_some(d.line));
    let row_h = f32::from(theme::ROW_HEIGHT);
    let pad = content_pad(rows.len(), view.view_h, row_h);
    let measure = left.then(|| cx.entity().downgrade());
    let side_marks = replace_side_marks(left, display);

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
                .text_xs()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .children(rows.into_iter().enumerate().map(move |(i, row)| {
                            let marked = view.has_comment(side, row.ln);
                            let drafting_here = drafting_line == Some(row.ln);
                            let is_omit = matches!(row.kind, RowKind::Omit { .. });
                            let omit_id = match row.kind {
                                RowKind::Omit { id, .. } => Some(id),
                                _ => None,
                            };
                            let bg = if drafting_here {
                                rgb(0xdbe4ff)
                            } else if is_omit {
                                rgb(0xf0f3f7)
                            } else {
                                kind_bg(row.kind)
                            };
                            let row_id = if left { ("row-l", i) } else { ("row-r", i) };
                            let line = row.ln;
                            let parts = side_marks.get(i).cloned().flatten();
                            div()
                                .id(row_id)
                                .h(theme::ROW_HEIGHT)
                                .px_3()
                                .bg(bg)
                                .text_color(if is_omit {
                                    theme::muted()
                                } else {
                                    theme::text()
                                })
                                .overflow_hidden()
                                .when(marked && !is_omit, |d| {
                                    d.border_l_2().border_color(theme::accent())
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(id) = omit_id {
                                        this.expand_omit(id, cx);
                                    } else {
                                        this.begin_draft(side, line, window, cx);
                                    }
                                }))
                                .child(render_row_text(row.text, parts))
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
                },
            )
            .absolute()
            .size_full(),
        )
}

fn center_gutter(
    display: &DisplayRows,
    view: &DiffView,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let placed = view.placed.clone();
    let omit_links = view.omit_links.clone();
    div()
        .id("gutter")
        .relative()
        .overflow_hidden()
        .w(theme::GUTTER_WIDTH)
        .h_full()
        .flex_none()
        .bg(theme::white())
        .on_scroll_wheel(cx.listener(DiffView::on_wheel))
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !hovered && this.hover_copy.take().is_some() {
                cx.notify();
            }
        }))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            let y = f32::from(event.position.y);
            let copy = this
                .hover_bands
                .iter()
                .find(|band| y >= band.top && y <= band.bottom)
                .map(|band| band.copy.clone());
            if this.hover_copy != copy {
                this.hover_copy = copy;
                cx.notify();
            }
        }))
        .child(ln_col(true, &display.old_rows, view.applied_old))
        .child(ln_col(false, &display.new_rows, view.applied_new))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        paint_bridges(window, &placed);
                        paint_omit_links(window, &omit_links);
                    });
                },
            )
            .absolute()
            .top(px(0.))
            .left(px(LN_COL))
            .w(px(BRIDGE_COL))
            .h_full(),
        )
}

fn ln_col(left: bool, rows: &[DisplayRow], scroll_top: f32) -> impl IntoElement {
    let id = if left { "ln-left" } else { "ln-right" };
    let rows = rows.to_vec();
    div()
        .id(id)
        .absolute()
        .top(px(-scroll_top))
        .w(px(LN_COL))
        .when(left, |col| col.left(px(0.)))
        .when(!left, |col| col.right(px(0.)))
        .font_family(theme::MONO_FONT)
        .text_size(px(10.))
        .text_color(theme::faint())
        .when(left, |col| col.text_right().pr_1())
        .when(!left, |col| col.pl_1())
        .children(rows.into_iter().enumerate().map(move |(i, row)| {
            let ln_id = if left { ("ln-l", i) } else { ("ln-r", i) };
            let label = match row.kind {
                RowKind::Omit { .. } => "⋯".to_string(),
                _ => row.ln.to_string(),
            };
            div()
                .id(ln_id)
                .h(theme::ROW_HEIGHT)
                .child(label)
        }))
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

fn row_at(rows: &[DisplayRow], scroll_top: f32, anchor: f32, row_h: f32) -> Option<usize> {
    if rows.is_empty() || row_h <= 0. {
        return None;
    }
    let i = ((scroll_top + anchor) / row_h).floor() as isize;
    if i < 0 {
        Some(0)
    } else if i as usize >= rows.len() {
        Some(rows.len() - 1)
    } else {
        Some(i as usize)
    }
}

fn nearest_hunk_index(s_rows: f32, lands: &[crate::domain::HunkLand]) -> Option<usize> {
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

fn scroll_s_for_target(
    target: HunkJumpTarget,
    old_rows: &[DisplayRow],
    new_rows: &[DisplayRow],
    knots: &[ScrollKnot],
    row_h: f32,
    current_s: f32,
) -> f32 {
    let rows = if target.side == Side::Old {
        old_rows
    } else {
        new_rows
    };
    let Some(i) = rows
        .iter()
        .position(|r| r.ln == target.ln && !matches!(r.kind, RowKind::Omit { .. }))
    else {
        return current_s;
    };
    s_from(
        target.side == Side::Old,
        i as f32 * row_h,
        knots,
        row_h,
        current_s,
    )
}

fn replace_side_marks(left: bool, display: &DisplayRows) -> Vec<Option<Vec<TokenPart>>> {
    let n = if left {
        display.old_rows.len()
    } else {
        display.new_rows.len()
    };
    let mut marks = vec![None; n];
    for bridge in &display.bridges {
        let Bridge::Replace {
            old_from,
            old_to,
            new_from,
            new_to,
            ..
        } = *bridge
        else {
            continue;
        };
        let olds: Vec<&str> = display.old_rows[old_from as usize..old_to as usize]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        let news: Vec<&str> = display.new_rows[new_from as usize..new_to as usize]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        let (old_m, new_m) = replace_marks(&olds, &news);
        if left {
            for (i, parts) in old_m.into_iter().enumerate() {
                marks[old_from as usize + i] = Some(parts);
            }
        } else {
            for (i, parts) in new_m.into_iter().enumerate() {
                marks[new_from as usize + i] = Some(parts);
            }
        }
    }
    marks
}

fn render_row_text(text: String, parts: Option<Vec<TokenPart>>) -> impl IntoElement {
    match parts {
        Some(parts) if parts.iter().any(|p| p.changed) => div()
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .children(parts.into_iter().map(|p| {
                if p.changed {
                    div()
                        .bg(theme::mod_chg())
                        .rounded(px(2.))
                        .child(p.text)
                        .into_any_element()
                } else {
                    div().child(p.text).into_any_element()
                }
            }))
            .into_any_element(),
        _ => div().child(text).into_any_element(),
    }
}

fn kind_bg(kind: RowKind) -> gpui::Rgba {
    match kind {
        RowKind::Replace => theme::mod_bg(),
        RowKind::Insert => theme::add_bg(),
        RowKind::Delete => theme::del_bg(),
        RowKind::Equal | RowKind::Omit { .. } => theme::white(),
    }
}

/// Viewport Y of an empty side's only seam. A short opposite file cannot scroll
/// its last line down to the one-third anchor, so the seam sits on that line.
fn empty_seam(other_n: usize, view_h: f32, row_h: f32) -> f32 {
    let anchor = view_h / 3.;
    if other_n == 0 {
        return anchor;
    }
    ((other_n as f32 - 1.) * row_h).min(anchor)
}

fn content_pad(n: usize, view_h: f32, row_h: f32) -> f32 {
    if n == 0 || view_h < 1. {
        return 0.;
    }
    let anchor = view_h / 3.;
    (view_h - anchor - row_h).max(0.)
}

fn max_scroll(n: usize, anchor: f32, row_h: f32) -> f32 {
    if n == 0 {
        0.
    } else {
        ((n as f32 - 1.) * row_h - anchor).max(0.)
    }
}

fn track(content_y: f32, n: usize, anchor: f32, row_h: f32) -> f32 {
    (content_y - anchor).clamp(0., max_scroll(n, anchor, row_h))
}

fn content_from_scroll(scroll_top: f32, n: usize, anchor: f32, row_h: f32) -> f32 {
    if n == 0 || scroll_top <= 0. {
        return 0.;
    }
    let max = max_scroll(n, anchor, row_h);
    if scroll_top >= max - 0.5 {
        n as f32 * row_h
    } else {
        scroll_top + anchor
    }
}

fn interp(s: f32, knots: &[ScrollKnot], row_h: f32) -> (f32, f32) {
    let Some(first) = knots.first() else {
        return (0., 0.);
    };
    if s <= first.s as f32 * row_h {
        return (first.old_y as f32 * row_h, first.new_y as f32 * row_h);
    }
    let last = knots[knots.len() - 1];
    if s >= last.s as f32 * row_h {
        return (last.old_y as f32 * row_h, last.new_y as f32 * row_h);
    }
    for i in 1..knots.len() {
        let a = knots[i - 1];
        let b = knots[i];
        let a_s = a.s as f32 * row_h;
        let b_s = b.s as f32 * row_h;
        if s <= b_s {
            let t = if b_s == a_s { 0. } else { (s - a_s) / (b_s - a_s) };
            let old = (a.old_y as f32 + t * (b.old_y as f32 - a.old_y as f32)) * row_h;
            let new = (a.new_y as f32 + t * (b.new_y as f32 - a.new_y as f32)) * row_h;
            return (old, new);
        }
    }
    (last.old_y as f32 * row_h, last.new_y as f32 * row_h)
}

fn s_from(old_side: bool, content_px: f32, knots: &[ScrollKnot], row_h: f32, current_s: f32) -> f32 {
    let (cur_old, cur_new) = interp(current_s, knots, row_h);
    let cur = if old_side { cur_old } else { cur_new };
    if (cur - content_px).abs() < 0.5 {
        return current_s;
    }
    let content_rows = content_px / row_h;
    for i in 1..knots.len() {
        let a = knots[i - 1];
        let b = knots[i];
        let (a_y, b_y) = if old_side {
            (a.old_y, b.old_y)
        } else {
            (a.new_y, b.new_y)
        };
        if b_y > a_y {
            let a_yf = a_y as f32;
            let b_yf = b_y as f32;
            if content_rows >= a_yf && content_rows <= b_yf {
                let t = (content_rows - a_yf) / (b_yf - a_yf);
                return (a.s as f32 + t * (b.s as f32 - a.s as f32)) * row_h;
            }
        }
    }
    let end = knots[knots.len() - 1];
    let end_y = if old_side { end.old_y } else { end.new_y };
    if content_rows >= end_y as f32 {
        end.s as f32 * row_h
    } else {
        0.
    }
}

fn gap_intervals(
    old_pane: bool,
    bridges: &[Bridge],
    old_n: usize,
    new_n: usize,
    old_scroll: f32,
    new_scroll: f32,
    view_h: f32,
    row_h: f32,
) -> Vec<(f32, f32)> {
    let n = if old_pane { old_n } else { new_n };
    if n == 0 && view_h > 0. {
        return vec![(0., view_h)];
    }
    let mut out = Vec::new();
    for bridge in bridges {
        let (y0, y1, cover_scroll, cover_n) = match bridge {
            Bridge::Insert { new_from, new_to, .. } if old_pane => (
                *new_from as f32 * row_h - new_scroll,
                *new_to as f32 * row_h - new_scroll,
                old_scroll,
                old_n,
            ),
            Bridge::Delete { old_from, old_to, .. } if !old_pane => (
                *old_from as f32 * row_h - old_scroll,
                *old_to as f32 * row_h - old_scroll,
                new_scroll,
                new_n,
            ),
            _ => continue,
        };
        let covered = (-cover_scroll, cover_n as f32 * row_h - cover_scroll);
        for (a, b) in subtract_span((y0, y1), covered) {
            let top = a.max(0.);
            let bot = b.min(view_h);
            if bot - top > 0.5 {
                out.push((top, bot));
            }
        }
    }
    out
}

fn subtract_span(span: (f32, f32), cover: (f32, f32)) -> Vec<(f32, f32)> {
    let (a, b) = span;
    let (c, d) = cover;
    if b <= a {
        return Vec::new();
    }
    if d <= c || b <= c || a >= d {
        return vec![(a, b)];
    }
    let mut parts = Vec::new();
    if a < c {
        parts.push((a, c.min(b)));
    }
    if b > d {
        parts.push((d.max(a), b));
    }
    parts
}

fn place_bridge(
    bridge: &Bridge,
    old_n: usize,
    new_n: usize,
    old_scroll: f32,
    new_scroll: f32,
    old_pane: f32,
    new_pane: f32,
    view_h: f32,
    row_h: f32,
    x_l: f32,
    x_r: f32,
) -> Option<PlacedBridge> {
    let content_y = |rows: f32, scroll: f32, pane: f32| pane + rows * row_h - scroll;
    let edge_y = |rows: f32, n: usize, other_n: usize, scroll: f32, pane: f32| {
        if n == 0 {
            pane + empty_seam(other_n, view_h, row_h)
        } else {
            content_y(rows, scroll, pane)
        }
    };
    let (kind, points) = match *bridge {
        Bridge::Insert {
            old_seam,
            new_from,
            new_to,
            ..
        } => {
            if new_to <= new_from {
                return None;
            }
            (
                RowKind::Insert,
                vec![
                    (
                        x_l,
                        edge_y(old_seam as f32, old_n, new_n, old_scroll, old_pane),
                    ),
                    (x_r, content_y(new_from as f32, new_scroll, new_pane)),
                    (x_r, content_y(new_to as f32, new_scroll, new_pane)),
                ],
            )
        }
        Bridge::Delete {
            old_from,
            old_to,
            new_seam,
            ..
        } => {
            if old_to <= old_from {
                return None;
            }
            (
                RowKind::Delete,
                vec![
                    (x_l, content_y(old_from as f32, old_scroll, old_pane)),
                    (
                        x_r,
                        edge_y(new_seam as f32, new_n, old_n, new_scroll, new_pane),
                    ),
                    (x_l, content_y(old_to as f32, old_scroll, old_pane)),
                ],
            )
        }
        Bridge::Replace {
            old_from,
            old_to,
            new_from,
            new_to,
            ..
        } => {
            if old_to <= old_from && new_to <= new_from {
                return None;
            }
            (
                RowKind::Replace,
                vec![
                    (
                        x_l,
                        edge_y(old_from as f32, old_n, new_n, old_scroll, old_pane),
                    ),
                    (
                        x_r,
                        edge_y(new_from as f32, new_n, old_n, new_scroll, new_pane),
                    ),
                    (
                        x_r,
                        edge_y(new_to as f32, new_n, old_n, new_scroll, new_pane),
                    ),
                    (
                        x_l,
                        edge_y(old_to as f32, old_n, new_n, old_scroll, old_pane),
                    ),
                ],
            )
        }
    };
    Some(PlacedBridge { kind, points })
}

fn paint_bridges(window: &mut Window, placed: &[PlacedBridge]) {
    for bridge in placed {
        if bridge.points.len() < 3 {
            continue;
        }
        let pts: Vec<_> = bridge
            .points
            .iter()
            .map(|(x, y)| point(px(*x), px(*y)))
            .collect();
        let mut path = PathBuilder::fill();
        path.add_polygon(&pts, true);
        if let Ok(path) = path.build() {
            window.paint_path(path, kind_bg(bridge.kind));
        }
    }
}

fn paint_omit_links(window: &mut Window, links: &[(f32, f32, f32, f32)]) {
    for &(x0, y0, x1, y1) in links {
        let mut stroke = PathBuilder::stroke(px(1.4));
        let mid = (x0 + x1) / 2.;
        stroke.move_to(point(px(x0), px(y0)));
        stroke.cubic_bezier_to(
            point(px(x1), px(y1)),
            point(px(mid), px(y0)),
            point(px(mid), px(y1)),
        );
        if let Ok(path) = stroke.build() {
            window.paint_path(path, rgb(0xb5b5b5));
        }
    }
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

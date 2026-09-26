use gpui::{
    App, ClipboardItem, Context, Div, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
    WindowControlArea, div, prelude::*, px, rgb, svg,
};
use std::collections::HashSet;
use std::rc::Rc;

use crate::domain::{ChangedPath, Comparison, DisplayRow, PathStatus, Review, RowKind, Side};
use crate::export;
use crate::git::{self, FileDiff};
use super::file_tree::{self, TreeRow};
#[cfg(target_os = "macos")]
use super::mac_column_vibrancy::ColumnVibrancy;
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
    #[cfg(target_os = "macos")]
    tree_vibrancy: Option<ColumnVibrancy>,
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
            #[cfg(target_os = "macos")]
            tree_vibrancy: None,
        }
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
    }

    fn select_path(&mut self, path: String) {
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
        let file = git::file_diff(&snap.comparison, &path, status, &Default::default());
        snap.selected_path = path;
        snap.file = file;
        self.drafting = None;
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

}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_collapsed_dirs();
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
            let sub = match &s.file {
                FileDiff::Text { hunk_count, .. } => {
                    format!(
                        "{hunk_count} difference{} · {n} comment{}",
                        if *hunk_count == 1 { "" } else { "s" },
                        if n == 1 { "" } else { "s" }
                    )
                }
                FileDiff::Binary => "binary file".into(),
                FileDiff::Error(e) => e.clone(),
            };
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
                        .child(subtitle),
                )
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
        Some(FileDiff::Text { rows, .. }) => {
            // One scroll for L+gutter+R → lockstep sync (ADR 0003 intent).
            div()
                .id("diff-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .child(code_pane(true, rows, view, cx))
                        .child(center_gutter(rows))
                        .child(code_pane(false, rows, view, cx)),
                )
                .into_any_element()
        }
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
    rows: &[DisplayRow],
    view: &DiffView,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let id = if left { "code-left" } else { "code-right" };
    let side = if left { Side::Old } else { Side::New };
    let rows = rows.to_vec();
    let drafting_line = view
        .drafting
        .as_ref()
        .and_then(|d| (d.side == side).then_some(d.line));

    div()
        .id(id)
        .flex_1()
        .min_w(px(0.))
        .font_family(theme::MONO_FONT)
        .text_xs()
        .children(rows.into_iter().enumerate().map(move |(i, row)| {
            let empty = if left {
                row.kind == RowKind::Insert || row.old_ln.is_none()
            } else {
                row.kind == RowKind::Delete || row.new_ln.is_none()
            };
            let text = if left {
                row.old_text.clone()
            } else {
                row.new_text.clone()
            };
            let ln = if left { row.old_ln } else { row.new_ln };
            let marked = ln.is_some_and(|n| view.has_comment(side, n));
            let drafting_here = ln.is_some_and(|n| drafting_line == Some(n));
            let bg = if drafting_here {
                rgb(0xdbe4ff)
            } else if empty {
                theme::gap_bg()
            } else {
                match row.kind {
                    RowKind::Replace => theme::mod_bg(),
                    RowKind::Insert => theme::add_bg(),
                    RowKind::Delete => theme::del_bg(),
                    RowKind::Equal => theme::white(),
                }
            };
            let row_id = if left {
                ("row-l", i)
            } else {
                ("row-r", i)
            };
            let mut el = div()
                .id(row_id)
                .h(theme::ROW_HEIGHT)
                .px_3()
                .bg(bg)
                .text_color(if empty {
                    theme::faint()
                } else {
                    theme::text()
                })
                .overflow_hidden()
                .when(marked, |d| d.border_l_2().border_color(theme::accent()))
                .child(if empty {
                    // Subtle hatch stand-in
                    "╱".to_string()
                } else {
                    text
                });

            if let Some(line) = ln.filter(|_| !empty) {
                el = el.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| {
                    this.begin_draft(side, line, window, cx);
                }));
            }
            el
        }))
}

fn center_gutter(rows: &[DisplayRow]) -> impl IntoElement {
    let rows_l = rows.to_vec();
    let rows_r = rows.to_vec();
    let ribbons = ribbon_marks(rows);
    div()
        .id("gutter")
        .w(theme::GUTTER_WIDTH)
        .flex_none()
        .flex()
        .border_x_1()
        .border_color(theme::line())
        .bg(rgb(0xf7f8fb))
        .child(ln_col(true, rows_l))
        .child(ribbon_col(ribbons))
        .child(ln_col(false, rows_r))
}

fn ribbon_marks(rows: &[DisplayRow]) -> Vec<&'static str> {
    let mut out = Vec::with_capacity(rows.len());
    let mut i = 0;
    while i < rows.len() {
        let kind = rows[i].kind;
        if kind == RowKind::Equal {
            out.push("");
            i += 1;
            continue;
        }
        let start = i;
        while i < rows.len() && rows[i].kind == kind {
            i += 1;
        }
        for j in start..i {
            out.push(if j == start { "»" } else { "│" });
        }
    }
    out
}

fn ribbon_col(marks: Vec<&'static str>) -> impl IntoElement {
    div()
        .id("ribbon")
        .w(px(12.))
        .flex_none()
        .font_family(theme::MONO_FONT)
        .text_xs()
        .text_color(theme::accent())
        .flex()
        .flex_col()
        .items_center()
        .children(marks.into_iter().enumerate().map(|(i, m)| {
            div()
                .id(("rib", i))
                .h(theme::ROW_HEIGHT)
                .child(m)
        }))
}

fn ln_col(left: bool, rows: Vec<DisplayRow>) -> impl IntoElement {
    let id = if left { "ln-left" } else { "ln-right" };
    div()
        .id(id)
        .flex_1()
        .font_family(theme::MONO_FONT)
        .text_xs()
        .text_color(theme::faint())
        .when(left, |d| d.text_right().pr_1())
        .when(!left, |d| d.pl_1())
        .children(rows.into_iter().enumerate().map(move |(i, row)| {
            let ln = if left { row.old_ln } else { row.new_ln };
            let bg = match row.kind {
                RowKind::Replace => theme::mod_bg(),
                RowKind::Insert if !left => theme::add_bg(),
                RowKind::Delete if left => theme::del_bg(),
                _ => rgb(0xf7f8fb),
            };
            let ln_id = if left {
                ("ln-l", i)
            } else {
                ("ln-r", i)
            };
            div()
                .id(ln_id)
                .h(theme::ROW_HEIGHT)
                .bg(bg)
                .child(ln.map(|n| n.to_string()).unwrap_or_default())
        }))
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

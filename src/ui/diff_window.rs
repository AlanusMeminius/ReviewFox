use gpui::{
    AnyElement, AnyView, App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Subscription, WeakEntity, Window,
    WindowControlArea, actions, canvas, div, prelude::*, px, rgb,
};
use std::collections::HashSet;
use std::rc::Rc;

use crate::domain::{
    Anchor, ChangedPath, Comparison, DiffFontSize, PathStatus, Review, SearchMatch, SearchScope,
    Side, ViewOptions, search_file,
};

use super::appearance::{self, UiTextSize};
use super::diff::pane::{self, DualPane, FontOp, PaneEvent, SlotBounds, placeholder};
use super::file_tree::{self, TreeRow};
use super::file_tree_rows;
use super::icon_button::IconButton;
#[cfg(target_os = "macos")]
use super::mac_column_vibrancy::ColumnVibrancy;
use super::scrollbar;
use super::splitter::{self, Axis, ResizeState};
use super::theme;
use super::tooltip::{self, Tooltip};
use super::window_geometry;
use crate::export;
use crate::git::{self, FileDiff};
use crate::window_geometry_store;

actions!(diff, [CloseDiff, DismissOrCloseDiff]);

/// Key context on the Diff window root.
const CONTEXT: &str = "Diff";

#[cfg(target_os = "macos")]
const CLOSE_KEY: &str = "cmd-w";
#[cfg(not(target_os = "macos"))]
const CLOSE_KEY: &str = "ctrl-w";

/// Diff-window bindings, scoped to [`CONTEXT`].
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("escape", DismissOrCloseDiff, Some(CONTEXT)),
        KeyBinding::new(CLOSE_KEY, CloseDiff, Some(CONTEXT)),
    ]
}

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

/// The Diff window shell: tree, chrome, search bar, comments, draft bar and
/// Review. The dual pane is its own Entity (`DualPane`), driven by methods
/// and heard through `PaneEvent`s, so scrolling notifies only the pane.
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
    pane: Entity<DualPane>,
    _pane_events: Subscription,
    /// Cached view that renders the shell; created on the first render.
    shell: Option<Entity<DiffShell>>,
    /// Where the shell leaves room for the pane this frame.
    pane_bounds: SlotBounds,
    /// Last `PaneEvent::HunkIndexChanged`; drives chrome.
    hunk_index: Option<usize>,
    /// Last `PaneEvent::HoverCopy`; shown in chrome.
    hover_copy: Option<String>,
    #[cfg(target_os = "macos")]
    window_vibrancy: Option<ColumnVibrancy>,
    /// Diff-computation knobs; does not change Comparison identity.
    view_options: ViewOptions,
    /// §3.1.2; persisted in `settings.json`.
    sync_horizontal_scroll: bool,
    /// §3.1.1; persisted in `settings.json`.
    soft_wrap: bool,
    /// In-file search query; empty = no hits.
    search_query: String,
    search_scope: SearchScope,
    /// When true, keystrokes edit `search_query` (like the draft bar).
    searching: bool,
    bounds_sub: Option<gpui::Subscription>,
}

/// Renders DiffView's shell as a cached view. A pane scroll dirties DiffView
/// (the pane's ancestor), whose render is then only the root, this cached
/// shell and the pane slot. The shell re-renders when DiffView notifies.
struct DiffShell {
    view: WeakEntity<DiffView>,
    _observe: Subscription,
}

impl Render for DiffShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.view
            .update(cx, |view, cx| view.render_shell(window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl DiffView {
    pub fn with_snapshot(
        snapshot: DiffSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let review = Review::new(snapshot.comparison.clone());
        let pane = cx.new(DualPane::new);
        let pane_events = cx.subscribe_in(&pane, window, |this, _, event, window, cx| match event {
            PaneEvent::BeginDraft { side, ln } => this.begin_draft(*side, *ln, window, cx),
            PaneEvent::HunkIndexChanged(index) => {
                this.hunk_index = *index;
                cx.notify();
            }
            PaneEvent::HoverCopy(copy) => {
                this.hover_copy = copy.clone();
                cx.notify();
            }
        });
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
            pane,
            _pane_events: pane_events,
            shell: None,
            pane_bounds: SlotBounds::default(),
            hunk_index: None,
            hover_copy: None,
            #[cfg(target_os = "macos")]
            window_vibrancy: None,
            view_options: ViewOptions::default(),
            sync_horizontal_scroll: crate::settings_store::sync_horizontal_scroll(
                &crate::settings_store::load_file(),
            ),
            soft_wrap: crate::settings_store::soft_wrap(&crate::settings_store::load_file()),
            search_query: String::new(),
            search_scope: SearchScope::Both,
            searching: false,
            bounds_sub: None,
        };
        this.with_pane(cx, |pane, cx| {
            pane.set_sync_horizontal(this.sync_horizontal_scroll, cx);
            pane.set_soft_wrap(this.soft_wrap, cx);
        });
        this.open_in_pane(cx);
        this
    }

    fn with_pane<R>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut DualPane, &mut Context<DualPane>) -> R,
    ) -> R {
        self.pane.update(cx, f)
    }

    /// Anchors of the selected path's DraftComments, for the pane's comment index.
    fn path_anchors(&self) -> Vec<Anchor> {
        match (&self.review, &self.snapshot) {
            (Some(review), Some(snap)) => review
                .comments_for_path(&snap.selected_path)
                .map(|c| c.anchor.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Hand the selected file to the pane: file start, everything folded.
    fn open_in_pane(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = self.snapshot.as_ref() else {
            return;
        };
        let file = snap.file.clone();
        let path = snap.selected_path.clone();
        let comments = self.path_anchors();
        self.with_pane(cx, |pane, cx| pane.open(&path, &file, comments, cx));
    }

    fn refresh_comments(&mut self, cx: &mut Context<Self>) {
        let comments = self.path_anchors();
        self.with_pane(cx, |pane, cx| pane.set_comments(comments, cx));
    }

    fn set_drafting(&mut self, drafting: Option<Drafting>, cx: &mut Context<Self>) {
        let line = drafting.as_ref().map(|d| (d.side, d.line));
        self.drafting = drafting;
        self.with_pane(cx, |pane, cx| pane.set_drafting(line, cx));
    }

    /// Recompute Alignment under the current ViewOptions.
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
    }

    fn toggle_ignore_whitespace(&mut self, cx: &mut Context<Self>) {
        self.view_options.ignore_whitespace = !self.view_options.ignore_whitespace;
        self.recompute_alignment();
        if let Some(FileDiff::Text { alignment, .. }) = self.snapshot.as_ref().map(|s| &s.file) {
            let alignment = alignment.clone();
            self.with_pane(cx, |pane, cx| pane.set_alignment(alignment, cx));
        }
        cx.notify();
    }

    fn toggle_sync_horizontal_scroll(&mut self, cx: &mut Context<Self>) {
        self.sync_horizontal_scroll = !self.sync_horizontal_scroll;
        let mut file = crate::settings_store::load_file();
        file.sync_horizontal_scroll = Some(self.sync_horizontal_scroll);
        crate::settings_store::save_file(&file).ok();
        self.with_pane(cx, |pane, cx| {
            pane.set_sync_horizontal(self.sync_horizontal_scroll, cx);
        });
        cx.notify();
    }

    fn toggle_soft_wrap(&mut self, cx: &mut Context<Self>) {
        self.soft_wrap = !self.soft_wrap;
        let mut file = crate::settings_store::load_file();
        file.soft_wrap = Some(self.soft_wrap);
        crate::settings_store::save_file(&file).ok();
        self.with_pane(cx, |pane, cx| pane.set_soft_wrap(self.soft_wrap, cx));
        cx.notify();
    }

    fn sync_search_to_pane(&mut self, cx: &mut Context<Self>) {
        let q = SharedString::from(self.search_query.clone());
        self.with_pane(cx, |pane, cx| pane.set_search_query(q, cx));
    }

    fn jump_hunk(&mut self, dir: i32, cx: &mut Context<Self>) {
        self.with_pane(cx, |pane, cx| pane.jump_hunk(dir, cx));
    }

    /// Selected path's position in tree order, as `(index, total)`.
    fn file_position(&self) -> (usize, usize) {
        let Some(snap) = &self.snapshot else {
            return (0, 0);
        };
        let order = file_tree::file_order(&snap.changed_paths);
        let index = order
            .iter()
            .position(|p| *p == snap.selected_path)
            .unwrap_or(0);
        (index, order.len())
    }

    /// Steps to the neighbouring file in tree order; stops at either end.
    fn jump_file(&mut self, dir: i32, cx: &mut Context<Self>) {
        let Some(snap) = &self.snapshot else {
            return;
        };
        let order = file_tree::file_order(&snap.changed_paths);
        let (index, _) = self.file_position();
        let target = if dir < 0 {
            index.checked_sub(1)
        } else {
            Some(index + 1)
        };
        if let Some(path) = target.and_then(|i| order.get(i).cloned()) {
            self.select_path(path, cx);
            cx.notify();
        }
    }

    fn jump_match(&mut self, side: Side, ln: u32, cx: &mut Context<Self>) {
        self.with_pane(cx, |pane, cx| pane.jump_match(side, ln, cx));
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
    pub fn apply_snapshot(&mut self, incoming: DiffSnapshot, cx: &mut Context<Self>) {
        match &mut self.snapshot {
            Some(current) if current.comparison == incoming.comparison => {
                current.selected_path = incoming.selected_path;
                current.changed_paths = incoming.changed_paths;
                current.file = incoming.file;
            }
            _ => {
                self.review = Some(Review::new(incoming.comparison.clone()));
                self.set_drafting(None, cx);
                self.export_status = None;
                self.snapshot = Some(incoming);
            }
        }
        self.recompute_alignment();
        self.open_in_pane(cx);
    }

    fn select_path(&mut self, path: String, cx: &mut Context<Self>) {
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
        self.set_drafting(None, cx);
        self.open_in_pane(cx);
        window_geometry_store::note_diff_selected_path(path);
        window_geometry_store::flush();
    }

    fn begin_draft(&mut self, side: Side, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.set_drafting(
            Some(Drafting {
                side,
                line,
                body: String::new(),
            }),
            cx,
        );
        window.focus(&self.focus);
        cx.notify();
    }

    fn commit_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.drafting.take() else {
            return;
        };
        self.set_drafting(None, cx);
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
        self.refresh_comments(cx);
        cx.notify();
    }

    /// Cmd/Ctrl+W: always close the Diff window.
    fn close_diff(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    /// Esc: dismiss Find → cancel draft → else close window.
    fn dismiss_or_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching {
            self.searching = false;
            cx.notify();
            return;
        }
        if self.drafting.is_some() {
            self.set_drafting(None, cx);
            cx.notify();
            return;
        }
        window.remove_window();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let mods = &event.keystroke.modifiers;
        if mods.secondary() && !mods.alt && !mods.shift {
            let op = match event.keystroke.key.as_str() {
                "=" | "+" => Some(FontOp::Inc),
                "-" => Some(FontOp::Dec),
                "0" => Some(FontOp::Reset),
                _ => None,
            };
            if let Some(op) = op {
                self.font_size(op, cx);
                return;
            }
        }
        if self.searching {
            match event.keystroke.key.as_str() {
                "enter" => {
                    if let Some(m) = self.current_matches().into_iter().next() {
                        self.jump_match(m.side, m.ln, cx);
                    }
                }
                "backspace" => {
                    self.search_query.pop();
                    self.sync_search_to_pane(cx);
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
                        self.sync_search_to_pane(cx);
                        cx.notify();
                    }
                }
            }
            return;
        }
        if self.drafting.is_none() {
            match event.keystroke.key.as_str() {
                "}" => self.jump_file(1, cx),
                "{" => self.jump_file(-1, cx),
                "]" if mods.shift => self.jump_file(1, cx),
                "[" if mods.shift => self.jump_file(-1, cx),
                "]" => self.jump_hunk(1, cx),
                "[" => self.jump_hunk(-1, cx),
                "/" => {
                    // Focus is already on DiffView; open search mode.
                    self.searching = true;
                    self.set_drafting(None, cx);
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => self.commit_draft(cx),
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

    fn font_size(&mut self, op: FontOp, cx: &mut Context<Self>) {
        self.with_pane(cx, |pane, cx| pane.set_font_size(op, cx));
        // The toolbar shows the size, so it re-renders with the pane.
        cx.notify();
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

    /// Everything but the pane, rendered by the cached `DiffShell`.
    fn render_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.sync_collapsed_dirs();
        // Cleared here, set by the body's canvas if this file shows the pane.
        self.pane_bounds.set(None);
        let tree_w = if self.tree_collapsed {
            px(0.)
        } else {
            px(self.tree_width)
        };
        let show_tree_split = !self.tree_collapsed;

        div()
            .id("diff-shell")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            // The only band that reaches both window edges, so it can own the whole
            // drag surface and seat the caption buttons in the corner.
            .child(render_titlebar(self, window, cx))
            .child(
                div()
                    .id("diff-body")
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .overflow_hidden()
                    .child(render_tree_pane(self, tree_w, cx))
                    .when(show_tree_split, |d| {
                        d.child(splitter::handle(
                            "diff-tree-resize-handle",
                            Axis::HorizontalLeading,
                            self.tree_resize_handler(cx),
                            self.tree_resize_state.clone(),
                        ))
                    })
                    // Frosted desk: the content island floats here, inset on all four
                    // sides so the material reads around it.
                    .child(
                        div()
                            .id("diff-stage")
                            .relative()
                            .h_full()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .bg(theme::sidebar())
                            .flex()
                            .flex_col()
                            .pt(px(theme::CHANGES_TOP_INSET))
                            .pl(px(theme::CHANGES_INSET))
                            .pr(px(theme::CHANGES_INSET))
                            .child(render_dual_pane(self, cx))
                            .child(render_status_bar(self, cx)),
                    ),
            )
            .into_any_element()
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
                // Outside the debounced flush: the maximize glyph must flip on every bounds change.
                cx.notify();
                window_geometry::debounce_flush(cx);
            }));
        }
        let shell = self
            .shell
            .get_or_insert_with(|| {
                let view = cx.entity();
                cx.new(|cx| DiffShell {
                    view: view.downgrade(),
                    _observe: cx.observe(&view, |_, _, cx| cx.notify()),
                })
            })
            .clone();

        #[cfg(target_os = "macos")]
        {
            // Full window, not a tree-column strip: the content island is inset on
            // every side, so without material under the gutters they would be holes
            // straight through to the desktop.
            ColumnVibrancy::ensure_synced_window(&mut self.window_vibrancy, window);
        }

        div()
            .id("diff")
            .relative()
            .size_full()
            .overflow_hidden()
            // The window's one translucent layer; the tree column and the stage stay
            // clear so it is never painted twice.
            .bg(theme::frost())
            .font_family(appearance::ui_font(cx))
            // Unsized UI text inherits gpui's 1rem default (16px), scaled like the rest.
            .ui_text_size(16., cx)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &CloseDiff, window, cx| {
                this.close_diff(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DismissOrCloseDiff, window, cx| {
                this.dismiss_or_close(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.handle_key(event, cx);
            }))
            .child(AnyView::from(shell).cached(StyleRefinement::default().size_full()))
            .child(pane::slot(&self.pane, self.pane_bounds.clone()))
    }
}

/// Width the leading zone reserves when the tree is collapsed. Same rule as the
/// main window's titlebar, so the island's left edge stays under the titlebar's
/// content instead of drifting.
fn collapsed_leading_width() -> f32 {
    let controls = 12. + f32::from(theme::TOGGLE_SIZE) + theme::CHROME_GAP;
    #[cfg(target_os = "macos")]
    let controls = controls + theme::TRAFFIC_LIGHTS_WIDTH;
    controls
}

/// The selected file's path and where the reader is in it, as `(path, subtitle)`.
fn file_status(view: &DiffView, cx: &mut Context<DiffView>) -> (String, String) {
    match &view.snapshot {
        Some(s) => {
            let n = view
                .review
                .as_ref()
                .map(|r| r.comments_for_path(&s.selected_path).count())
                .unwrap_or(0);
            let mut sub = match &s.file {
                FileDiff::Text { .. } => {
                    let hunk_count = view.pane.read(cx).hunk_count().unwrap_or(0);
                    let hunk_part = if hunk_count == 0 {
                        "0 differences".into()
                    } else {
                        let n = view.hunk_index.unwrap_or(0) + 1;
                        format!("hunk {n} of {hunk_count}")
                    };
                    format!("{hunk_part} · {n} comment{}", if n == 1 { "" } else { "s" })
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
    }
}

/// Rides on the stage under the island: the file being read and where in it.
fn render_status_bar(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let (path, subtitle) = file_status(view, cx);
    div()
        .id("diff-status-bar")
        .flex_none()
        .h(px(STATUS_BAR_HEIGHT))
        // Text starts where the island's content does, one radius in.
        .px(px(theme::CHANGES_RADIUS))
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(theme::muted())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .font_family(appearance::code_font(cx))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(path),
        )
        .child(div().flex_none().whitespace_nowrap().child(subtitle))
}

/// The status bar's band below the island. It replaces the stage's bottom inset,
/// so the text sits centred between the island and the window edge.
const STATUS_BAR_HEIGHT: f32 = 28.;

fn render_titlebar(
    view: &DiffView,
    window: &Window,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    // Leading zone spans exactly what sits left of the stage, so what follows starts
    // on the stage's left edge and lines up with the island below.
    let leading_w = if view.tree_collapsed {
        px(collapsed_leading_width())
    } else {
        px(view.tree_width + splitter::HANDLE_WIDTH)
    };
    div()
        .id("diff-titlebar")
        .h(theme::TITLEBAR_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .child(
            div()
                .id("diff-titlebar-leading")
                .w(leading_w)
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                // Centering alone puts the pill `CHANGES_TOP_INSET` closer to the
                // window edge than to the island: both gaps are (T-H)/2, but the
                // lower one also spans the inset. Padding by the inset makes the two
                // exactly equal, (T + inset - H) / 2, for any pill height and any
                // inset value. The caption buttons sit outside this zone so they
                // still reach the physical corner.
                .pt(px(theme::CHANGES_TOP_INSET))
                .gap(px(theme::CHROME_GAP))
                .pl(px(12.))
                .overflow_hidden()
                .children(traffic_lights_space())
                .child(toggle_button("diff-tree-toggle", view.tree_collapsed, cx))
                .child(
                    div()
                        .id("diff-titlebar-drag-leading")
                        .h_full()
                        .flex_1()
                        .min_w(px(0.))
                        .window_control_area(WindowControlArea::Drag)
                        .occlude(),
                ),
        )
        .child(
            div()
                .id("diff-titlebar-main")
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .items_center()
                // Balances the toolbar against the island below; see the note in the
                // main window's `titlebar-leading`.
                .pt(px(theme::CHANGES_TOP_INSET))
                .gap_2()
                // Same inset the island uses, measured from the stage's left edge.
                .pl(px(theme::CHANGES_INSET))
                .child(render_nav_capsule(view, cx))
                .child(
                    capsule()
                        .child(nav_button(
                            "expand-all",
                            "unfold_vertical.svg",
                            "Expand All",
                            None,
                            true,
                            false,
                            cx.listener(|this, _, _, cx| {
                                this.with_pane(cx, |pane, cx| pane.expand_all(cx));
                            }),
                        ))
                        .child(nav_button(
                            "collapse-eq",
                            "fold_vertical.svg",
                            "Collapse Unchanged",
                            None,
                            true,
                            false,
                            cx.listener(|this, _, _, cx| {
                                this.with_pane(cx, |pane, cx| pane.collapse_unchanged(cx));
                            }),
                        ))
                        .child(nav_button(
                            "ignore-ws",
                            "pilcrow.svg",
                            "Ignore Whitespace",
                            None,
                            true,
                            view.view_options.ignore_whitespace,
                            cx.listener(|this, _, _, cx| this.toggle_ignore_whitespace(cx)),
                        ))
                        .child(nav_button(
                            "sync-h-scroll",
                            "chevrons_left.svg",
                            "Sync Horizontal Scroll",
                            None,
                            true,
                            view.sync_horizontal_scroll,
                            cx.listener(|this, _, _, cx| this.toggle_sync_horizontal_scroll(cx)),
                        ))
                        .child(nav_button(
                            "soft-wrap",
                            "chevrons_up_down.svg",
                            "Soft Wrap",
                            None,
                            true,
                            view.soft_wrap,
                            cx.listener(|this, _, _, cx| this.toggle_soft_wrap(cx)),
                        )),
                )
                .child(font_size_group(view.pane.read(cx).font_px(), cx))
                .child(capsule().child(nav_button(
                    "find",
                    "search.svg",
                    "Find",
                    Some("/".into()),
                    true,
                    view.searching,
                    cx.listener(|this, _, _, cx| {
                        if this.searching {
                            this.searching = false;
                            cx.notify();
                        } else {
                            this.searching = true;
                            this.set_drafting(None, cx);
                            cx.notify();
                        }
                    }),
                )))
                .child(capsule().child(nav_button(
                    "export",
                    "export.svg",
                    "Copy Review to Clipboard",
                    None,
                    true,
                    false,
                    cx.listener(|this, _, _, cx| this.export_to_clipboard(cx)),
                )))
                .children(view.export_status.as_ref().map(|status| {
                    div()
                        .flex_none()
                        .ui_text_size(12., cx)
                        .text_color(theme::accent())
                        .child(status.clone())
                }))
                .child(
                    div()
                        .id("diff-titlebar-drag")
                        .h_full()
                        .flex_1()
                        .min_w(px(0.))
                        .window_control_area(WindowControlArea::Drag)
                        .occlude(),
                )
                .child(
                    // Doubles as the trailing inset when no caption buttons follow.
                    div()
                        .id("diff-titlebar-drag-trailing")
                        .h_full()
                        .w(px(theme::CHANGES_INSET))
                        .flex_none()
                        .window_control_area(WindowControlArea::Drag)
                        .occlude(),
                ),
        )
        // Outside the zones' padding: close must land in the physical corner.
        .children(super::window_controls::window_controls(window))
}

fn render_tree_pane(
    view: &DiffView,
    width: gpui::Pixels,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let mono = appearance::code_font(cx);
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
        // A clear column straight on the frosted desk, not an island - same role as
        // the main window's workspace sidebar. Its chrome lives in `#diff-titlebar`.
        .bg(theme::sidebar())
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
                        file_tree_rows::dir_row(("ddir", i), depth, name, collapsed, cx)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.collapsed_dirs.remove(&toggle_path) {
                                    this.collapsed_dirs.insert(toggle_path.clone());
                                }
                                cx.notify();
                            }))
                    }
                    TreeRow::File { depth, path } => {
                        let path_click = path.path.clone();
                        let active = selected == path.path;
                        file_tree_rows::file_row(
                            ("dfile", i),
                            depth,
                            &path,
                            active,
                            mono.clone(),
                            cx,
                        )
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_path(path_click.clone(), cx);
                            cx.notify();
                        }))
                    }
                })),
            sb,
            )
        })
}

/// Whether the search bar renders above the pane inside the island. Mirrors the
/// early return in [`render_search_bar`].
fn has_search(view: &DiffView) -> bool {
    view.searching || !view.search_query.is_empty()
}

/// Whether anything renders below the pane inside the island. The pane cannot
/// round its own corners (see the note on the island's bottom inset), so this is
/// what decides who owns the island's bottom edge.
fn has_footer(view: &DiffView) -> bool {
    if view.drafting.is_some() {
        return true;
    }
    let path = view
        .snapshot
        .as_ref()
        .map(|s| s.selected_path.clone())
        .unwrap_or_default();
    view.review
        .as_ref()
        .is_some_and(|r| r.comments_for_path(&path).next().is_some())
}

fn render_dual_pane(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    div()
        .id("diff-main")
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(theme::white())
        .rounded(px(theme::CHANGES_RADIUS))
        .overflow_hidden()
        // The pane paints at the window root (see `pane::slot`) and gpui clips
        // children to the parent rect, not to its radius, so nothing here can round
        // the pane against this card. Wherever the pane is the first or last child,
        // hold it one radius clear of that edge and let the card draw its own corner;
        // the search bar and the footer round themselves when they are present.
        .when(!has_search(view), |island| {
            island.pt(px(theme::CHANGES_RADIUS))
        })
        .when(!has_footer(view), |island| {
            island.pb(px(theme::CHANGES_RADIUS))
        })
        .child(render_search_bar(view, cx))
        .child(render_body(view, cx))
        .child(render_comments(view, cx))
        .child(render_draft_bar(view, appearance::code_font(cx), cx))
}

fn render_search_bar(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let mono = appearance::code_font(cx);
    if !view.searching && view.search_query.is_empty() {
        return div().into_any_element();
    }
    let matches = view.current_matches();
    let (scope_icon, scope_label) = match view.search_scope {
        SearchScope::Old => ("square_minus.svg", "Search Old Side"),
        SearchScope::New => ("square_plus.svg", "Search New Side"),
        SearchScope::Both => ("diff.svg", "Search Both Sides"),
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
        // Always the island's first child; match the card radius.
        .rounded_t(px(theme::CHANGES_RADIUS))
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
                        .ui_text_size(12., cx)
                        .text_color(theme::muted())
                        .child(hint.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .font_family(mono.clone())
                        .text_xs()
                        .text_color(theme::text())
                        .child(query_display),
                )
                .child(
                    IconButton::new("search-scope", scope_icon, scope_label)
                        .shortcut("Tab")
                        .pressed(true)
                        .on_click(cx.listener(|this, _, _, cx| this.cycle_search_scope(cx))),
                )
                .child(
                    div()
                        .ui_text_size(12., cx)
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
                            .ui_text_size(12., cx)
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

/// The pane's place in the shell. The pane itself is mounted over it by
/// `pane::slot`, so a pane frame never re-renders the shell.
fn render_body(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    match view.snapshot.as_ref().map(|s| &s.file) {
        Some(FileDiff::Text { .. }) => {
            let slot = view.pane_bounds.clone();
            div()
                .id("diff-pane-slot")
                .relative()
                .flex_1()
                .min_h(px(0.))
                .child(
                    canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .into_any_element()
        }
        Some(FileDiff::Binary) => placeholder("Binary file — no Alignment", cx),
        Some(FileDiff::Error(msg)) => placeholder(msg, cx),
        None => placeholder("Open Diff from the main window", cx),
    }
}

fn render_comments(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let mono = appearance::code_font(cx);
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
        // Last child unless a draft bar follows: match the island radius so the
        // card's bottom corners read round.
        .when(view.drafting.is_none(), |strip| {
            strip.rounded_b(px(theme::CHANGES_RADIUS))
        })
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
                .ui_text_size(12., cx)
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            div()
                                .font_family(mono.clone())
                                // Own size: the UI text around it scales, Code Font chrome does not.
                                .text_xs()
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

fn render_draft_bar(view: &DiffView, mono: SharedString, cx: &App) -> impl IntoElement {
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
        // Always the island's last child; match the card radius.
        .rounded_b(px(theme::CHANGES_RADIUS))
        .px_3()
        .py_2()
        .child(
            div()
                .ui_text_size(12., cx)
                .text_color(theme::muted())
                .child(hint),
        )
        .child(
            div()
                .mt_1()
                .font_family(mono)
                .text_sm()
                .text_color(theme::text())
                .child(format!("{}▌", draft.body)),
        )
        .into_any_element()
}

/// Inset between the nav capsule's border and its buttons.
const NAV_INSET: f32 = 2.;
const NAV_BUTTON_RADIUS: f32 = 5.;

/// `« ‹ File 3/12 · Hunk 2/5 › »`: outer chevrons step files, inner ones hunks.
/// Sized to `TOGGLE_SIZE` so it sits in the toolbar like any other button.
fn render_nav_capsule(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let (index, total) = view.file_position();
    let file = if total == 0 {
        "—".to_string()
    } else {
        format!("{}/{total}", index + 1)
    };
    let hunk_count = view.pane.read(cx).hunk_count().unwrap_or(0);
    let hunk = if hunk_count == 0 {
        "—".to_string()
    } else {
        format!("{}/{hunk_count}", view.hunk_index.unwrap_or(0) + 1)
    };
    capsule()
        .child(nav_button(
            "prev-file",
            "chevrons_left.svg",
            "Previous File",
            Some("{".into()),
            index > 0,
            false,
            cx.listener(|this, _, _, cx| this.jump_file(-1, cx)),
        ))
        .child(nav_button(
            "prev-hunk",
            "chevron_left.svg",
            "Previous Hunk",
            Some("[".into()),
            true,
            false,
            cx.listener(|this, _, _, cx| this.jump_hunk(-1, cx)),
        ))
        .child(
            div()
                .flex_none()
                .h_full()
                .px_2()
                // The line box reserves descender room, so figures and caps sit
                // high against the geometrically centred chevrons (measured ~1.5px).
                .relative()
                .top(px(1.5))
                .flex()
                .items_center()
                .gap_1()
                .ui_text_size(12., cx)
                .child(div().text_color(theme::faint()).child("File"))
                .child(div().text_color(theme::text()).child(file))
                .child(div().text_color(theme::faint()).child("·"))
                .child(div().text_color(theme::faint()).child("Hunk"))
                .child(div().text_color(theme::text()).child(hunk)),
        )
        .child(nav_button(
            "next-hunk",
            "chevron_right.svg",
            "Next Hunk",
            Some("]".into()),
            true,
            false,
            cx.listener(|this, _, _, cx| this.jump_hunk(1, cx)),
        ))
        .child(nav_button(
            "next-file",
            "chevrons_right.svg",
            "Next File",
            Some("}".into()),
            index + 1 < total,
            false,
            cx.listener(|this, _, _, cx| this.jump_file(1, cx)),
        ))
}

/// Bordered group of toolbar buttons, `TOGGLE_SIZE` tall like any other button.
fn capsule() -> Div {
    div()
        .flex_none()
        .h(theme::TOGGLE_SIZE)
        .flex()
        .items_center()
        .p(px(NAV_INSET))
        // Concentric with the buttons' hover: button radius + inset + border.
        .rounded(px(NAV_BUTTON_RADIUS + NAV_INSET + 1.))
        .border_1()
        .border_color(theme::line())
        .bg(theme::white())
}

/// Fills the capsule's height: `TOGGLE_SIZE` less border and inset on both sides.
/// Pressed buttons sit on the range tint with an accent glyph, like `IconButton`.
fn nav_button(
    id: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    shortcut: Option<SharedString>,
    enabled: bool,
    pressed: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let color = if !enabled {
        theme::faint()
    } else if pressed {
        theme::accent()
    } else {
        theme::muted()
    };
    div()
        .id(id)
        .h_full()
        .w(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(NAV_BUTTON_RADIUS))
        .when(pressed, |button| button.bg(theme::range()))
        .tooltip(Tooltip::text(tooltip, shortcut))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|button| button.bg(theme::hover()))
                .active(|button| button.bg(theme::element_active()))
                .on_click(on_click)
        })
        .child(
            gpui::svg()
                .size(theme::ICON_SIZE)
                .flex_none()
                .path(icon)
                .text_color(color),
        )
}

/// `− 13 +`, like a browser's zoom control: the number is the current size and
/// resets it, so no third glyph competes with the two signs.
fn font_size_group(font_px: u32, cx: &mut Context<DiffView>) -> impl IntoElement {
    capsule()
        .child(nav_button(
            "font-dec",
            "minus.svg",
            "Smaller Text",
            Some(tooltip::cmd("-")),
            font_px > DiffFontSize::MIN,
            false,
            cx.listener(|this, _, _, cx| this.font_size(FontOp::Dec, cx)),
        ))
        .child(
            div()
                .id("font-reset")
                .h_full()
                .min_w(px(24.))
                .px_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(NAV_BUTTON_RADIUS))
                .cursor_pointer()
                .text_xs()
                .text_color(if font_px == DiffFontSize::DEFAULT {
                    theme::muted()
                } else {
                    theme::accent()
                })
                .tooltip(Tooltip::text("Reset Text Size", Some(tooltip::cmd("0"))))
                .hover(|button| button.bg(theme::hover()))
                .active(|button| button.bg(theme::element_active()))
                .on_click(cx.listener(|this, _, _, cx| this.font_size(FontOp::Reset, cx)))
                .child(font_px.to_string()),
        )
        .child(nav_button(
            "font-inc",
            "plus.svg",
            "Larger Text",
            Some(tooltip::cmd("=")),
            font_px < DiffFontSize::MAX,
            false,
            cx.listener(|this, _, _, cx| this.font_size(FontOp::Inc, cx)),
        ))
}

fn toggle_button(
    id: &'static str,
    collapsed: bool,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let label = if collapsed {
        "Show Files"
    } else {
        "Hide Files"
    };
    IconButton::new(id, "sidebar_title.svg", label)
        .pressed(collapsed)
        .on_click(cx.listener(|this, _, _, cx| {
            this.tree_collapsed = !this.tree_collapsed;
            cx.notify();
        }))
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

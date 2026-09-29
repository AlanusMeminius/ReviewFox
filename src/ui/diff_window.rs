use gpui::{
    AnyElement, AnyView, App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Subscription, Task, Timer, WeakEntity,
    Window, WindowControlArea, actions, canvas, div, prelude::*, px,
};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::{
    Anchor, ChangedPath, Comparison, DiffFontSize, LineSpan, PathStatus, Review, SearchFileText,
    SearchFiles, SearchMatch, SearchSide, Side, ViewOptions, next_match_index, next_search_side,
    prev_match_index, search_file, search_files, toggle_search_files,
};

use super::appearance::{self, UiTextSize};
use super::diff::pane::{self, DualPane, FontOp, PaneComment, PaneEvent, SlotBounds, placeholder};
use super::file_tree::{self, TreeRow};
use super::file_tree_rows::{self, RowSurface};
use super::icon_button::IconButton;
#[cfg(target_os = "macos")]
use super::mac_column_vibrancy::ColumnVibrancy;
use super::scrollbar;
use super::splitter::{self, Axis, ResizeState};
use super::text_field::{TextField, TextFieldEvent, TextFieldStyle};
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

/// An open DraftComment: which LineSpan the bottom dock is writing to, and
/// whether it is a new one or an existing body being edited.
struct Drafting {
    side: Side,
    start: u32,
    count: u32,
    /// Id of the comment being reopened; `None` while creating one. Editing
    /// only ever rewrites the body, so the Anchor is not carried here.
    editing: Option<u64>,
}

impl Drafting {
    /// Inclusive last line of the span.
    fn end(&self) -> u32 {
        self.start + self.count.saturating_sub(1)
    }
}

/// A LineSpan as the dock and the comments list name it: `postimage L3` for one
/// line, `postimage L3–5` for a span.
fn span_label(side: Side, start: u32, count: u32) -> String {
    if count <= 1 {
        format!("{} L{start}", side.label())
    } else {
        format!("{} L{start}–{}", side.label(), start + count - 1)
    }
}

/// Inclusive `(side, start, end)` wash matches this LineSpan.
fn selection_matches_span(selection: Option<(Side, u32, u32)>, side: Side, span: LineSpan) -> bool {
    selection.is_some_and(|(sel_side, start, end)| {
        sel_side == side && start == span.start && end == span.start + span.count.saturating_sub(1)
    })
}

/// The Diff window shell: tree, chrome, search bar, comments, draft dock and
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
    /// Where the shell measures the floating find bar; painted after DualPane.
    find_bar_bounds: SlotBounds,
    /// Same, for the bottom draft dock.
    draft_dock_bounds: SlotBounds,
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
    /// In-file search query; empty = no hits. Mirrored from [`Self::search_field`].
    search_query: String,
    /// Side factor: Preimage / Postimage / Both. Default on open: Postimage.
    search_side: SearchSide,
    /// Files factor: current file or all changed paths. Default on open: File.
    search_files: SearchFiles,
    /// When true, the floating find bar is open and the field should hold focus.
    searching: bool,
    /// 0-based index into [`Self::current_matches`]; set when hits exist.
    search_match_index: Option<usize>,
    /// Pending query recompute (~150ms). Cancelled by Enter / prev / next / factor change.
    search_debounce: Option<Task<()>>,
    /// Cached old/new texts for All-files find, in tree order.
    all_search_texts: Option<Vec<(String, Arc<str>, Arc<str>)>>,
    /// Real IME-capable input (same control as Settings); drives `search_query`.
    search_field: Entity<TextField>,
    _search_subscriptions: Vec<Subscription>,
    /// The bottom dock's body field. Kept for the window's life so the draft
    /// dock can open on the very first icon click without re-creating it.
    draft_field: Entity<TextField>,
    _draft_subscription: Subscription,
    _draft_observe: Subscription,
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
        let pane_events =
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| match event {
                PaneEvent::OpenDraft { side, start, count } => {
                    this.begin_draft(*side, *start, *count, window, cx)
                }
                PaneEvent::OpenEdit { id } => this.begin_edit(*id, window, cx),
                PaneEvent::SelectionStarted => {
                    // New gutter drag already owns the wash — only close the dock.
                    if this.drafting.is_some() {
                        this.close_dock(window, cx);
                    }
                }
                PaneEvent::HunkIndexChanged(index) => {
                    this.hunk_index = *index;
                    cx.notify();
                }
                PaneEvent::HoverCopy(copy) => {
                    this.hover_copy = copy.clone();
                    cx.notify();
                }
            });
        let search_field = cx.new(|cx| {
            TextField::new("Search this file…", false, cx).with_style(TextFieldStyle::Search)
        });
        let search_subscriptions = vec![
            cx.observe(&search_field, |this, search, cx| {
                let q = search.read(cx).content().to_string();
                if q == this.search_query {
                    return;
                }
                this.search_query = q;
                this.search_match_index = None;
                this.schedule_search_recompute(cx);
            }),
            cx.subscribe(
                &search_field,
                |this, _, event: &TextFieldEvent, cx| match event {
                    TextFieldEvent::Confirm => this.jump_search(1, cx),
                },
            ),
        ];
        let draft_field = cx.new(|cx| {
            TextField::new("Write a comment…", false, cx).with_style(TextFieldStyle::Draft)
        });
        let draft_subscription = cx.subscribe_in(
            &draft_field,
            window,
            |this, _, event: &TextFieldEvent, window, cx| match event {
                TextFieldEvent::Confirm => this.commit_draft(window, cx),
            },
        );
        // Grow/shrink the bottom dock when Shift+Enter adds lines.
        let draft_observe = cx.observe(&draft_field, |_, _, cx| cx.notify());
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
            find_bar_bounds: SlotBounds::default(),
            draft_dock_bounds: SlotBounds::default(),
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
            search_side: SearchSide::Postimage,
            search_files: SearchFiles::File,
            searching: false,
            search_match_index: None,
            search_debounce: None,
            all_search_texts: None,
            search_field,
            _search_subscriptions: search_subscriptions,
            draft_field,
            _draft_subscription: draft_subscription,
            _draft_observe: draft_observe,
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

    /// The selected path's DraftComments, for the pane's comment index and its
    /// start-line icons.
    fn path_comments(&self) -> Vec<PaneComment> {
        match (&self.review, &self.snapshot) {
            (Some(review), Some(snap)) => review
                .comments_for_path(&snap.selected_path)
                .map(|c| PaneComment {
                    id: c.id,
                    anchor: c.anchor.clone(),
                })
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
        let comments = self.path_comments();
        self.with_pane(cx, |pane, cx| pane.open(&path, &file, comments, cx));
    }

    fn refresh_comments(&mut self, cx: &mut Context<Self>) {
        let comments = self.path_comments();
        self.with_pane(cx, |pane, cx| pane.set_comments(comments, cx));
    }

    fn set_drafting(&mut self, drafting: Option<Drafting>, cx: &mut Context<Self>) {
        let span = drafting.as_ref().map(|d| (d.side, d.start, d.end()));
        self.drafting = drafting;
        self.with_pane(cx, |pane, cx| pane.set_drafting(span, cx));
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
        let side = self.search_side;
        let active = self.active_search_hit();
        self.with_pane(cx, |pane, cx| {
            pane.set_search_query(q, cx);
            pane.set_search_side(side, cx);
            pane.set_active_search_match(active, cx);
        });
    }

    fn active_search_hit(&self) -> Option<pane::ActiveSearchMatch> {
        let matches = self.current_matches();
        let i = self.search_match_index.filter(|&i| i < matches.len())?;
        let m = &matches[i];
        // Only paint active identity when the hit is on the selected path.
        let selected = self.snapshot.as_ref().map(|s| s.selected_path.as_str());
        if let Some(path) = m.path.as_deref() {
            if selected != Some(path) {
                return None;
            }
        }
        Some(pane::ActiveSearchMatch {
            side: m.side,
            ln: m.ln,
            bytes: m.bytes.clone(),
        })
    }

    fn cancel_search_debounce(&mut self) {
        self.search_debounce = None;
    }

    /// Debounced recompute after query edits (~150ms).
    fn schedule_search_recompute(&mut self, cx: &mut Context<Self>) {
        self.cancel_search_debounce();
        // Highlight follows the query immediately; select/land waits for debounce.
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            view.update(cx, |this, cx| {
                this.sync_search_to_pane(cx);
                cx.notify();
            })
            .ok();
        });
        self.search_debounce = Some(cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(150)).await;
            this.update(cx, |this, cx| {
                this.search_debounce = None;
                this.apply_search_hits(true, cx);
            })
            .ok();
        }));
    }

    /// Recompute matches; when `land_first` and any hits, select first and center-scroll.
    fn apply_search_hits(&mut self, land_first: bool, cx: &mut Context<Self>) {
        if self.search_files == SearchFiles::All {
            self.ensure_all_search_texts();
        }
        let matches = self.current_matches();
        if matches.is_empty() {
            self.search_match_index = None;
            self.sync_search_to_pane(cx);
            cx.notify();
            return;
        }
        if land_first || self.search_match_index.is_none() {
            self.search_match_index = Some(0);
            let m = matches[0].clone();
            self.jump_to_search_match(m, cx);
        } else if let Some(i) = self.search_match_index {
            if i >= matches.len() {
                self.search_match_index = Some(0);
                let m = matches[0].clone();
                self.jump_to_search_match(m, cx);
            } else {
                self.sync_search_to_pane(cx);
            }
        }
        cx.notify();
    }

    fn ensure_all_search_texts(&mut self) {
        if self.all_search_texts.is_some() {
            return;
        }
        let Some(snap) = &self.snapshot else {
            self.all_search_texts = Some(Vec::new());
            return;
        };
        let opts = self.view_options.clone();
        let order = file_tree::file_order(&snap.changed_paths);
        let mut out = Vec::with_capacity(order.len());
        for path in order {
            let status = snap
                .changed_paths
                .iter()
                .find(|p| p.path == path)
                .map(|p| p.status)
                .unwrap_or(PathStatus::Modify);
            if let FileDiff::Text {
                old_text, new_text, ..
            } = git::file_diff(&snap.comparison, &path, status, &opts)
            {
                out.push((path, old_text, new_text));
            }
        }
        self.all_search_texts = Some(out);
    }

    fn invalidate_all_search_texts(&mut self) {
        self.all_search_texts = None;
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

    fn jump_to_search_match(&mut self, m: SearchMatch, cx: &mut Context<Self>) {
        if let Some(path) = m.path.clone() {
            let needs = self
                .snapshot
                .as_ref()
                .is_none_or(|s| s.selected_path != path);
            if needs {
                self.select_path(path, cx);
            }
        }
        let byte = m.bytes.start;
        let side = m.side;
        let ln = m.ln;
        let active = pane::ActiveSearchMatch {
            side,
            ln,
            bytes: m.bytes.clone(),
        };
        let q = SharedString::from(self.search_query.clone());
        let search_side = self.search_side;
        self.with_pane(cx, |pane, cx| {
            pane.set_search_query(q, cx);
            pane.set_search_side(search_side, cx);
            pane.set_active_search_match(Some(active), cx);
            pane.jump_match(side, ln, Some(byte), cx);
        });
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = false;
        self.search_query.clear();
        self.search_match_index = None;
        self.cancel_search_debounce();
        self.search_field
            .update(cx, |field, cx| field.set_content("", cx));
        self.sync_search_to_pane(cx);
        window.focus(&self.focus);
        cx.notify();
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = true;
        self.search_side = SearchSide::Postimage;
        self.search_files = SearchFiles::File;
        self.set_drafting(None, cx);
        let handle = self.search_field.read(cx).focus_handle(cx);
        window.focus(&handle);
        self.sync_search_to_pane(cx);
        cx.notify();
    }

    /// `dir > 0` next, `dir < 0` prev; wraps. Lands and updates `search_match_index`.
    fn jump_search(&mut self, dir: i32, cx: &mut Context<Self>) {
        self.cancel_search_debounce();
        if self.search_files == SearchFiles::All {
            self.ensure_all_search_texts();
        }
        let matches = self.current_matches();
        // Stale index after a mid-debounce query change — treat as no selection.
        if self.search_match_index.is_some_and(|i| i >= matches.len()) {
            self.search_match_index = None;
        }
        let next = if dir < 0 {
            prev_match_index(matches.len(), self.search_match_index)
        } else {
            next_match_index(matches.len(), self.search_match_index)
        };
        let Some(i) = next else {
            self.search_match_index = None;
            self.sync_search_to_pane(cx);
            cx.notify();
            return;
        };
        self.search_match_index = Some(i);
        if let Some(m) = matches.get(i).cloned() {
            self.jump_to_search_match(m, cx);
        }
        cx.notify();
    }

    fn cycle_search_side(&mut self, cx: &mut Context<Self>) {
        self.cancel_search_debounce();
        self.search_side = next_search_side(self.search_side);
        self.apply_search_hits(true, cx);
    }

    fn toggle_search_files_factor(&mut self, cx: &mut Context<Self>) {
        self.cancel_search_debounce();
        self.search_files = toggle_search_files(self.search_files);
        if self.search_files == SearchFiles::All {
            self.ensure_all_search_texts();
        }
        self.apply_search_hits(true, cx);
    }

    fn set_search_side(&mut self, side: SearchSide, cx: &mut Context<Self>) {
        if self.search_side == side {
            return;
        }
        self.cancel_search_debounce();
        self.search_side = side;
        self.apply_search_hits(true, cx);
    }

    fn set_search_files_factor(&mut self, files: SearchFiles, cx: &mut Context<Self>) {
        if self.search_files == files {
            return;
        }
        self.cancel_search_debounce();
        self.search_files = files;
        if self.search_files == SearchFiles::All {
            self.ensure_all_search_texts();
        }
        self.apply_search_hits(true, cx);
    }

    fn current_matches(&self) -> Vec<SearchMatch> {
        match self.search_files {
            SearchFiles::File => {
                let Some(FileDiff::Text {
                    old_text, new_text, ..
                }) = self.snapshot.as_ref().map(|s| &s.file)
                else {
                    return Vec::new();
                };
                search_file(old_text, new_text, &self.search_query, self.search_side)
            }
            SearchFiles::All => {
                let Some(files) = &self.all_search_texts else {
                    return Vec::new();
                };
                let inputs: Vec<SearchFileText<'_>> = files
                    .iter()
                    .map(|(path, old, new)| SearchFileText {
                        path: path.as_str(),
                        old_text: old.as_ref(),
                        new_text: new.as_ref(),
                    })
                    .collect();
                search_files(&inputs, &self.search_query, self.search_side)
            }
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
        self.invalidate_all_search_texts();
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

    /// Open the bottom dock on a LineSpan with `body` loaded. Find and draft are
    /// mutually exclusive, so an open find bar is dismissed first. The dock's
    /// span becomes the selection, so the icon that reopens it stays under the
    /// pointer after a save. Reveals the span start (expand fold / scroll).
    fn open_dock(
        &mut self,
        drafting: Drafting,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if has_search(self) {
            self.close_search(window, cx);
        }
        let (side, start, end) = (drafting.side, drafting.start, drafting.end());
        self.set_drafting(Some(drafting), cx);
        self.with_pane(cx, |pane, cx| {
            pane.select_span(side, start, end, cx);
            pane.reveal_line(side, start, cx);
        });
        self.draft_field
            .update(cx, |field, cx| field.set_content(body, cx));
        let handle = self.draft_field.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    /// Empty gutter icon: write a new DraftComment on the selected LineSpan.
    fn begin_draft(
        &mut self,
        side: Side,
        start: u32,
        count: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_dock(
            Drafting {
                side,
                start,
                count,
                editing: None,
            },
            String::new(),
            window,
            cx,
        );
    }

    /// Filled gutter icon or island Edit: reopen an existing DraftComment
    /// with its body loaded. Its Anchor is what the dock names, so the span is
    /// the comment's own rather than whatever was selected.
    fn begin_edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((side, span, body)) = self.comment_target(id) else {
            return;
        };
        self.open_dock(
            Drafting {
                side,
                start: span.start,
                count: span.count,
                editing: Some(id),
            },
            body,
            window,
            cx,
        );
    }

    /// Where a DraftComment is anchored and what it says. `None` for an unknown
    /// id or a File anchor, neither of which the line dock can write to.
    fn comment_target(&self, id: u64) -> Option<(Side, LineSpan, String)> {
        let c = self.review.as_ref()?.comments.iter().find(|c| c.id == id)?;
        match &c.anchor {
            Anchor::Line { side, span, .. } => Some((*side, *span, c.body.clone())),
            Anchor::File { .. } => None,
        }
    }

    /// Close the draft dock without touching the pane selection wash.
    fn close_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_drafting(None, cx);
        self.draft_field
            .update(cx, |field, cx| field.set_content("", cx));
        window.focus(&self.focus);
        cx.notify();
    }

    /// Esc / cancel button: drop the draft and clear the selection wash.
    fn cancel_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_dock(window, cx);
        self.with_pane(cx, |pane, cx| pane.clear_selection(cx));
    }

    /// Island row body: selection wash on that comment's span, dock stays closed.
    /// If a draft is open, cancel it first (clears wash), then select this row.
    /// Reveals the span start when off-screen or inside a collapsed Equal.
    fn select_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafting.is_some() {
            self.cancel_draft(window, cx);
        }
        let Some((side, span, _)) = self.comment_target(id) else {
            return;
        };
        let end = span.start + span.count.saturating_sub(1);
        self.with_pane(cx, |pane, cx| {
            pane.select_span(side, span.start, end, cx);
            pane.reveal_line(side, span.start, cx);
        });
        cx.notify();
    }

    /// Immediate delete. Closes the dock if it was editing this id; clears the
    /// wash if it pointed at this comment's span.
    fn delete_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.drafting.as_ref().and_then(|d| d.editing) == Some(id) {
            self.close_dock(window, cx);
        }
        let clear_wash = self.comment_target(id).is_some_and(|(side, span, _)| {
            selection_matches_span(self.pane.read(cx).selection(), side, span)
        });
        if clear_wash {
            self.with_pane(cx, |pane, cx| pane.clear_selection(cx));
        }
        if let Some(review) = &mut self.review {
            review.delete_comment(id);
        }
        self.refresh_comments(cx);
        cx.notify();
    }

    /// Enter in the dock: add the span as a DraftComment, or rewrite the body of
    /// the one being edited. An empty body closes without writing and clears the
    /// wash (same as cancel) so Enter is never a way to make a comment blank.
    /// A real save keeps the selection wash so the island row stays selected.
    fn commit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.drafting.take() else {
            return;
        };
        let body = self.draft_field.read(cx).content().trim().to_string();
        if body.is_empty() {
            self.cancel_draft(window, cx);
            return;
        }
        self.close_dock(window, cx);
        let path = self
            .snapshot
            .as_ref()
            .map(|s| s.selected_path.clone())
            .unwrap_or_default();
        if let Some(review) = &mut self.review {
            match draft.editing {
                Some(id) => {
                    review.update_comment_body(id, body);
                }
                None => {
                    review.add_line_span_comment(path, draft.side, draft.start, draft.count, body);
                }
            }
        }
        self.refresh_comments(cx);
        cx.notify();
    }

    /// Cmd/Ctrl+W: always close the Diff window.
    fn close_diff(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    /// Esc: Find → cancel draft (clears wash) → clear selection → close window.
    fn dismiss_or_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if has_search(self) {
            self.close_search(window, cx);
            return;
        }
        if self.drafting.is_some() {
            self.cancel_draft(window, cx);
            return;
        }
        if self.pane.read(cx).selection().is_some() {
            self.with_pane(cx, |pane, cx| pane.clear_selection(cx));
            cx.notify();
            return;
        }
        window.remove_window();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
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
            // Typing goes through TextField (IME). Tab / Shift-Tab / Shift+Enter stay here.
            match event.keystroke.key.as_str() {
                "enter" if mods.shift => self.jump_search(-1, cx),
                "tab" if mods.shift => self.toggle_search_files_factor(cx),
                "tab" => self.cycle_search_side(cx),
                _ => {}
            }
            return;
        }
        if self.drafting.is_some() {
            // The dock's TextField owns typing (IME included). Enter reaches it
            // as `Confirm`; Shift+Enter has no binding there, so it lands here.
            if event.keystroke.key == "enter" && mods.shift {
                self.draft_field.update(cx, |field, cx| {
                    field.insert("\n", window, cx);
                });
            }
            return;
        }
        match event.keystroke.key.as_str() {
            "}" => self.jump_file(1, cx),
            "{" => self.jump_file(-1, cx),
            "]" if mods.shift => self.jump_file(1, cx),
            "[" if mods.shift => self.jump_file(-1, cx),
            "]" => self.jump_hunk(1, cx),
            "[" => self.jump_hunk(-1, cx),
            "/" => self.open_search(window, cx),
            _ => {}
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
                            true,
                        ))
                    })
                    // Frosted desk: the content island floats here, inset on all four
                    // sides so the material reads around it. Comment island sits to
                    // the right of the diff island when the path has comments.
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
                            .child(
                                div()
                                    .id("diff-stage-islands")
                                    .flex_1()
                                    .min_h(px(0.))
                                    .flex()
                                    .gap(px(theme::COMMENT_ISLAND_GAP))
                                    .child(render_dual_pane(self, cx))
                                    .child(render_comment_island(self, cx)),
                            )
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

        // DualPane paints after the shell and would cover any in-shell shadow.
        // Measure the find slot in the shell; paint the chrome here, after the pane.
        let find_overlay = has_search(self).then(|| {
            pane::overlay_slot(
                self.find_bar_bounds.clone(),
                render_search_bar(self, window, cx),
            )
        });
        if find_overlay.is_none() {
            self.find_bar_bounds.set(None);
        }
        let draft_overlay = self.drafting.is_some().then(|| {
            pane::overlay_slot(
                self.draft_dock_bounds.clone(),
                render_draft_dock(self, window, cx),
            )
        });
        if draft_overlay.is_none() {
            self.draft_dock_bounds.set(None);
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key(event, window, cx);
            }))
            .child(AnyView::from(shell).cached(StyleRefinement::default().size_full()))
            .child(pane::slot(&self.pane, self.pane_bounds.clone()))
            .children(find_overlay)
            .children(draft_overlay)
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
        px(view.tree_width + splitter::RAIL_HANDLE_WIDTH)
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
                    has_search(view),
                    cx.listener(|this, _, window, cx| {
                        if has_search(this) {
                            this.close_search(window, cx);
                        } else {
                            this.open_search(window, cx);
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
                            file_tree_rows::dir_row(
                                ("ddir", i),
                                depth,
                                name,
                                collapsed,
                                RowSurface::Desk,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.collapsed_dirs.remove(&toggle_path) {
                                        this.collapsed_dirs.insert(toggle_path.clone());
                                    }
                                    cx.notify();
                                },
                            ))
                        }
                        TreeRow::File { depth, path } => {
                            let path_click = path.path.clone();
                            let active = selected == path.path;
                            file_tree_rows::file_row(
                                ("dfile", i),
                                depth,
                                &path,
                                active,
                                RowSurface::Desk,
                                mono.clone(),
                                cx,
                            )
                            .cursor_pointer()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.select_path(path_click.clone(), cx);
                                    cx.notify();
                                },
                            ))
                        }
                    })),
                sb,
            )
        })
}

/// Whether the floating find bar is visible over the pane.
fn has_search(view: &DiffView) -> bool {
    view.searching || !view.search_query.is_empty()
}

/// Whether anything renders below the pane inside the island. The pane cannot
/// round its own corners (see the note on the island's bottom inset), so this is
/// what decides who owns the island's bottom edge.

fn draft_line_count(view: &DiffView, cx: &App) -> usize {
    view.draft_field.read(cx).visual_line_count()
}

fn draft_dock_h_for(view: &DiffView, cx: &App) -> f32 {
    theme::draft_dock_height(draft_line_count(view, cx))
}

fn render_dual_pane(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
    let find_open = has_search(view);
    div()
        .id("diff-main")
        .relative()
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
        // hold it one radius clear of that edge and let the card draw its own corner.
        // Find is an absolute overlay (not in flex flow) so open/close never changes
        // island flow height; when open, body top pad clears the capsule instead.
        .when(!find_open, |island| island.pt(px(theme::CHANGES_RADIUS)))
        .pb(px(theme::CHANGES_RADIUS))
        .child(render_body(view, find_open, cx))
        // Measure only — chrome paints after DualPane (see DiffView::render).
        .child(render_find_measure(view))
        .child(render_draft_measure(view, cx))
}

/// Invisible slot where the floating find bar will be painted above DualPane.
fn render_find_measure(view: &DiffView) -> impl IntoElement {
    if !has_search(view) {
        return div().into_any_element();
    }
    let slot = view.find_bar_bounds.clone();
    div()
        .absolute()
        .top(px(theme::FIND_BAR_INSET))
        .left(px(theme::FIND_BAR_INSET))
        .right(px(theme::FIND_BAR_INSET))
        .h(px(theme::FIND_BAR_HEIGHT))
        .child(canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {}).size_full())
        .into_any_element()
}

fn render_search_bar(
    view: &DiffView,
    window: &Window,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let matches = view.current_matches();
    let total = matches.len();
    let current = view
        .search_match_index
        .filter(|&i| i < total)
        .map(|i| i + 1)
        .unwrap_or(0);
    let count = format!("{current}/{total}");
    let focused = view
        .search_field
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let side = view.search_side;
    let files = view.search_files;

    // Fills the measured OverlaySlot (window-root, after DualPane) so shadow
    // falls on the code. Controls: input | Side | Files | Find | n/N | prev | next | close.
    div()
        .size_full()
        .flex()
        .items_center()
        .gap_1()
        .p(px(theme::FIND_BAR_PAD))
        .rounded(px(theme::FIND_BAR_RADIUS))
        .bg(theme::find_bar_bg())
        .shadow(theme::find_bar_shadow())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(theme::FIND_FIELD_HEIGHT))
                .flex()
                .items_center()
                .rounded(px(theme::FIND_FIELD_RADIUS))
                .border_1()
                .border_color(if focused {
                    theme::border_focused()
                } else {
                    theme::line()
                })
                .child(view.search_field.clone()),
        )
        .child(render_side_segment(side, cx))
        .child(render_files_segment(files, cx))
        .child(
            IconButton::new("search-run", "search.svg", "Find")
                .shortcut("Enter")
                .disabled(total == 0)
                .on_click(cx.listener(|this, _, _, cx| this.jump_search(1, cx))),
        )
        .child(
            div()
                .flex_none()
                .ui_text_size(12., cx)
                .text_color(if total == 0 {
                    theme::faint()
                } else {
                    theme::muted()
                })
                .child(count),
        )
        .child(
            IconButton::new("search-prev", "chevron_up.svg", "Previous Match")
                .shortcut("Shift-Enter")
                .disabled(total == 0)
                .on_click(cx.listener(|this, _, _, cx| this.jump_search(-1, cx))),
        )
        .child(
            IconButton::new("search-next", "chevron_down.svg", "Next Match")
                .shortcut("Enter")
                .disabled(total == 0)
                .on_click(cx.listener(|this, _, _, cx| this.jump_search(1, cx))),
        )
        .child(
            IconButton::new("search-close", "close.svg", "Close Find")
                .shortcut("Esc")
                .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
        )
        .into_any_element()
}

/// Find Side/Files track: same height / inset / radius / border as [`capsule`].
fn search_seg_track() -> Div {
    div()
        .flex_none()
        .h(theme::TOGGLE_SIZE)
        .flex()
        .items_center()
        .p(px(NAV_INSET))
        .rounded(px(NAV_BUTTON_RADIUS + NAV_INSET + 1.))
        .border_1()
        .border_color(theme::line())
        .bg(theme::white())
}

fn search_seg_btn(
    id: &'static str,
    label: &'static str,
    selected: bool,
    cx: &mut Context<DiffView>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h_full()
        .px(px(6.))
        .rounded(px(NAV_BUTTON_RADIUS))
        .flex()
        .items_center()
        .cursor_pointer()
        .when(selected, |d| d.bg(theme::range()))
        .when(!selected, |d| d.hover(|d| d.bg(theme::hover())))
        .on_click(on_click)
        .child(
            div()
                .ui_label_size(11., cx)
                .font_weight(if selected {
                    gpui::FontWeight::MEDIUM
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if selected {
                    theme::accent()
                } else {
                    theme::muted()
                })
                .child(label),
        )
}

fn render_side_segment(side: SearchSide, cx: &mut Context<DiffView>) -> impl IntoElement {
    search_seg_track()
        .id("search-side")
        .child(search_seg_btn(
            "search-side-preimage",
            "Preimage",
            side == SearchSide::Preimage,
            cx,
            cx.listener(|this, _, _, cx| this.set_search_side(SearchSide::Preimage, cx)),
        ))
        .child(search_seg_btn(
            "search-side-postimage",
            "Postimage",
            side == SearchSide::Postimage,
            cx,
            cx.listener(|this, _, _, cx| this.set_search_side(SearchSide::Postimage, cx)),
        ))
        .child(search_seg_btn(
            "search-side-both",
            "Both",
            side == SearchSide::Both,
            cx,
            cx.listener(|this, _, _, cx| this.set_search_side(SearchSide::Both, cx)),
        ))
}

fn render_files_segment(files: SearchFiles, cx: &mut Context<DiffView>) -> impl IntoElement {
    search_seg_track()
        .id("search-files")
        .child(search_seg_btn(
            "search-files-file",
            "File",
            files == SearchFiles::File,
            cx,
            cx.listener(|this, _, _, cx| this.set_search_files_factor(SearchFiles::File, cx)),
        ))
        .child(search_seg_btn(
            "search-files-all",
            "All",
            files == SearchFiles::All,
            cx,
            cx.listener(|this, _, _, cx| this.set_search_files_factor(SearchFiles::All, cx)),
        ))
}

/// The pane's place in the shell. The pane itself is mounted over it by
/// `pane::slot`, so a pane frame never re-renders the shell.
///
/// When find is open, a leading spacer clears the floating capsule so the
/// measured slot (and DualPane) starts below the bar. Top lines stay reachable
/// without putting the find chrome in flex flow (no island height jitter).
///
/// Draft dock matches find's chrome path: island-absolute measure +
/// `overlay_slot` after DualPane. No bottom flex clearance — opening it must
/// not resize DualPane (that was the jitter). The dock floats over the code.
fn render_body(view: &DiffView, find_open: bool, cx: &mut Context<DiffView>) -> impl IntoElement {
    match view.snapshot.as_ref().map(|s| &s.file) {
        Some(FileDiff::Text { .. }) => {
            let slot = view.pane_bounds.clone();
            div()
                .id("diff-pane-slot")
                .relative()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .when(find_open, |body| {
                    body.child(
                        div()
                            .id("diff-find-clearance")
                            .flex_none()
                            .h(px(theme::FIND_CONTENT_PAD)),
                    )
                })
                .child(
                    div().relative().flex_1().min_h(px(0.)).child(
                        canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    ),
                )
                .into_any_element()
        }
        Some(FileDiff::Binary) => placeholder("Binary file — no Alignment", cx),
        Some(FileDiff::Error(msg)) => placeholder(msg, cx),
        None => placeholder("Open Diff from the main window", cx),
    }
}

/// Right-hand comment island: only when the selected path has DraftComments.
/// Row body selects the span (wash + selected styling); Edit / gutter icons open
/// the dock; Delete removes immediately.
fn render_comment_island(view: &DiffView, cx: &mut Context<DiffView>) -> impl IntoElement {
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

    let n = comments.len();
    let selection = view.pane.read(cx).selection();
    let (scroll, sb) = scrollbar::vertical("diff-comments-sb", cx);

    div()
        .id("diff-comment-island")
        .flex_none()
        .w(px(theme::COMMENT_ISLAND_WIDTH))
        .h_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(theme::white())
        .rounded(px(theme::CHANGES_RADIUS))
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .px(px(12.))
                .pt(px(10.))
                .pb(px(8.))
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .ui_text_size(11., cx)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child("COMMENTS"),
                )
                .child(
                    div()
                        .ui_text_size(11., cx)
                        .text_color(theme::muted())
                        .child(format!("{n}")),
                ),
        )
        .child(scrollbar::overlay_flex(
            div()
                .id("diff-comments-scroll")
                .size_full()
                .pt_1()
                .pb_1()
                .track_scroll(&scroll)
                .overflow_y_scroll()
                .children(comments.into_iter().map(|c| {
                    let label = match &c.anchor {
                        Anchor::Line { side, span, .. } => {
                            span_label(*side, span.start, span.count)
                        }
                        Anchor::File { .. } => "file".into(),
                    };
                    let id = c.id;
                    let selected = match &c.anchor {
                        Anchor::Line { side, span, .. } => {
                            selection_matches_span(selection, *side, *span)
                        }
                        Anchor::File { .. } => false,
                    };
                    div()
                        .id(("cmt", c.id as usize))
                        .mx_1()
                        .my_0p5()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .cursor_pointer()
                        .when(selected, |row| row.bg(theme::range()))
                        .hover(move |row| {
                            if selected {
                                row.bg(theme::range())
                            } else {
                                row.bg(theme::hover())
                            }
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_comment(id, window, cx)
                        }))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .font_family(mono.clone())
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(theme::accent())
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(label),
                                )
                                .child(
                                    IconButton::new(
                                        ("cmt-edit", id as usize),
                                        "pencil.svg",
                                        "Edit DraftComment",
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.begin_edit(id, window, cx);
                                    })),
                                )
                                .child(
                                    IconButton::new(
                                        ("cmt-del", id as usize),
                                        "trash.svg",
                                        "Delete DraftComment",
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.delete_comment(id, window, cx);
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .mt(px(4.))
                                .ui_text_size(12., cx)
                                .text_color(theme::text())
                                .child(c.body),
                        )
                })),
            sb,
        ))
        .into_any_element()
}

/// Invisible slot where the bottom draft dock will be painted above DualPane.
/// Bottom-anchored mirror of [`render_find_measure`].
fn render_draft_measure(view: &DiffView, cx: &App) -> impl IntoElement {
    if view.drafting.is_none() {
        return div().into_any_element();
    }
    let slot = view.draft_dock_bounds.clone();
    let h = draft_dock_h_for(view, cx);
    div()
        .absolute()
        .bottom(px(theme::FIND_BAR_INSET))
        .left(px(theme::FIND_BAR_INSET))
        .right(px(theme::FIND_BAR_INSET))
        .h(px(h))
        .child(canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {}).size_full())
        .into_any_element()
}

/// The bottom dock: find-bar chrome (inset / radius / shadow-only / translucent
/// white) with a meta row over the DraftComment body field.
fn render_draft_dock(
    view: &DiffView,
    window: &Window,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let Some(draft) = &view.drafting else {
        return div().into_any_element();
    };
    let label = span_label(draft.side, draft.start, draft.count);
    let focused = view
        .draft_field
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let mono = appearance::code_font(cx);

    // Occlude so DualPane under the float does not take the click (that was
    // starting a selection and cancelling the draft). Find avoids this because
    // its clearance keeps DualPane out from under the bar; draft floats over.
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(theme::DRAFT_DOCK_GAP))
        .p(px(theme::FIND_BAR_PAD))
        .rounded(px(theme::FIND_BAR_RADIUS))
        .bg(theme::find_bar_bg())
        .shadow(theme::find_bar_shadow())
        .occlude()
        .child(
            div()
                .flex_none()
                .h(px(theme::DRAFT_META_HEIGHT))
                .flex()
                .items_center()
                .gap_2()
                .pl(px(6.))
                .child(
                    div()
                        .ui_text_size(11., cx)
                        .text_color(theme::muted())
                        .child(if draft.editing.is_some() {
                            "Edit DraftComment ·"
                        } else {
                            "DraftComment ·"
                        }),
                )
                .child(
                    div()
                        .font_family(mono)
                        .text_xs()
                        .text_color(theme::text())
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .ui_text_size(11., cx)
                        .text_color(theme::faint())
                        .child("Enter to save · Shift-Enter for a newline · Esc to cancel"),
                )
                .child(
                    IconButton::new("draft-cancel", "close.svg", "Cancel DraftComment")
                        .shortcut("Esc")
                        .on_click(cx.listener(|this, _, window, cx| this.cancel_draft(window, cx))),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .items_start()
                .rounded(px(theme::FIND_FIELD_RADIUS))
                .border_1()
                .border_color(if focused {
                    theme::border_focused()
                } else {
                    theme::line()
                })
                .child(view.draft_field.clone()),
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
                .flex()
                .items_center()
                .gap_1()
                .ui_label_size(12., cx)
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

/// Group of toolbar buttons, `TOGGLE_SIZE` tall like any other button.
fn capsule() -> Div {
    div()
        .flex_none()
        .h(theme::TOGGLE_SIZE)
        .flex()
        .items_center()
        .p(px(NAV_INSET))
        .gap(px(NAV_INSET))
        // Concentric with the buttons' hover: button radius + inset.
        .rounded(px(NAV_BUTTON_RADIUS + NAV_INSET))
        .bg(theme::white())
}

/// Fills the capsule's height: `TOGGLE_SIZE` less inset on both sides.
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
                .text_color(if font_px == DiffFontSize::DEFAULT {
                    theme::muted()
                } else {
                    theme::accent()
                })
                .tooltip(Tooltip::text("Reset Text Size", Some(tooltip::cmd("0"))))
                .hover(|button| button.bg(theme::hover()))
                .active(|button| button.bg(theme::element_active()))
                .on_click(cx.listener(|this, _, _, cx| this.font_size(FontOp::Reset, cx)))
                .child(div().ui_label_size(12., cx).child(font_px.to_string())),
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

#[cfg(test)]
mod tests {
    use super::{LineSpan, Side, selection_matches_span, span_label};

    #[test]
    fn span_label_names_one_line_and_a_range() {
        assert_eq!(span_label(Side::Postimage, 3, 1), "postimage L3");
        assert_eq!(span_label(Side::Preimage, 3, 3), "preimage L3–5");
        // A zero count can only come from a bad write; still not a range.
        assert_eq!(span_label(Side::Postimage, 7, 0), "postimage L7");
    }

    #[test]
    fn selection_matches_span_by_side_and_inclusive_end() {
        let span = LineSpan { start: 3, count: 3 };
        assert!(selection_matches_span(
            Some((Side::Postimage, 3, 5)),
            Side::Postimage,
            span
        ));
        assert!(!selection_matches_span(
            Some((Side::Preimage, 3, 5)),
            Side::Postimage,
            span
        ));
        assert!(!selection_matches_span(
            Some((Side::Postimage, 3, 4)),
            Side::Postimage,
            span
        ));
        assert!(!selection_matches_span(None, Side::Postimage, span));
    }
}

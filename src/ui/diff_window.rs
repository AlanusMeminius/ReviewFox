use crate::publication::{
    BatchCancellation, Connection, DeleteStatus, EditStatus, PublicationBatch, PublicationKey,
    PublicationRecord, PublicationService, PublicationState, PublicationStore, ReviewOrigin,
};
use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, ClipboardItem, Context, Div, Entity,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Subscription, Task, Timer, WeakEntity, Window, WindowControlArea, actions, canvas, div,
    ease_out_quint, prelude::*, px, svg,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::{
    Anchor, ChangedPath, Comparison, DiffFontSize, LineSpan, PathStatus, SearchFileText,
    SearchFiles, SearchMatch, SearchSide, Side, ViewOptions, next_match_index, next_search_side,
    prev_match_index, search_file, search_files, toggle_search_files,
};

use super::appearance::{self, UiTextSize};
use super::diff::pane::{self, DualPane, FontOp, PaneComment, PaneEvent, SlotBounds, placeholder};
use super::diff::review::{OpenReview, PathView};
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
    pub origin: crate::publication::ReviewOrigin,
    pub comparison: Comparison,
    pub changed_paths: Vec<ChangedPath>,
    pub selected_path: String,
    pub file: FileDiff,
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

struct ActiveBatch {
    cancellation: BatchCancellation,
    keys: Vec<PublicationKey>,
}
impl Drop for ActiveBatch {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// The Diff window shell: tree, chrome, search bar, comments, draft dock and
/// Review. The dual pane is its own Entity (`DualPane`), driven by methods
/// and heard through `PaneEvent`s, so scrolling notifies only the pane.
pub struct DiffView {
    focus: FocusHandle,
    tree_collapsed: bool,
    tree_width: f32,
    tree_resize_state: Rc<ResizeState>,
    /// The toggle's preference; the narrow-window yield never rewrites it.
    comments_visible: bool,
    /// Last width the user gave the comment island.
    comment_width: f32,
    /// Shown by the toggle while the window yields it: the diff floor and the
    /// snap threshold are dropped until the island fits again.
    comments_forced: bool,
    /// Generation bumped per animated show/hide so the drawer animation restarts.
    /// `None` while nothing should animate (window open, live resize).
    comment_anim_gen: Option<usize>,
    /// Diff card width held until the show/hide slide finishes. The pane keeps
    /// this layout so its code does not reflow over the comment island; the
    /// card edge and the island edge move together, then the pane catches up.
    comment_pane_freeze: Option<f32>,
    comment_resize_state: Rc<ResizeState>,
    pub snapshot: Option<DiffSnapshot>,
    /// Review for the Comparison on screen, and the unsaved line dock.
    open_review: OpenReview,
    publication_store: Option<PublicationStore>,
    publication_records: HashMap<u64, PublicationRecord>,
    publication_protected: HashSet<u64>,
    publication_error: Option<String>,
    publishing: HashSet<PublicationKey>,
    updating: HashSet<PublicationKey>,
    deleting: HashSet<PublicationKey>,
    queued_deletes: HashSet<PublicationKey>,
    queued_updates: HashSet<PublicationKey>,
    refreshing: HashSet<(Comparison, ReviewOrigin)>,
    checking: HashSet<PublicationKey>,
    check_errors: HashMap<PublicationKey, String>,
    republish_confirmations: HashMap<PublicationKey, String>,
    active_batch: Option<ActiveBatch>,
    batch_status: Option<String>,
    export_status: Option<String>,
    /// Ephemeral; paths in set are collapsed. Default empty = all expanded.
    collapsed_dirs: HashSet<String>,
    /// Reset collapsed_dirs when this no longer matches current ChangedPath list.
    tree_path_fingerprint: Vec<String>,
    /// Sidebar path filter. Empty shows every changed file. Mirrored from [`Self::tree_filter`].
    tree_query: String,
    tree_filter: Entity<TextField>,
    _tree_filter_sub: Subscription,
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
    /// Cached preimage/postimage texts for All-files find, in tree order.
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
        let open_review = match crate::review_store::ReviewStore::application() {
            Ok(store) => OpenReview::reopen(
                snapshot.comparison.clone(),
                snapshot.selected_path.clone(),
                snapshot.origin.clone(),
                store,
            ),
            Err(error) => OpenReview::unavailable(
                snapshot.comparison.clone(),
                snapshot.selected_path.clone(),
                snapshot.origin.clone(),
                error.to_string(),
            ),
        };
        let pane = cx.new(DualPane::new);
        let pane_events =
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| match event {
                PaneEvent::OpenDraft { side, start, count } => {
                    this.begin_draft(*side, *start, *count, window, cx)
                }
                PaneEvent::OpenEdit { id } => this.begin_edit(*id, window, cx),
                PaneEvent::SelectionStarted => {
                    // New gutter drag already owns the wash — only close the dock.
                    if this.open_review.dock().is_some() {
                        this.close_dock(window, cx);
                    }
                }
                PaneEvent::FocusDiff => window.focus(&this.focus),
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
        let tree_filter =
            cx.new(|cx| TextField::new("search", false, cx).with_style(TextFieldStyle::Search));
        let tree_filter_sub = cx.observe(&tree_filter, |this, field, cx| {
            let q = field.read(cx).content().to_string();
            if q == this.tree_query {
                return;
            }
            // Only the empty → text step opens matching dirs. A collapse during
            // the same query stays until the field is cleared.
            let was_empty = this.tree_query.trim().is_empty();
            this.tree_query = q;
            if was_empty {
                this.expand_dirs_for_tree_query();
            }
            cx.notify();
        });
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
        let publication_store = PublicationStore::application().ok();
        let open_review = match &publication_store {
            Some(store) => open_review.with_publication_store(store.clone()),
            None => open_review,
        };
        // Grow/shrink the bottom dock when Shift+Enter adds lines.
        let draft_observe = cx.observe(&draft_field, |_, _, cx| cx.notify());
        let shell = window_geometry_store::snapshot();
        let mut this = Self {
            focus: cx.focus_handle(),
            tree_collapsed: shell.tree_collapsed,
            tree_width: f32::from(theme::DIFF_TREE_WIDTH),
            tree_resize_state: Rc::new(ResizeState::with_drag_latch()),
            comments_visible: shell.comments_visible,
            comment_width: theme::COMMENT_ISLAND_WIDTH,
            comments_forced: false,
            comment_anim_gen: None,
            comment_pane_freeze: None,
            comment_resize_state: Rc::new(ResizeState::with_drag_latch()),
            snapshot: Some(snapshot),
            open_review,
            publication_store,
            publication_records: HashMap::new(),
            publication_protected: HashSet::new(),
            publication_error: None,
            publishing: HashSet::new(),
            updating: HashSet::new(),
            deleting: HashSet::new(),
            queued_deletes: HashSet::new(),
            queued_updates: HashSet::new(),
            refreshing: HashSet::new(),
            checking: HashSet::new(),
            check_errors: HashMap::new(),
            republish_confirmations: HashMap::new(),
            active_batch: None,
            batch_status: None,
            export_status: None,
            collapsed_dirs: HashSet::new(),
            tree_path_fingerprint: Vec::new(),
            tree_query: String::new(),
            tree_filter,
            _tree_filter_sub: tree_filter_sub,
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
        this.reload_publications();
        this.refresh_publications(cx);
        this.open_in_pane(cx);
        this
    }

    fn reload_publications(&mut self) {
        self.publication_records.clear();
        self.publication_error = None;
        let Some(store) = &self.publication_store else {
            self.publication_error = Some("Publication storage unavailable".into());
            return;
        };
        match store.protected_comments(&self.open_review.review().comparison) {
            Ok(protected) => self.publication_protected = protected,
            Err(error) => self.publication_error = Some(format!("{error:#}")),
        }
        for comment in &self.open_review.review().comments {
            match store.load(
                &self.open_review.review().comparison,
                self.open_review.origin(),
                comment.id,
            ) {
                Ok(Some(record)) => {
                    self.publication_records.insert(comment.id, record);
                }
                Ok(None) => {}
                Err(error) => {
                    self.publication_error = Some(format!("{error:#}"));
                }
            }
        }
    }

    fn publication_in_progress(&self, id: u64) -> bool {
        self.publication_in_progress_for(&self.open_review.review().comparison, id)
    }
    fn publication_in_progress_for(&self, comparison: &Comparison, id: u64) -> bool {
        self.publishing
            .iter()
            .chain(self.updating.iter())
            .chain(self.deleting.iter())
            .chain(self.checking.iter())
            .any(|key| key.comparison == *comparison && key.comment_id == id)
    }
    fn can_delete_comment(&self, id: u64) -> bool {
        self.publication_error.is_none()
            && self.open_review.storage_error().is_none()
            && match self.publication_records.get(&id) {
                Some(record) => {
                    !matches!(
                        record.deletion,
                        Some(DeleteStatus::Sending | DeleteStatus::Unknown(_))
                    ) && (!matches!(
                        record.state,
                        PublicationState::Failed(_) | PublicationState::Unsupported(_)
                    ) || !self.publication_in_progress(id))
                }
                None => {
                    !self.publication_in_progress(id) && !self.publication_protected.contains(&id)
                }
            }
    }

    fn can_publish(&self, id: u64) -> bool {
        self.publication_error.is_none()
            && self.open_review.storage_error().is_none()
            && self
                .open_review
                .origin()
                .preparation_eligibility(&self.open_review.review().comparison)
                .is_ok()
            && !self.publication_in_progress(id)
            && self.open_review.review().comments.iter().any(|c| {
                c.id == id && matches!(&c.anchor, Anchor::Line { span, .. } if span.count>0)
            })
            && self.publication_records.get(&id).is_none_or(|record| {
                (matches!(
                    record.state,
                    PublicationState::Failed(_) | PublicationState::Unsupported(_)
                ) || record.website_deleted
                    || record.deletion == Some(DeleteStatus::Confirmed))
                    && !matches!(
                        record.deletion,
                        Some(DeleteStatus::Sending | DeleteStatus::Unknown(_))
                    )
                    && !record.edit.as_ref().is_some_and(|edit| {
                        matches!(
                            edit.status,
                            EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                        )
                    })
            })
    }

    fn batch_draft_ids(&self) -> Vec<u64> {
        if self.publication_error.is_some()
            || self.open_review.storage_error().is_some()
            || self
                .open_review
                .origin()
                .preparation_eligibility(&self.open_review.review().comparison)
                .is_err()
        {
            return Vec::new();
        }
        self.open_review
            .review()
            .comments
            .iter()
            .filter(|comment| {
                PublicationRecord::batch_eligible(self.publication_records.get(&comment.id))
            })
            .map(|comment| comment.id)
            .collect()
    }
    fn can_publish_batch(&self) -> bool {
        let ids = self.batch_draft_ids();
        self.active_batch.is_none()
            && !ids.is_empty()
            && ids.iter().all(|id| !self.publication_in_progress(*id))
    }
    fn publish_all(&mut self, cx: &mut Context<Self>) {
        if !self.can_publish_batch() {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        let origin = self.open_review.origin().clone();
        let comparison = self.open_review.review().comparison.clone();
        match service.reserve_all(
            self.open_review.review(),
            self.open_review.contexts(),
            &origin,
        ) {
            Ok(batch) => {
                let keys: Vec<_> = batch
                    .ids()
                    .map(|id| PublicationKey::new(comparison.clone(), origin.clone(), id))
                    .collect();
                self.publishing.extend(keys.iter().cloned());
                self.active_batch = Some(ActiveBatch {
                    cancellation: batch.cancellation(),
                    keys,
                });
                self.batch_status = Some(format!(
                    "Publishing {} drafts to GitLab…",
                    batch.remaining()
                ));
                self.reload_publications();
                self.publish_batch_step(batch, service, cx);
            }
            Err(error) => {
                self.publication_error = Some(format!("{error:#}"));
            }
        }
        cx.notify();
    }
    fn cancel_batch(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = &self.active_batch {
            active.cancellation.cancel();
        }
        self.batch_status = Some("Stopping batch; unsent items require manual retry".into());
        cx.notify();
    }
    fn publish_batch_step(
        &mut self,
        mut batch: PublicationBatch,
        service: PublicationService,
        cx: &mut Context<Self>,
    ) {
        // Each task sends exactly one item. A closed window drops the returned queue,
        // so no detached loop can continue publishing after the view disappears.
        let next_id = batch.ids().next();
        cx.spawn(async move |this, cx| {
            let (batch, result) = cx
                .background_executor()
                .spawn({
                    let service = service.clone();
                    async move {
                        let connection = Connection::new(
                            crate::settings_store::effective_base_url(
                                &crate::settings_store::load_file(),
                            ),
                            crate::settings_store::load_pat().unwrap_or_default(),
                        );
                        let result = batch.publish_next(&service, &connection).await;
                        (batch, result)
                    }
                })
                .await;
            let _ = this.update(cx, move |this, cx| {
                let key = next_id.and_then(|id| {
                    this.active_batch
                        .as_ref()?
                        .keys
                        .iter()
                        .find(|key| key.comment_id == id)
                        .cloned()
                });
                if let Some(key) = &key {
                    this.publishing.remove(key);
                }
                let stopped = !matches!(&result, Ok(Some(_)));
                if let Err(error) = result {
                    this.publication_error = Some(format!("{error:#}"));
                }
                this.reload_publications();
                if let Some(key) = &key {
                    if key.matches_review(
                        &this.open_review.review().comparison,
                        this.open_review.origin(),
                    ) && this.queued_updates.remove(key)
                        && this
                            .publication_records
                            .get(&key.comment_id)
                            .is_some_and(|record| record.receipt().is_some())
                    {
                        this.queue_comment_update(key.comment_id, cx);
                    }
                    this.continue_deletions(key, cx);
                }
                if stopped || batch.remaining() == 0 {
                    drop(batch);
                    if let Some(active) = this.active_batch.take() {
                        for key in &active.keys {
                            this.publishing.remove(key);
                            this.continue_deletions(key, cx);
                        }
                    }
                    this.reload_publications();
                    this.batch_status =
                        Some("Batch finished; each comment shows its result".into());
                } else {
                    this.publish_batch_step(batch, service, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn publish_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.can_publish(id) {
            return;
        }
        let Some(store) = self.publication_store.clone() else {
            return;
        };
        let review = self.open_review.review().clone();
        let origin = self.open_review.origin().clone();
        let service = PublicationService::new(cx.http_client(), store);
        match service.queue_create(&review, &origin, id) {
            Ok(record) => {
                self.publication_records.insert(id, record);
            }
            Err(error) => {
                self.publication_error = Some(format!("{error:#}"));
                cx.notify();
                return;
            }
        }
        self.send_queued_comment(id, cx);
    }
    fn send_queued_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        let review = self.open_review.review().clone();
        let contexts = self.open_review.contexts().clone();
        let origin = self.open_review.origin().clone();
        let key = PublicationKey::new(review.comparison.clone(), origin.clone(), id);
        self.publishing.insert(key.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let base_url = crate::settings_store::effective_base_url(
                        &crate::settings_store::load_file(),
                    );
                    let connection = Connection::new(
                        base_url,
                        crate::settings_store::load_pat().unwrap_or_default(),
                    );
                    service
                        .publish(&review, &contexts, &origin, id, &connection)
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.publishing.remove(&key);
                if this.open_review.review().comparison == key.comparison {
                    this.reload_publications();
                }
                if key.matches_review(
                    &this.open_review.review().comparison,
                    this.open_review.origin(),
                ) {
                    match result {
                        Ok(record) => {
                            this.publication_records.insert(id, record);
                        }
                        Err(error) => {
                            this.publication_error = Some(format!("{error:#}"));
                        }
                    }
                    if this.queued_updates.remove(&key)
                        && this
                            .publication_records
                            .get(&id)
                            .is_some_and(|record| record.receipt().is_some())
                    {
                        this.queue_comment_update(id, cx);
                    }
                }
                this.continue_deletions(&key, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn can_check_comment(&self, id: u64) -> bool {
        !self.publication_in_progress(id)
            && self.open_review.storage_error().is_none()
            && self.publication_records.get(&id).is_some_and(|record| {
                record.receipt().is_some() || matches!(record.state, PublicationState::Unknown(_))
            })
    }
    fn can_republish_comment(&self, id: u64) -> bool {
        self.can_check_comment(id)
            && self
                .open_review
                .origin()
                .preparation_eligibility(&self.open_review.review().comparison)
                .is_ok()
            && self.publication_records.get(&id).is_some_and(|record| {
                record.receipt().is_none()
                    && record.deletion.is_none()
                    && matches!(record.state, PublicationState::Unknown(_))
            })
    }
    fn can_keep_uncertain_comment(&self, id: u64) -> bool {
        !self.publication_in_progress(id)
            && self.open_review.storage_error().is_none()
            && self.publication_records.get(&id).is_some_and(|record| {
                record.receipt().is_none()
                    && matches!(record.state, PublicationState::Unknown(_))
                    && record.deletion == Some(DeleteStatus::Pending)
            })
    }
    fn keep_uncertain_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.can_keep_uncertain_comment(id) {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        match service.cancel_pending_delete(
            &self.open_review.review().comparison,
            self.open_review.origin(),
            id,
        ) {
            Ok(record) => {
                self.publication_records.insert(id, record);
                self.queued_deletes.remove(&PublicationKey::new(
                    self.open_review.review().comparison.clone(),
                    self.open_review.origin().clone(),
                    id,
                ));
            }
            Err(error) => {
                self.publication_error = Some(format!("{error:#}"));
            }
        }
        cx.notify();
    }
    fn request_republish(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.can_republish_comment(id) {
            return;
        }
        let key = PublicationKey::new(
            self.open_review.review().comparison.clone(),
            self.open_review.origin().clone(),
            id,
        );
        self.republish_confirmations
            .insert(key, self.publication_records[&id].operation_id.clone());
        cx.notify();
    }
    fn confirm_republish(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.can_republish_comment(id) {
            return;
        }
        let key = PublicationKey::new(
            self.open_review.review().comparison.clone(),
            self.open_review.origin().clone(),
            id,
        );
        if self.republish_confirmations.get(&key)
            != self
                .publication_records
                .get(&id)
                .map(|record| &record.operation_id)
        {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        match service.queue_republish(
            self.open_review.review(),
            self.open_review.origin(),
            id,
            true,
        ) {
            Ok(record) => {
                self.publication_records.insert(id, record);
                self.republish_confirmations.remove(&key);
                self.send_queued_comment(id, cx);
            }
            Err(error) => {
                self.check_errors.insert(key, format!("{error:#}"));
            }
        }
        cx.notify();
    }
    fn check_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.can_check_comment(id) {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        let key = PublicationKey::new(
            self.open_review.review().comparison.clone(),
            self.open_review.origin().clone(),
            id,
        );
        self.check_errors.remove(&key);
        self.checking.insert(key.clone());
        cx.spawn(async move |this, cx| {
            let reader = service.clone();
            let captured = key.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    reader
                        .check_again(
                            &captured.comparison,
                            &captured.origin,
                            id,
                            &Self::saved_connection(),
                        )
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.checking.remove(&key);
                if key.matches_review(
                    &this.open_review.review().comparison,
                    this.open_review.origin(),
                ) {
                    let Some(comment) = this
                        .open_review
                        .review()
                        .comments
                        .iter()
                        .find(|comment| comment.id == id)
                    else {
                        return;
                    };
                    let editing = this
                        .open_review
                        .dock()
                        .is_some_and(|dock| dock.editing == Some(id));
                    match result.and_then(|checked| {
                        service.reconcile_check(
                            &key.comparison,
                            &key.origin,
                            id,
                            &comment.body,
                            editing,
                            checked,
                        )
                    }) {
                        Ok(outcome) => {
                            if let Some(body) = outcome.adopt_body {
                                let view = this.open_review.adopt_body(id, &body);
                                this.apply_review(&view, cx);
                            }
                            let deleted = outcome.record.deletion == Some(DeleteStatus::Confirmed);
                            this.publication_records.insert(id, outcome.record);
                            if deleted {
                                let view = this.open_review.complete_delete(id);
                                this.apply_review(&view, cx);
                            }
                        }
                        Err(error) => {
                            this.check_errors
                                .insert(key.clone(), format!("Check failed: {error:#}"));
                        }
                    }
                    // Checking never resumes queued update/delete writes. Their durable intent
                    // remains available through the explicit retry controls.
                    this.queued_updates.remove(&key);
                    this.queued_deletes.remove(&key);
                    this.reload_publications();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn publication_service(&self, cx: &App) -> Option<PublicationService> {
        self.publication_store
            .clone()
            .map(|store| PublicationService::new(cx.http_client(), store))
    }
    fn continue_deletions(&mut self, finished: &PublicationKey, cx: &mut Context<Self>) {
        let keys: Vec<_> = self
            .queued_deletes
            .iter()
            .filter(|key| {
                key.comparison == finished.comparison && key.comment_id == finished.comment_id
            })
            .cloned()
            .collect();
        for key in keys {
            let Some(store) = &self.publication_store else {
                continue;
            };
            let Ok(Some(record)) = store.load_key(&key) else {
                continue;
            };
            if record.deletion == Some(DeleteStatus::Confirmed) {
                self.queued_deletes.remove(&key);
                if key.matches_review(
                    &self.open_review.review().comparison,
                    self.open_review.origin(),
                ) {
                    let view = self.open_review.complete_delete(key.comment_id);
                    self.apply_review(&view, cx);
                    self.reload_publications();
                }
            } else if record.receipt().is_some() && record.deletion == Some(DeleteStatus::Pending) {
                self.start_delete(key, false, cx);
            }
        }
    }
    fn start_delete(
        &mut self,
        key: PublicationKey,
        confirm_changed_body: bool,
        cx: &mut Context<Self>,
    ) {
        if self.publication_in_progress_for(&key.comparison, key.comment_id) {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        let Ok(Some(record)) = service.store.load_key(&key) else {
            return;
        };
        if record.receipt().is_none()
            || record.website_deleted
            || record.edit.as_ref().is_some_and(|edit| {
                matches!(
                    edit.status,
                    EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                )
            })
        {
            return;
        }
        self.queued_deletes.remove(&key);
        self.queued_updates.remove(&key);
        self.deleting.insert(key.clone());
        let task_key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service
                        .delete(
                            &task_key.comparison,
                            &task_key.origin,
                            task_key.comment_id,
                            &Self::saved_connection(),
                            confirm_changed_body,
                        )
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.deleting.remove(&key);
                if key.matches_review(
                    &this.open_review.review().comparison,
                    this.open_review.origin(),
                ) {
                    match result {
                        Ok(record) => {
                            if record.deletion == Some(DeleteStatus::Confirmed) {
                                let view = this.open_review.complete_delete(key.comment_id);
                                this.apply_review(&view, cx);
                            }
                            this.reload_publications();
                        }
                        Err(error) => {
                            this.publication_error = Some(format!("Delete failed: {error:#}"));
                        }
                    }
                }
                this.continue_deletions(&key, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn retry_delete(&mut self, id: u64, confirm_changed_body: bool, cx: &mut Context<Self>) {
        self.start_delete(
            PublicationKey::new(
                self.open_review.review().comparison.clone(),
                self.open_review.origin().clone(),
                id,
            ),
            confirm_changed_body,
            cx,
        );
    }
    fn saved_connection() -> Connection {
        Connection::new(
            crate::settings_store::effective_base_url(&crate::settings_store::load_file()),
            crate::settings_store::load_pat().unwrap_or_default(),
        )
    }
    fn refresh_publications(&mut self, cx: &mut Context<Self>) {
        let comparison = self.open_review.review().comparison.clone();
        let origin = self.open_review.origin().clone();
        let refresh_key = (comparison.clone(), origin.clone());
        if self.refreshing.contains(&refresh_key) {
            return;
        }
        let ids: Vec<_> = self
            .publication_records
            .iter()
            .filter_map(|(id, record)| {
                record
                    .receipt()
                    .filter(|_| record.deletion != Some(DeleteStatus::Confirmed))
                    .map(|_| *id)
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        self.refreshing.insert(refresh_key.clone());
        // Convert durable sends with no live task to uncertainty; restart never retries them.
        for id in &ids {
            if !self.publication_in_progress(*id) {
                if let Ok(record) = service.restore_interrupted(&comparison, &origin, *id) {
                    self.publication_records.insert(*id, record);
                }
            }
        }
        cx.spawn(async move |this, cx| {
            let reader = service.clone();
            let results = cx
                .background_executor()
                .spawn(async move {
                    let connection = Self::saved_connection();
                    let mut results = Vec::new();
                    for id in ids {
                        results
                            .push((id, reader.read(&comparison, &origin, id, &connection).await));
                    }
                    results
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.refreshing.remove(&refresh_key);
                if this.open_review.review().comparison == refresh_key.0
                    && this.open_review.origin().same_target(&refresh_key.1)
                {
                    this.reload_publications();
                    for (id, result) in results {
                        let Some(comment) = this
                            .open_review
                            .review()
                            .comments
                            .iter()
                            .find(|comment| comment.id == id)
                        else {
                            continue;
                        };
                        let editing = this
                            .open_review
                            .dock()
                            .is_some_and(|dock| dock.editing == Some(id));
                        match result.and_then(|remote| {
                            service.reconcile(
                                &refresh_key.0,
                                &refresh_key.1,
                                id,
                                &comment.body,
                                editing,
                                remote,
                            )
                        }) {
                            Ok(outcome) => {
                                if let Some(body) = outcome.adopt_body {
                                    let view = this.open_review.adopt_body(id, &body);
                                    this.apply_review(&view, cx);
                                }
                                this.publication_records.insert(id, outcome.record);
                            }
                            Err(error) => {
                                this.publication_error = Some(format!("Refresh failed: {error:#}"))
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn queue_comment_update(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        let comparison = self.open_review.review().comparison.clone();
        let origin = self.open_review.origin().clone();
        let Some(comment) = self
            .open_review
            .review()
            .comments
            .iter()
            .find(|comment| comment.id == id)
        else {
            return;
        };
        match service.queue_edit(&comparison, &origin, id, &comment.body) {
            Ok(record) => {
                self.publication_records.insert(id, record);
            }
            Err(error) => {
                self.publication_error = Some(format!("{error:#}"));
                return;
            }
        }
        let key = PublicationKey::new(comparison, origin, id);
        if self.updating.contains(&key) || self.publication_in_progress(id) {
            self.queued_updates.insert(key);
            return;
        }
        self.update_comment(id, false, cx);
    }
    fn update_comment(&mut self, id: u64, overwrite: bool, cx: &mut Context<Self>) {
        if self.publication_in_progress(id) || self.open_review.storage_error().is_some() {
            return;
        }
        let Some(record) = self.publication_records.get(&id) else {
            return;
        };
        if record.deletion.is_some()
            || record.website_deleted
            || record.edit.as_ref().is_none_or(|edit| {
                matches!(
                    edit.status,
                    EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                ) || (!overwrite && matches!(edit.status, EditStatus::Conflict { .. }))
            })
        {
            return;
        }
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        let key = PublicationKey::new(
            self.open_review.review().comparison.clone(),
            self.open_review.origin().clone(),
            id,
        );
        self.updating.insert(key.clone());
        let task_key = key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service
                        .update(
                            &task_key.comparison,
                            &task_key.origin,
                            id,
                            &Self::saved_connection(),
                            overwrite,
                        )
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.updating.remove(&key);
                let queued = this.queued_updates.remove(&key);
                if key.matches_review(
                    &this.open_review.review().comparison,
                    this.open_review.origin(),
                ) {
                    this.reload_publications();
                    match result {
                        Ok(record) => {
                            this.publication_records.insert(id, record);
                        }
                        Err(error) => this.publication_error = Some(format!("{error:#}")),
                    }
                    if queued
                        && this.publication_records.get(&id).is_some_and(|record| {
                            record
                                .edit
                                .as_ref()
                                .is_some_and(|edit| matches!(edit.status, EditStatus::Pending))
                        })
                    {
                        this.update_comment(id, false, cx);
                    }
                }
                this.continue_deletions(&key, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn adopt_website_body(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.publication_service(cx) else {
            return;
        };
        match service.adopt_website(
            &self.open_review.review().comparison,
            self.open_review.origin(),
            id,
        ) {
            Ok(outcome) => {
                if self
                    .open_review
                    .dock()
                    .is_some_and(|dock| dock.editing == Some(id))
                {
                    self.close_dock(window, cx);
                }
                if let Some(body) = outcome.adopt_body {
                    let view = self.open_review.adopt_body(id, &body);
                    self.apply_review(&view, cx);
                }
                self.publication_records.insert(id, outcome.record);
            }
            Err(error) => self.publication_error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn with_pane<R>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut DualPane, &mut Context<DualPane>) -> R,
    ) -> R {
        self.pane.update(cx, f)
    }

    fn pane_comments(view: &PathView) -> Vec<PaneComment> {
        view.comments
            .iter()
            .map(|c| PaneComment {
                id: c.id,
                anchor: c.anchor.clone(),
            })
            .collect()
    }

    /// Map one module result onto the pane: this path's marks and, if the dock
    /// is open, its line span. Selection and the text field stay here.
    fn apply_review(&mut self, view: &PathView, cx: &mut Context<Self>) {
        let drafting = view.dock.map(|d| {
            (
                d.side,
                d.span.start,
                d.span.start + d.span.count.saturating_sub(1),
            )
        });
        let comments = Self::pane_comments(view);
        self.with_pane(cx, |pane, cx| {
            pane.set_comments(comments, cx);
            pane.set_drafting(drafting, cx);
        });
    }

    fn clear_draft_field(&mut self, cx: &mut Context<Self>) {
        self.draft_field
            .update(cx, |field, cx| field.set_content("", cx));
    }

    /// Hand the selected file to the pane: file start, everything folded.
    fn open_in_pane(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = self.snapshot.as_ref() else {
            return;
        };
        let file = snap.file.clone();
        let path = snap.selected_path.clone();
        let view = self.open_review.current();
        let comments = Self::pane_comments(&view);
        self.with_pane(cx, |pane, cx| pane.open(&path, &file, comments, cx));
        self.apply_review(&view, cx);
    }

    /// Recompute Alignment under the current ViewOptions.
    fn recompute_alignment(&mut self) {
        let opts = self.view_options.clone();
        let Some(snap) = self.snapshot.as_mut() else {
            return;
        };
        let FileDiff::Text {
            alignment,
            preimage_text,
            postimage_text,
        } = &mut snap.file
        else {
            return;
        };
        *alignment = git::compute_alignment(preimage_text, postimage_text, &opts);
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
                preimage_text,
                postimage_text,
                ..
            } = git::file_diff(&snap.comparison, &path, status, &opts)
            {
                out.push((path, preimage_text, postimage_text));
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

    /// Open file's place in the nav order: `(Some(index), total)` when it is in
    /// the list, `(None, total)` when a filter has excluded it. Empty query uses
    /// every changed file. Collapse does not affect this list.
    fn file_position(&self) -> (Option<usize>, usize) {
        let Some(snap) = &self.snapshot else {
            return (None, 0);
        };
        let order = file_tree::file_order_query(&snap.changed_paths, &self.tree_query);
        let index = order.iter().position(|p| *p == snap.selected_path);
        (index, order.len())
    }

    /// Steps to the neighbouring file in nav order. Stops at either end.
    /// An open file outside the list moves to the first match forward, or the last backward.
    fn jump_file(&mut self, dir: i32, cx: &mut Context<Self>) {
        let Some(snap) = &self.snapshot else {
            return;
        };
        let order = file_tree::file_order_query(&snap.changed_paths, &self.tree_query);
        let Some(path) = file_tree::step_file(&order, &snap.selected_path, dir) else {
            return;
        };
        self.select_path(path, cx);
        cx.notify();
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
        if self.open_review.dock().is_some() {
            let view = self.open_review.cancel();
            self.apply_review(&view, cx);
        }
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
                    preimage_text,
                    postimage_text,
                    ..
                }) = self.snapshot.as_ref().map(|s| &s.file)
                else {
                    return Vec::new();
                };
                search_file(
                    preimage_text,
                    postimage_text,
                    &self.search_query,
                    self.search_side,
                )
            }
            SearchFiles::All => {
                let Some(files) = &self.all_search_texts else {
                    return Vec::new();
                };
                let inputs: Vec<SearchFileText<'_>> = files
                    .iter()
                    .map(|(path, preimage, postimage)| SearchFileText {
                        path: path.as_str(),
                        preimage_text: preimage.as_ref(),
                        postimage_text: postimage.as_ref(),
                    })
                    .collect();
                search_files(&inputs, &self.search_query, self.search_side)
            }
        }
    }

    fn tree_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |raw, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                if this.tree_resize_state.drag_consumed() {
                    return;
                }
                let available = f32::from(window.viewport_size().width);
                let origin = this.tree_resize_state.drag_origin().unwrap_or(raw);
                match splitter::leading_rail_drag(
                    raw,
                    origin,
                    !this.tree_collapsed,
                    available,
                    splitter::MIN_DIFF_CONTENT_WIDTH,
                ) {
                    splitter::RailDrag::Stay => {}
                    splitter::RailDrag::Reveal => {
                        this.tree_resize_state.consume_drag();
                        this.tree_collapsed = false;
                        window_geometry_store::set_tree_collapsed(false);
                        cx.notify();
                    }
                    splitter::RailDrag::Show(width) => {
                        let changed = this.tree_collapsed || this.tree_width != width;
                        if this.tree_collapsed {
                            window_geometry_store::set_tree_collapsed(false);
                        }
                        this.tree_collapsed = false;
                        this.tree_width = width;
                        if changed {
                            cx.notify();
                        }
                    }
                    splitter::RailDrag::Hide => {
                        if !this.tree_collapsed {
                            this.tree_collapsed = true;
                            window_geometry_store::set_tree_collapsed(true);
                            cx.notify();
                        }
                    }
                }
            })
            .ok();
        })
    }

    /// Stage width the diff and comment islands share, less the gap between them.
    fn comment_room(&self, window: &Window) -> f32 {
        let tree = if self.tree_collapsed {
            0.
        } else {
            self.tree_width + theme::CHANGES_SHADOW_GAP
        };
        f32::from(window.viewport_size().width)
            - tree
            - theme::CHANGES_INSET * 2.
            - theme::CHANGES_SHADOW_GAP
    }

    fn comment_layout(&self, room: f32) -> splitter::Collapse {
        if self.comments_visible {
            self.comment_fit(room)
        } else {
            splitter::Collapse::Hidden
        }
    }

    /// The island's layout were it shown.
    fn comment_fit(&self, room: f32) -> splitter::Collapse {
        let floor = if self.comments_forced {
            0.
        } else {
            splitter::MIN_DIFF_CONTENT_WIDTH
        };
        let threshold = if self.comments_forced {
            0.
        } else {
            splitter::COLLAPSE_THRESHOLD
        };
        splitter::resolve_collapsible(self.comment_width, room, floor, threshold)
    }

    /// Width of the diff card for the current comment layout.
    fn diff_card_width(&self, room: f32) -> f32 {
        match self.comment_layout(room) {
            splitter::Collapse::Width(width) => (room - width).max(0.),
            splitter::Collapse::Hidden => room + theme::CHANGES_SHADOW_GAP,
        }
    }

    /// Start a show/hide slide from the pane's current width. The freeze drops
    /// when this generation's animation has finished.
    fn begin_comment_anim(&mut self, room: f32, cx: &mut Context<Self>) {
        let visual = self
            .pane_bounds
            .get()
            .map(|bounds| f32::from(bounds.size.width))
            .unwrap_or_else(|| self.diff_card_width(room));
        self.comment_pane_freeze = Some(visual);
        let generation = self.comment_anim_gen.map_or(0, |n| n + 1);
        self.comment_anim_gen = Some(generation);
        cx.spawn(async move |this, cx| {
            Timer::after(COMMENT_DRAWER_ANIM).await;
            this.update(cx, |this, cx| {
                if this.comment_anim_gen == Some(generation) {
                    this.comment_pane_freeze = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Toggle preference only. A narrow-window yield (`comments_forced`) must not call this.
    fn set_comments_preference(&mut self, visible: bool) {
        if self.comments_visible == visible {
            return;
        }
        self.comments_visible = visible;
        window_geometry_store::set_comments_visible(visible);
    }

    /// Show at the last width. Drops the diff floor (and, via `comment_fit`,
    /// the snap threshold) when that width cannot sit beside a 400px diff.
    fn reveal_comments(&mut self, room: f32, cx: &mut Context<Self>) {
        self.begin_comment_anim(room, cx);
        self.set_comments_preference(true);
        self.comments_forced = splitter::resolve_collapsible(
            self.comment_width,
            room,
            splitter::MIN_DIFF_CONTENT_WIDTH,
            splitter::COLLAPSE_THRESHOLD,
        ) == splitter::Collapse::Hidden;
        cx.notify();
    }

    fn toggle_comments(&mut self, window: &Window, cx: &mut Context<Self>) {
        let room = self.comment_room(window);
        if self.comment_layout(room) == splitter::Collapse::Hidden {
            self.reveal_comments(room, cx);
        } else {
            self.begin_comment_anim(room, cx);
            self.set_comments_preference(false);
            self.comments_forced = false;
            cx.notify();
        }
    }

    fn comment_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |raw, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                if this.comment_resize_state.drag_consumed() {
                    return;
                }
                let room = this.comment_room(window);
                let shown = this.comment_layout(room) != splitter::Collapse::Hidden;
                // A short leftward nudge on the parked handle opens like the button.
                // Hide-on-drag still uses the 160px threshold below.
                if !shown {
                    let origin = this.comment_resize_state.drag_origin().unwrap_or(raw);
                    if raw - origin < COMMENT_REVEAL_NUDGE {
                        return;
                    }
                    this.comment_resize_state.consume_drag();
                    this.reveal_comments(room, cx);
                    return;
                }
                // HorizontalTrailing reports distance to viewport right; the pointer
                // rides the middle of the gap, and the stage is inset on the right.
                let requested = raw - theme::CHANGES_INSET - theme::CHANGES_SHADOW_GAP / 2.;
                this.comments_forced = false;
                match splitter::resolve_collapsible(
                    requested,
                    room,
                    splitter::MIN_DIFF_CONTENT_WIDTH,
                    splitter::COLLAPSE_THRESHOLD,
                ) {
                    splitter::Collapse::Width(width) => {
                        if !shown {
                            this.comment_anim_gen = None;
                            this.comment_pane_freeze = None;
                        }
                        this.set_comments_preference(true);
                        this.comment_width = width;
                    }
                    splitter::Collapse::Hidden => {
                        if shown {
                            this.begin_comment_anim(room, cx);
                        }
                        this.set_comments_preference(false);
                    }
                }
                cx.notify();
            })
            .ok();
        })
    }

    fn expand_dirs_for_tree_query(&mut self) {
        if self.tree_query.trim().is_empty() {
            return;
        }
        let Some(paths) = self.snapshot.as_ref().map(|s| s.changed_paths.clone()) else {
            return;
        };
        self.collapsed_dirs.retain(|dir| {
            !paths.iter().any(|p| {
                file_tree::path_matches_query(&p.path, &self.tree_query)
                    && (p.path == *dir || p.path.starts_with(&format!("{dir}/")))
            })
        });
    }

    fn collapse_all_dirs(&mut self) {
        let Some(paths) = self.snapshot.as_ref().map(|s| s.changed_paths.clone()) else {
            return;
        };
        self.collapsed_dirs = file_tree::flatten(&paths, &HashSet::new())
            .into_iter()
            .filter_map(|row| match row {
                TreeRow::Dir { path, .. } => Some(path),
                TreeRow::File { .. } => None,
            })
            .collect();
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
        let had_dock = self.open_review.dock().is_some();
        let shown = self.open_review.show_origin(
            incoming.comparison.clone(),
            incoming.selected_path.clone(),
            incoming.origin.clone(),
        );
        match &mut self.snapshot {
            Some(current)
                if current.comparison == incoming.comparison
                    && current.origin == incoming.origin =>
            {
                current.selected_path = incoming.selected_path;
                current.changed_paths = incoming.changed_paths;
                current.file = incoming.file;
            }
            _ => {
                self.export_status = None;
                self.snapshot = Some(incoming);
            }
        }
        if had_dock && shown.dock.is_none() {
            self.clear_draft_field(cx);
        }
        self.reload_publications();
        self.refresh_publications(cx);
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
        let had_dock = self.open_review.dock().is_some();
        self.open_review.select_path(path.clone());
        if had_dock {
            self.clear_draft_field(cx);
        }
        self.open_in_pane(cx);
        window_geometry_store::note_diff_selected_path(path);
        window_geometry_store::flush();
    }

    /// Open the bottom dock on the span the module already recorded, with `body`
    /// loaded. Find and draft are mutually exclusive, so an open find bar is
    /// dismissed first. The dock's span becomes the selection, so the icon that
    /// reopens it stays under the pointer after a save. Opening the dock does
    /// not navigate: a newly dragged span must stay where the user selected it.
    fn open_dock(
        &mut self,
        view: PathView,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if has_search(self) {
            self.close_search(window, cx);
        }
        self.apply_review(&view, cx);
        let Some(dock) = view.dock else {
            return;
        };
        let end = dock.span.start + dock.span.count.saturating_sub(1);
        self.with_pane(cx, |pane, cx| {
            pane.select_span(dock.side, dock.span.start, end, cx);
        });
        self.draft_field
            .update(cx, |field, cx| field.set_content(body, cx));
        let handle = self.draft_field.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    /// Empty gutter icon: write a line DraftComment on the selected LineSpan.
    fn begin_draft(
        &mut self,
        side: Side,
        start: u32,
        count: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.open_review.begin_draft(side, start, count);
        if let Some(FileDiff::Text {
            preimage_text,
            postimage_text,
            ..
        }) = self.snapshot.as_ref().map(|snapshot| &snapshot.file)
        {
            self.open_review
                .capture_draft_context(preimage_text.clone(), postimage_text.clone());
        }
        self.open_dock(view, String::new(), window, cx);
    }

    /// Filled gutter icon or island Edit: reopen a line DraftComment with its
    /// body loaded. Unknown id or a file Anchor leaves the dock alone.
    fn begin_edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.open_review.begin_edit(id);
        let Some(body) = view.body.clone() else {
            return;
        };
        let dock = view.dock;
        self.open_dock(view, body, window, cx);
        // Editing from the comment island can target a folded/offscreen line.
        // Keep that navigation separate from opening a new draft in the gutter.
        if let Some(dock) = dock {
            self.with_pane(cx, |pane, cx| {
                pane.reveal_line(dock.side, dock.span.start, cx);
            });
        }
    }

    /// Line span of a DraftComment. `None` for an unknown id or a file Anchor.
    fn comment_target(&self, id: u64) -> Option<(Side, LineSpan)> {
        let c = self
            .open_review
            .review()
            .comments
            .iter()
            .find(|c| c.id == id)?;
        match &c.anchor {
            Anchor::Line { side, span, .. } => Some((*side, *span)),
            Anchor::File { .. } => None,
        }
    }

    /// Close the draft dock without touching the pane selection wash.
    fn close_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.open_review.cancel();
        self.apply_review(&view, cx);
        self.clear_draft_field(cx);
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
    /// File anchors stay a no-op.
    fn select_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_review.dock().is_some() {
            self.cancel_draft(window, cx);
        }
        let Some((side, span)) = self.comment_target(id) else {
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
        if !self.can_delete_comment(id) {
            return;
        }
        if self.publication_records.get(&id).is_some_and(|record| {
            !matches!(
                record.state,
                PublicationState::Failed(_) | PublicationState::Unsupported(_)
            ) && record.deletion != Some(DeleteStatus::Confirmed)
        }) {
            let Some(service) = self.publication_service(cx) else {
                return;
            };
            let Some(body) = self
                .open_review
                .review()
                .comments
                .iter()
                .find(|comment| comment.id == id)
                .map(|comment| comment.body.clone())
            else {
                return;
            };
            let key = PublicationKey::new(
                self.open_review.review().comparison.clone(),
                self.open_review.origin().clone(),
                id,
            );
            match service.queue_delete(&key.comparison, &key.origin, id, &body) {
                Ok(record) => {
                    self.publication_records.insert(id, record);
                    self.queued_deletes.insert(key.clone());
                    self.start_delete(key, false, cx);
                }
                Err(error) => {
                    self.publication_error = Some(format!("{error:#}"));
                }
            }
            cx.notify();
            return;
        }
        let clear_wash = self.comment_target(id).is_some_and(|(side, span)| {
            selection_matches_span(self.pane.read(cx).selection(), side, span)
        });
        let editing_this = self
            .open_review
            .dock()
            .is_some_and(|d| d.editing == Some(id));
        let view = self.open_review.delete(id);
        if clear_wash {
            self.with_pane(cx, |pane, cx| pane.clear_selection(cx));
        }
        self.apply_review(&view, cx);
        if editing_this {
            self.clear_draft_field(cx);
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// Enter in the dock. The module trims; an empty body cancels and this also
    /// clears the wash. A real save keeps the wash so the island row stays selected.
    fn commit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_review.dock().is_none() {
            return;
        }
        let edited_id = self.open_review.dock().and_then(|dock| dock.editing);
        let body = self.draft_field.read(cx).content().to_string();
        let blank = body.trim().is_empty();
        let view = self.open_review.commit(&body);
        self.apply_review(&view, cx);
        self.clear_draft_field(cx);
        window.focus(&self.focus);
        if !blank {
            if let Some(id) = edited_id {
                if self
                    .publication_records
                    .get(&id)
                    .is_some_and(|record| record.receipt().is_some())
                {
                    self.queue_comment_update(id, cx);
                } else if self.publication_in_progress(id)
                    && self.publication_records.contains_key(&id)
                {
                    self.queue_comment_update(id, cx);
                }
            }
        }
        if blank {
            self.with_pane(cx, |pane, cx| pane.clear_selection(cx));
        }
        cx.notify();
    }

    /// Cmd/Ctrl+W: always close the Diff window.
    fn close_diff(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(active) = &self.active_batch {
            active.cancellation.cancel();
        }
        window.remove_window();
    }

    /// Esc: TextSelection, then find, cancel draft (clears wash), gutter line span, close.
    fn dismiss_or_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pane.read(cx).has_text_selection() {
            self.with_pane(cx, |pane, cx| pane.clear_text_selection(cx));
            return;
        }
        if has_search(self) {
            self.close_search(window, cx);
            return;
        }
        if self.open_review.dock().is_some() {
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
        if mods.secondary() && !mods.alt && !mods.shift && event.keystroke.key == "c" {
            // A focused find field or draft body owns this key: TextField's Copy
            // already wrote that field's selection. Only the Diff focus writes
            // the TextSelection. A press in the code column focuses the Diff.
            if self.focus.is_focused(window)
                && let Some(text) = self.pane.read(cx).copied_text()
            {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                cx.stop_propagation();
            }
            return;
        }
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
        if self.open_review.dock().is_some() {
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
        let text = export::export_review(self.open_review.review(), self.open_review.contexts());
        if text.is_empty() {
            self.export_status = Some("No DraftComments to export".into());
        } else {
            let n = self.open_review.review().comments.len();
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
        let room = self.comment_room(window);
        if self.comments_forced && room - splitter::MIN_DIFF_CONTENT_WIDTH >= self.comment_width {
            self.comments_forced = false;
        }
        let comment_layout = self.comment_layout(room);

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
                    .child(render_tree_pane(self, tree_w, window, cx))
                    .when(show_tree_split, |d| {
                        d.child(splitter::handle(
                            "diff-tree-resize-handle",
                            Axis::HorizontalLeading,
                            self.tree_resize_handler(cx),
                            self.tree_resize_state.clone(),
                            true,
                        ))
                    })
                    // Frosted desk: the content island floats here. Top and right stay
                    // inset; the left gap is the rail seam while the tree is open,
                    // and the status bar replaces the bottom inset. Comment island
                    // sits to the right of the diff island while it is shown.
                    .child(
                        div()
                            .id("diff-stage")
                            .relative()
                            .h_full()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .bg(theme::software_palette().surface.desk)
                            .flex()
                            .flex_col()
                            .pt(px(theme::CHANGES_TOP_INSET))
                            // Tree open: the rail seam is the left gap. Collapsed:
                            // the island still needs the window-edge inset.
                            .when(self.tree_collapsed, |d| d.pl(px(theme::CHANGES_INSET)))
                            .pr(px(theme::CHANGES_INSET))
                            .child(
                                div()
                                    .id("diff-stage-islands")
                                    .relative()
                                    .flex_1()
                                    .min_h(px(0.))
                                    .flex()
                                    .child(render_dual_pane(self, cx))
                                    .children(render_comment_drawer(self, room, comment_layout, cx))
                                    .when(comment_layout == splitter::Collapse::Hidden, |d| {
                                        d.child(splitter::parked_handle(
                                            "diff-comment-parked-handle",
                                            self.comment_resize_handler(cx),
                                            self.comment_resize_state.clone(),
                                        ))
                                    }),
                            )
                            .child(render_status_bar(self, cx))
                            .when(self.tree_collapsed, |d| {
                                d.child(splitter::parked_leading_handle(
                                    "diff-tree-parked-handle",
                                    self.tree_resize_handler(cx),
                                    self.tree_resize_state.clone(),
                                ))
                            }),
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
        let draft_overlay = self.open_review.dock().is_some().then(|| {
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
            // The window's one full-width tint. The tree adds a bounded
            // backing for labels; the stage does not compound the material.
            .bg(theme::software_palette().surface.window_backing)
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
            .child(pane::slot(
                &self.pane,
                self.pane_bounds.clone(),
                self.comment_pane_freeze,
            ))
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
                .open_review
                .review()
                .comments_for_path(&s.selected_path)
                .count();
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
            let publication = match view.open_review.publication_eligibility() {
                Ok(()) => "GitLab MR · publication available",
                Err(_)
                    if view.open_review.storage_error().is_none()
                        && view
                            .open_review
                            .origin()
                            .preparation_eligibility(&view.open_review.review().comparison)
                            .is_ok() =>
                {
                    "GitLab MR · version details checked on publish"
                }
                Err(reason) => reason,
            };
            sub = format!("{sub} · {publication}");
            if let Some(error) = view.open_review.storage_error() {
                sub = format!("{sub} · {error}");
            }
            if let Some(copy) = &view.hover_copy {
                sub = format!("{sub} · {copy}");
            }
            (s.selected_path.clone(), sub)
        }
        None => ("—".into(), "No file selected".into()),
    }
}

/// Rides transparently on the stage under the island: the file being read and where in it.
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
        .text_color(theme::software_palette().text.secondary)
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

/// The status bar's space below the island. It replaces the stage's bottom inset,
/// so the text sits centred between the island and the window edge.
const STATUS_BAR_HEIGHT: f32 = 28.;

fn render_titlebar(
    view: &DiffView,
    window: &mut Window,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    // Leading zone spans exactly what sits left of the stage, so what follows starts
    // on the stage's left edge and lines up with the island below.
    let leading_w = if view.tree_collapsed {
        px(collapsed_leading_width())
    } else {
        px(view.tree_width + theme::CHANGES_SHADOW_GAP)
    };
    div()
        .id("diff-titlebar")
        .map(|bar| super::titlebar::app_owned(bar, window))
        .h(theme::TITLEBAR_HEIGHT)
        .bg(theme::software_palette().surface.titlebar_backing)
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
                        .map(|region| {
                            super::titlebar::drag_region(
                                region,
                                window,
                                cx,
                                "diff-titlebar-drag-leading",
                            )
                        }),
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
                // Tree open: the rail seam is the left gap, so the toolbar starts
                // on the stage edge and shares it with the island. Collapsed: the
                // island's own left inset, measured from the stage edge.
                .when(view.tree_collapsed, |d| d.pl(px(theme::CHANGES_INSET)))
                .child(render_nav_capsule(view, window, cx))
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
                .child(
                    div()
                        .id("publish-all-drafts")
                        .px_2()
                        .cursor_pointer()
                        .opacity(if view.can_publish_batch() || view.active_batch.is_some() {
                            1.0
                        } else {
                            0.4
                        })
                        .child(if view.active_batch.is_some() {
                            "Cancel batch".to_string()
                        } else {
                            format!("Publish all drafts ({})", view.batch_draft_ids().len())
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.active_batch.is_some() {
                                this.cancel_batch(cx);
                            } else {
                                this.publish_all(cx);
                            }
                        })),
                )
                .children(
                    view.batch_status
                        .as_ref()
                        .map(|status| div().text_xs().child(status.clone())),
                )
                .children(view.export_status.as_ref().map(|status| {
                    div()
                        .flex_none()
                        .ui_text_size(12., cx)
                        .text_color(theme::software_palette().text.titlebar_link)
                        .child(status.clone())
                }))
                .child(
                    div()
                        .id("diff-titlebar-drag")
                        .h_full()
                        .flex_1()
                        .min_w(px(0.))
                        .window_control_area(WindowControlArea::Drag)
                        .map(|region| {
                            super::titlebar::drag_region(region, window, cx, "diff-titlebar-drag")
                        }),
                )
                .child(comments_toggle_button(view, window, cx))
                .child(
                    // Doubles as the trailing inset when no caption buttons follow.
                    div()
                        .id("diff-titlebar-drag-trailing")
                        .h_full()
                        .w(px(theme::CHANGES_INSET))
                        .flex_none()
                        .window_control_area(WindowControlArea::Drag)
                        .map(|region| {
                            super::titlebar::drag_region(
                                region,
                                window,
                                cx,
                                "diff-titlebar-drag-trailing",
                            )
                        }),
                ),
        )
        // Outside the zones' padding: close must land in the physical corner.
        .children(super::window_controls::window_controls(window))
}

fn render_tree_pane(
    view: &DiffView,
    width: gpui::Pixels,
    window: &Window,
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
    let rows = file_tree::flatten_query(&paths, &view.collapsed_dirs, &view.tree_query);
    let filter_focused = view
        .tree_filter
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    div()
        .id("diff-tree")
        .h_full()
        .w(width)
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        // Light appearance exposes the native frosted material; dark appearance
        // adds a bounded tint behind ChangedPath labels.
        .bg(theme::software_palette().tree.desk.backing)
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .pt_1()
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .h(px(theme::FIND_FIELD_HEIGHT))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(gpui::Rgba {
                            a: if filter_focused { 1. } else { 0. },
                            ..theme::software_palette().field.focused_border
                        })
                        .bg(theme::software_palette().search.tree_filter_surface)
                        .child(
                            svg()
                                .ml_2()
                                .size(theme::ICON_SIZE_SM)
                                .flex_none()
                                .path("search.svg")
                                .text_color(theme::software_palette().text.secondary),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .h_full()
                                .child(view.tree_filter.clone()),
                        )
                        .when(!view.tree_query.is_empty(), |row| {
                            row.child(
                                div()
                                    .id("diff-tree-filter-clear")
                                    .mr_1()
                                    .size(px(18.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::software_palette().control.hover))
                                    .tooltip(Tooltip::text("Clear", None))
                                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                                        window.prevent_default();
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.tree_filter.update(cx, |field, cx| {
                                            field.set_content("", cx);
                                        });
                                        let handle = this.tree_filter.read(cx).focus_handle(cx);
                                        window.focus(&handle);
                                    }))
                                    .child(
                                        svg()
                                            .size(px(12.))
                                            .flex_none()
                                            .path("close.svg")
                                            .text_color(theme::software_palette().text.secondary),
                                    ),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            IconButton::new(
                                "diff-tree-collapse",
                                "fold_vertical.svg",
                                "Collapse All",
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.collapse_all_dirs();
                                cx.notify();
                            })),
                        )
                        .child(
                            IconButton::new(
                                "diff-tree-expand",
                                "unfold_vertical.svg",
                                "Expand All",
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.collapsed_dirs.clear();
                                cx.notify();
                            })),
                        ),
                ),
        )
        .child({
            let (scroll, sb) = scrollbar::vertical("diff-tree-sb", cx);
            let pane_width = f32::from(width);
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
                                pane_width,
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
                                pane_width,
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
        .bg(theme::software_palette().surface.island)
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
    let palette = theme::software_palette();
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
        .bg(theme::software_palette().surface.floating_overlay)
        .shadow(theme::find_bar_shadow())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(theme::FIND_FIELD_HEIGHT))
                .flex()
                .items_center()
                .rounded(px(theme::FIND_FIELD_RADIUS))
                .bg(palette.field.surface)
                .border_1()
                .border_color(if focused {
                    palette.field.focused_border
                } else {
                    palette.search.idle_border
                })
                .child(view.search_field.clone()),
        )
        .child(render_side_segment(side, cx))
        .child(render_files_segment(files, cx))
        .child(
            IconButton::new("search-run", "search.svg", "Find")
                .when_some(palette.search.control_surface, |button, color| {
                    button.background(color)
                })
                .shortcut("Enter")
                .disabled(total == 0)
                .on_click(cx.listener(|this, _, _, cx| this.jump_search(1, cx))),
        )
        .child(
            div()
                .flex_none()
                .ui_text_size(12., cx)
                .text_color(palette.text.secondary)
                .child(count),
        )
        .child(
            IconButton::new("search-prev", "chevron_up.svg", "Previous Match")
                .when_some(palette.search.control_surface, |button, color| {
                    button.background(color)
                })
                .shortcut("Shift-Enter")
                .disabled(total == 0)
                .on_click(cx.listener(|this, _, _, cx| this.jump_search(-1, cx))),
        )
        .child(
            IconButton::new("search-next", "chevron_down.svg", "Next Match")
                .when_some(palette.search.control_surface, |button, color| {
                    button.background(color)
                })
                .shortcut("Enter")
                .disabled(total == 0)
                .on_click(cx.listener(|this, _, _, cx| this.jump_search(1, cx))),
        )
        .child(
            IconButton::new("search-close", "close.svg", "Close Find")
                .when_some(palette.search.control_surface, |button, color| {
                    button.background(color)
                })
                .shortcut("Esc")
                .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
        )
        .into_any_element()
}

/// Find Side/Files track: same height and rounded inset as [`capsule`].
fn search_seg_track() -> Div {
    let palette = theme::software_palette();
    div()
        .flex_none()
        .h(theme::TOGGLE_SIZE)
        .flex()
        .items_center()
        .p(px(NAV_INSET))
        .gap(px(NAV_INSET))
        .rounded(px(NAV_BUTTON_RADIUS + NAV_INSET))
        .bg(palette
            .search
            .control_surface
            .unwrap_or(palette.field.surface))
}

fn search_seg_btn(
    id: &'static str,
    label: &'static str,
    selected: bool,
    cx: &mut Context<DiffView>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let palette = theme::software_palette();
    div()
        .id(id)
        .h_full()
        .px(px(6.))
        .rounded(px(NAV_BUTTON_RADIUS))
        .flex()
        .items_center()
        .cursor_pointer()
        .when(selected, |d| d.bg(palette.control.selected))
        .when(!selected || palette.search.control_surface.is_some(), |d| {
            d.hover(move |d| {
                d.bg(if selected {
                    palette.control.selected_hover
                } else {
                    palette.control.hover
                })
            })
        })
        .when(palette.search.control_surface.is_some(), |d| {
            d.active(move |d| {
                d.bg(if selected {
                    palette.control.selected_pressed
                } else {
                    palette.control.pressed
                })
            })
        })
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
                    theme::software_palette().text.link
                } else {
                    palette.text.secondary
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

/// How long the comment drawer takes to slide. The pane stays frozen for the
/// same span, then reflows once the edges have arrived.
const COMMENT_DRAWER_ANIM: Duration = Duration::from_millis(220);
/// Leftward travel on the parked handle that opens the island like the button.
const COMMENT_REVEAL_NUDGE: f32 = 8.;

/// Gap handle + comment island as a drawer. Width runs 0 ↔ gap + island.
/// Content stays left-aligned at the full width and the right side clips, so
/// the island's left edge stays one gap from the diff card's right edge.
fn render_comment_drawer(
    view: &DiffView,
    room: f32,
    layout: splitter::Collapse,
    cx: &mut Context<DiffView>,
) -> Option<AnyElement> {
    let (width, shown) = match (layout, view.comment_fit(room)) {
        (splitter::Collapse::Width(width), _) => (width, true),
        (splitter::Collapse::Hidden, splitter::Collapse::Width(width)) => (width, false),
        (splitter::Collapse::Hidden, splitter::Collapse::Hidden) => {
            (view.comment_width.min(room.max(0.)), false)
        }
    };
    let full = theme::CHANGES_SHADOW_GAP + width;
    let gap = if shown {
        splitter::handle(
            "diff-comment-resize-handle",
            Axis::HorizontalTrailing,
            view.comment_resize_handler(cx),
            view.comment_resize_state.clone(),
            true,
        )
        .into_any_element()
    } else {
        // Sliding shut: the parked handle owns the drag, so this strip must not.
        div()
            .flex_none()
            .w(px(theme::CHANGES_SHADOW_GAP))
            .into_any_element()
    };
    let drawer = div().flex_none().h_full().overflow_hidden().child(
        div()
            .w(px(full))
            .h_full()
            .flex()
            .child(gap)
            .child(render_comment_island(view, width, cx)),
    );
    match view.comment_anim_gen {
        Some(generation) => Some(
            drawer
                .with_animation(
                    ("diff-comment-drawer", generation),
                    Animation::new(COMMENT_DRAWER_ANIM).with_easing(ease_out_quint()),
                    move |this, t| this.w(px(full * if shown { t } else { 1. - t })),
                )
                .into_any_element(),
        ),
        None => shown.then(|| drawer.w(px(full)).into_any_element()),
    }
}

/// Right-hand comment island. Row body selects the span (wash + selected
/// styling); Edit / gutter icons open the dock; Delete removes immediately.
fn render_comment_island(
    view: &DiffView,
    width: f32,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let mono = appearance::code_font(cx);
    let path = view
        .snapshot
        .as_ref()
        .map(|s| s.selected_path.clone())
        .unwrap_or_default();
    let comments: Vec<_> = view
        .open_review
        .review()
        .comments_for_path(&path)
        .cloned()
        .collect();

    let n = comments.len();
    let selection = view.pane.read(cx).selection();
    let (scroll, sb) = scrollbar::vertical("diff-comments-sb", cx);

    div()
        .id("diff-comment-island")
        .flex_none()
        .w(px(width))
        .h_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(theme::software_palette().surface.island)
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
                        .text_color(theme::software_palette().text.section)
                        .child("COMMENTS"),
                )
                .child(div().id("refresh-publications").ui_text_size(11.,cx).text_color(theme::software_palette().text.link).cursor_pointer().child("Refresh GitLab").on_click(cx.listener(|this,_,_,cx|this.refresh_publications(cx))))
                .when(n > 0, |header| {
                    header.child(
                        div()
                            .ui_text_size(11., cx)
                            .text_color(theme::software_palette().text.secondary)
                            .child(format!("{n}")),
                    )
                }),
        )
        .child(if n == 0 {
            render_no_comments(cx).into_any_element()
        } else {
            scrollbar::overlay_flex(
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
                        let palette = theme::software_palette();
                        div()
                            .id(("cmt", c.id as usize))
                            .mx_1()
                            .my_0p5()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .cursor_pointer()
                            .when(selected, |row| row.bg(palette.control.selected))
                            .hover(move |row| {
                                row.bg(if selected {
                                    palette.control.selected_hover
                                } else {
                                    palette.control.hover
                                })
                            })
                            .active(move |row| {
                                row.bg(if selected {
                                    palette.control.selected_pressed
                                } else {
                                    palette.control.pressed
                                })
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
                                            .text_color(theme::software_palette().text.primary)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(label),
                                    )
                                    .child(
                                        div()
                                            .id(("cmt-publish", id as usize))
                                            .ui_text_size(11., cx)
                                            .child(if view.publication_records.get(&id).is_some_and(|r| matches!(r.state, PublicationState::Failed(_))) { "Retry publish" } else { "Publish" })
                                            .when(view.can_publish(id), |button| button.cursor_pointer().text_color(palette.text.link).on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.publish_comment(id, cx);
                                            })))
                                            .when(!view.can_publish(id), |button| button.text_color(palette.text.disabled)),
                                    )
                                    .child(
                                        IconButton::new(
                                            ("cmt-edit", id as usize),
                                            "pencil.svg",
                                            "Edit DraftComment",
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.begin_edit(id, window, cx);
                                            }),
                                        ),
                                    )
                                    .child(
                                        IconButton::new(
                                            ("cmt-del", id as usize),
                                            "trash.svg",
                                            "Delete DraftComment",
                                        )
                                        .disabled(!view.can_delete_comment(id))
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.delete_comment(id, window, cx);
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .ui_text_size(12., cx)
                                    .text_color(theme::software_palette().text.primary)
                                    .child(c.body),
                            )
                            .child(div().mt(px(4.)).ui_text_size(11., cx).text_color(palette.text.secondary).child(
                                if view.publication_in_progress(id) { "Syncing with GitLab…".into() }
                                else if let Some(error) = &view.publication_error { error.clone() }
                                else if let Some(record) = view.publication_records.get(&id) { record.status_label() }
                                else if view.publication_protected.contains(&id) { "Published or awaiting confirmation on another MR".into() }
                                else if let Err(reason) = view.open_review.origin().preparation_eligibility(&view.open_review.review().comparison) { reason.into() }
                                else if !matches!(&c.anchor, Anchor::Line { span, .. } if span.count>0) { "File comment publication unavailable".into() }
                                else { "Local draft · publish explicitly".into() }
                            ))
                            .child(render_publication_recovery(view, id, cx))
                            .when(view.publication_records.get(&id).and_then(|record|record.receipt()).is_some(),|row| {
                                let receipt=view.publication_records.get(&id).and_then(|record|record.receipt()).unwrap();
                                let label=receipt.placement_label();
                                row.child(div().ui_text_size(11.,cx).text_color(palette.text.secondary).child(label))
                            })
                            .when(view.publication_records.get(&id).is_some_and(|record|record.deletion.is_some()),|row| {
                                let record=view.publication_records.get(&id).unwrap();
                                match record.deletion.as_ref().unwrap() {
                                    DeleteStatus::Conflict {website_body} if !record.website_deleted => row
                                        .child(div().mt(px(6.)).ui_text_size(11.,cx).child("Website body:").child(div().child(website_body.clone())))
                                        .child(div().id(("confirm-delete",id as usize)).cursor_pointer().ui_text_size(11.,cx).text_color(palette.text.link).child("Delete this website comment").on_click(cx.listener(move|this,_,_,cx|{cx.stop_propagation();this.retry_delete(id,true,cx);}))),
                                    DeleteStatus::Pending | DeleteStatus::Failed(_) if record.receipt().is_some() && !record.website_deleted => row
                                        .child(div().id(("retry-delete",id as usize)).cursor_pointer().ui_text_size(11.,cx).text_color(palette.text.link).child("Retry delete").on_click(cx.listener(move|this,_,_,cx|{cx.stop_propagation();this.retry_delete(id,false,cx);}))),
                                    _ => row,
                                }
                            })
                            .when(view.publication_records.get(&id).is_some_and(|record|record.edit.is_some() && record.deletion.is_none()),|row| {
                                let record=view.publication_records.get(&id).unwrap();
                                match &record.edit.as_ref().unwrap().status {
                                    EditStatus::Conflict {website_body}=>row.child(div().mt(px(6.)).ui_text_size(11.,cx).child("Website body:").child(div().child(website_body.clone())))
                                        .child(div().id(("adopt-website",id as usize)).cursor_pointer().ui_text_size(11.,cx).text_color(palette.text.link).child("Adopt website body").on_click(cx.listener(move|this,_,window,cx|{cx.stop_propagation();this.adopt_website_body(id,window,cx);})))
                                        .child(div().id(("overwrite-website",id as usize)).cursor_pointer().ui_text_size(11.,cx).text_color(palette.text.link).child("Overwrite with local body").on_click(cx.listener(move|this,_,_,cx|{cx.stop_propagation();this.update_comment(id,true,cx);}))),
                                    EditStatus::Pending|EditStatus::Failed(_)=>row.child(div().id(("retry-update",id as usize)).cursor_pointer().ui_text_size(11.,cx).text_color(palette.text.link).child("Retry update").on_click(cx.listener(move|this,_,_,cx|{cx.stop_propagation();this.update_comment(id,false,cx);}))),
                                    _=>row,
                                }
                            })
                    })),
                sb,
            )
            .into_any_element()
        })
        .into_any_element()
}

fn render_publication_recovery(view: &DiffView, id: u64, cx: &mut Context<DiffView>) -> AnyElement {
    let Some(record) = view.publication_records.get(&id) else {
        return div().into_any_element();
    };
    let uncertain = matches!(record.state, PublicationState::Unknown(_))
        || matches!(
            record.deletion,
            Some(DeleteStatus::Unknown(_) | DeleteStatus::Sending)
        )
        || record.edit.as_ref().is_some_and(|edit| {
            matches!(
                edit.status,
                EditStatus::Unknown { .. } | EditStatus::Sending { .. }
            )
        });
    let key = PublicationKey::new(
        view.open_review.review().comparison.clone(),
        view.open_review.origin().clone(),
        id,
    );
    let palette = theme::software_palette();
    let mut controls = div().flex().flex_col().gap_1().ui_text_size(11., cx);
    if uncertain {
        controls = controls
            .child(
                div()
                    .id(("check-publication", id as usize))
                    .child("Check again")
                    .text_color(if view.can_check_comment(id) {
                        palette.text.link
                    } else {
                        palette.text.disabled
                    })
                    .when(view.can_check_comment(id), |button| {
                        button
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.check_comment(id, cx);
                            }))
                    }),
            )
            .child(div().child(
                "Checking only reads GitLab; any further write requires an explicit action.",
            ));
    }
    if let Some(url) = view
        .open_review
        .origin()
        .web_url(record.receipt().map(|receipt| receipt.note_id))
    {
        controls = controls.child(
            div()
                .id(("view-publication", id as usize))
                .cursor_pointer()
                .text_color(palette.text.link)
                .child("View on GitLab")
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&url);
                }),
        );
    }
    for (index, candidate) in record
        .recovery_candidates
        .iter()
        .enumerate()
        .filter(|_| record.receipt().is_none())
    {
        if let Some(url) = view.open_review.origin().web_url(Some(candidate.note_id)) {
            controls = controls.child(
                div()
                    .id(gpui::ElementId::Name(
                        format!("candidate-{id}-{index}").into(),
                    ))
                    .cursor_pointer()
                    .text_color(palette.text.link)
                    .child(format!("Candidate comment {}", candidate.note_id))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.open_url(&url);
                    }),
            );
        }
    }
    for attempt in &record.prior_attempts {
        controls = controls.child(div().child(format!(
            "Earlier uncertain publication attempt ({} candidates)",
            attempt.candidates.len()
        )));
        for candidate in &attempt.candidates {
            if let Some(url) = view.open_review.origin().web_url(Some(candidate.note_id)) {
                controls = controls.child(
                    div()
                        .cursor_pointer()
                        .text_color(palette.text.link)
                        .child(format!("Earlier candidate {}", candidate.note_id))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            cx.open_url(&url);
                        }),
                );
            }
        }
    }
    if let Some(error) = view.check_errors.get(&key) {
        controls = controls.child(div().child(error.clone()));
    }
    if view.can_keep_uncertain_comment(id) {
        controls = controls.child(div().child("A deletion is pending, but no unique remote comment has been identified."))
            .child(div().id(("keep-uncertain-comment", id as usize)).cursor_pointer().text_color(palette.text.link)
                .child("Keep comment and cancel unsent deletion")
                .on_click(cx.listener(move|this,_,_,cx| {cx.stop_propagation();this.keep_uncertain_comment(id,cx);})))
            .child(div().child("The create result stays unknown. Republishing still requires duplicate-risk confirmation."));
    }
    if view.republish_confirmations.get(&key) == Some(&record.operation_id)
        && view.can_republish_comment(id)
    {
        controls = controls
            .child(div().child(
                "The original attempt may already exist. Publishing again can create a duplicate.",
            ))
            .child(
                div()
                    .id(("confirm-republish", id as usize))
                    .cursor_pointer()
                    .text_color(palette.text.link)
                    .child("Publish again despite duplicate risk")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.confirm_republish(id, cx);
                    })),
            )
            .child(
                div()
                    .id(("cancel-republish", id as usize))
                    .cursor_pointer()
                    .child("Cancel")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let key = PublicationKey::new(
                            this.open_review.review().comparison.clone(),
                            this.open_review.origin().clone(),
                            id,
                        );
                        this.republish_confirmations.remove(&key);
                        cx.notify();
                    })),
            );
    } else if view.can_republish_comment(id) {
        controls = controls.child(
            div()
                .id(("request-republish", id as usize))
                .cursor_pointer()
                .text_color(palette.text.link)
                .child("Republish…")
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.request_republish(id, cx);
                })),
        );
    }
    controls.into_any_element()
}

fn render_no_comments(cx: &App) -> impl IntoElement {
    div()
        .px(px(12.))
        .pt(px(4.))
        .flex()
        .flex_col()
        .gap_1()
        .text_color(theme::software_palette().text.secondary)
        .child(div().ui_text_size(12., cx).child("No comments"))
        .child(
            div()
                .ui_text_size(11., cx)
                .child("Select lines, then click the gutter icon"),
        )
}

/// Invisible slot where the bottom draft dock will be painted above DualPane.
/// Bottom-anchored mirror of [`render_find_measure`].
fn render_draft_measure(view: &DiffView, cx: &App) -> impl IntoElement {
    if view.open_review.dock().is_none() {
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
    let palette = theme::software_palette();
    let Some(draft) = view.open_review.dock() else {
        return div().into_any_element();
    };
    let label = span_label(draft.side, draft.span.start, draft.span.count);
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
        .bg(theme::software_palette().surface.floating_overlay)
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
                        .text_color(palette.text.secondary)
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
                        .text_color(palette.text.primary)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .ui_text_size(11., cx)
                        .text_color(palette.text.secondary)
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
                .bg(palette.field.surface)
                .border_1()
                .border_color(gpui::Rgba {
                    a: if focused { 1. } else { 0. },
                    ..palette.field.focused_border
                })
                .child(view.draft_field.clone()),
        )
        .into_any_element()
}

/// Inset between the nav capsule's edge and its buttons.
const NAV_INSET: f32 = 2.;
const NAV_BUTTON_RADIUS: f32 = 5.;

/// Reserve the widest index with as many digits as `total`, independent of
/// the current position. Measure the UI face so proportional fonts work too.
fn nav_counter(text: String, total: usize, window: &Window, cx: &App) -> Div {
    let size = appearance::ui_text(cx, 12.);
    let face = gpui::font(appearance::ui_font(cx));
    let measure = |text: String| {
        let text: SharedString = text.into();
        let run = gpui::TextRun {
            len: text.len(),
            font: face.clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text, size, &[run], None)
            .width
    };
    let dash_width = measure("—".into());
    let width = if total == 0 {
        dash_width
    } else {
        let total_text = total.to_string();
        let digit_width = ('0'..='9')
            .map(|digit| measure(digit.to_string()))
            .fold(px(0.), |width, next| width.max(next));
        (digit_width * total_text.len() as f32).max(dash_width) + measure(format!("/{total_text}"))
    };
    div()
        .flex_none()
        .w(px(f32::from(width).ceil()))
        .whitespace_nowrap()
        .text_right()
        .text_color(theme::software_palette().text.primary)
        .child(text)
}

/// `« ‹ File 3/12 · Hunk 2/5 › »`: outer chevrons step files, inner ones hunks.
/// Sized to `TOGGLE_SIZE` so it sits in the toolbar like any other button.
fn render_nav_capsule(
    view: &DiffView,
    window: &Window,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let (index, total) = view.file_position();
    let file = match index {
        Some(i) => format!("{}/{total}", i + 1),
        None if total == 0 => "—".to_string(),
        None => format!("—/{total}"),
    };
    let prev_file = match index {
        Some(i) => i > 0,
        None => total > 0,
    };
    let next_file = match index {
        Some(i) => i + 1 < total,
        None => total > 0,
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
            prev_file,
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
                .child(
                    div()
                        .text_color(theme::software_palette().text.secondary)
                        .child("File"),
                )
                .child(nav_counter(file, total, window, cx))
                .child(
                    div()
                        .text_color(theme::software_palette().text.secondary)
                        .child("·"),
                )
                .child(
                    div()
                        .text_color(theme::software_palette().text.secondary)
                        .child("Hunk"),
                )
                .child(nav_counter(hunk, hunk_count, window, cx)),
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
            next_file,
            false,
            cx.listener(|this, _, _, cx| this.jump_file(1, cx)),
        ))
}

/// Group of toolbar buttons, `TOGGLE_SIZE` tall like any other button.
fn capsule() -> Div {
    div()
        .map(super::titlebar::consume_control_mouse_events)
        .flex_none()
        .h(theme::TOGGLE_SIZE)
        .flex()
        .items_center()
        .p(px(NAV_INSET))
        .gap(px(NAV_INSET))
        // Concentric with the buttons' hover: button radius + inset.
        .rounded(px(NAV_BUTTON_RADIUS + NAV_INSET))
        .bg(theme::software_palette().surface.island)
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
        theme::software_palette().text.disabled
    } else if pressed {
        theme::software_palette().text.link
    } else {
        theme::software_palette().text.secondary
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
        .when(pressed, |button| {
            button.bg(theme::software_palette().control.selected)
        })
        .tooltip(Tooltip::text(tooltip, shortcut))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|button| button.bg(theme::software_palette().control.hover))
                .active(|button| button.bg(theme::software_palette().control.pressed))
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
                    theme::software_palette().text.secondary
                } else {
                    theme::software_palette().text.link
                })
                .tooltip(Tooltip::text("Reset Text Size", Some(tooltip::cmd("0"))))
                .hover(|button| button.bg(theme::software_palette().control.hover))
                .active(|button| button.bg(theme::software_palette().control.pressed))
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
            window_geometry_store::set_tree_collapsed(this.tree_collapsed);
            cx.notify();
        }))
}

/// Highlighted while the island is closed, same as the file-tree toggle.
fn comments_toggle_button(
    view: &DiffView,
    window: &Window,
    cx: &mut Context<DiffView>,
) -> impl IntoElement {
    let shown = view.comment_layout(view.comment_room(window)) != splitter::Collapse::Hidden;
    let label = if shown {
        "Hide Comments"
    } else {
        "Show Comments"
    };
    IconButton::new("diff-comments-toggle", "sidebar_right.svg", label)
        .pressed(!shown)
        .on_click(cx.listener(|this, _, window, cx| this.toggle_comments(window, cx)))
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

    #[gpui::test]
    fn titlebar_capsule_consumes_clicks_on_buttons_and_padding(cx: &mut gpui::TestAppContext) {
        use gpui::IntoElement;
        for (enabled, position, activates) in [
            (true, gpui::point(gpui::px(10.), gpui::px(10.)), true),
            (false, gpui::point(gpui::px(10.), gpui::px(10.)), false),
            (true, gpui::point(gpui::px(1.), gpui::px(1.)), false),
        ] {
            super::super::titlebar::tests::assert_consumes_clicks(
                cx,
                move |clicks| {
                    use gpui::ParentElement;
                    super::capsule()
                        .child(super::nav_button(
                            "test-nav",
                            "chevron_right.svg",
                            "Next Hunk",
                            None,
                            enabled,
                            false,
                            move |_, _, _| clicks.set(clicks.get() + 1),
                        ))
                        .into_any_element()
                },
                position,
                activates,
            );
        }
    }

    #[gpui::test]
    fn opening_a_draft_on_visible_selected_lines_keeps_code_in_place(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::domain::{Alignment, AlignmentOp, Comparison, Oid, Repository};
        use crate::ui::appearance;
        use gpui::AppContext;

        cx.update(|cx| cx.set_global(appearance::resolve(&Default::default(), &[])));
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let text: std::sync::Arc<str> = (1..=100)
                .map(|i| format!("line {i}\n"))
                .collect::<String>()
                .into();
            let span = LineSpan {
                start: 1,
                count: 100,
            };
            let view = cx.new(|cx| {
                super::DiffView::with_snapshot(
                    super::DiffSnapshot {
                        origin: crate::publication::ReviewOrigin::Local,
                        comparison: Comparison {
                            repository: Repository::new("/tmp/diff-draft-test".into()),
                            base_oid: None,
                            head_oid: Oid::from_bytes([1; 20]),
                            uncommitted: false,
                        },
                        changed_paths: Vec::new(),
                        selected_path: "sample.txt".into(),
                        file: crate::git::FileDiff::Text {
                            alignment: Alignment {
                                ops: vec![AlignmentOp::Equal {
                                    preimage: span,
                                    postimage: span,
                                }],
                            },
                            preimage_text: text.clone(),
                            postimage_text: text,
                        },
                    },
                    window,
                    cx,
                )
            });
            view.update(cx, |view, cx| {
                view.with_pane(cx, |pane, cx| {
                    pane.set_soft_wrap(false, cx);
                    pane.expand_all(cx);
                });
                // At several viewport positions, select visible lines away from
                // the one-third navigation anchor, just as a gutter drag does.
                for (side, scroll, start) in [
                    (Side::Preimage, 200., 4),
                    (Side::Postimage, 600., 28),
                    (Side::Preimage, 600., 30),
                    (Side::Postimage, 1000., 48),
                ] {
                    let before = view.with_pane(cx, |pane, cx| {
                        pane.select_span(side, start, start + 2, cx);
                        pane.test_viewport_tops(600., Some(scroll))
                    });
                    view.begin_draft(side, start, 3, window, cx);
                    let dock = view.open_review.dock().expect("draft dock is open");
                    assert_eq!(dock.side, side);
                    assert_eq!(dock.span, LineSpan { start, count: 3 });
                    let after = view.with_pane(cx, |pane, _| pane.test_viewport_tops(600., None));
                    assert_eq!(
                        after, before,
                        "clicking comment on L{start} must keep both code panes still"
                    );
                }
            });
        });
    }

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

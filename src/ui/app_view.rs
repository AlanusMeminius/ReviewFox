use gpui::{
    Animation, AnimationExt, App, Bounds, ClickEvent, ClipboardItem, Context, Corner, Div,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, Render, Size, StatefulInteractiveElement, Styled,
    TitlebarOptions, Transformation, Window, WindowBounds, WindowControlArea, WindowDecorations,
    WindowHandle, WindowOptions, anchored, canvas, deferred, div, ease_out_quint, percentage,
    prelude::*, px, rgb, svg,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use super::OpenSettings;
use super::appearance::{self, UiTextSize};
use super::diff_window::{DiffSnapshot, DiffView};
use super::entry_chrome::{self, EntryKind, KindSwitchAction, OpenEntry, RestoreFailureAction};
use super::file_tree::{self, TreeRow};
use super::file_tree_rows::{self, RowSurface};
use super::gitlab_connection::{self, GitLabConnection};
use super::icon_button::IconButton;
#[cfg(target_os = "macos")]
use super::mac_column_vibrancy::ColumnVibrancy;
use super::metadata;
use super::scrollbar;
use super::selectable_markdown;
use super::settings;
use super::splitter::{self, Axis, ResizeState};
use super::theme;
use super::window_controls::window_controls;
use super::window_geometry;
use crate::domain::{ChangedPath, Comparison, Oid, PathStatus, Repository};
use crate::git::{self, BranchBrowser, BranchInfo, CommitInfo};
use crate::gitlab::{
    self, ListMergeRequestsResult, MergeRequestDetail, MergeRequestSummary, ResolveProjectResult,
    SettingsTarget,
};
use crate::loaded_browser::LoadedBrowser;
use crate::mr_entry::{self, GitLabForge, SystemGitObjects};
use crate::settings_store;
use crate::window_geometry_store::{self, DiffReopen};
use crate::workspace_store::{self, MrEntryLabel, WorkspaceEntry, WorkspaceStore};

pub struct AppView {
    focus: FocusHandle,
    repos_collapsed: bool,
    state: MainState,
    /// Workspace set + last + pinned (mirrors disk).
    store: WorkspaceStore,
    diff_window: Option<WindowHandle<DiffView>>,
    branch_picker: Option<BranchPicker>,
    mr_picker: Option<MrPicker>,
    /// Live window bounds of the chrome capsules (updated each frame via canvas).
    branch_toggle_bounds: Rc<Cell<Bounds<Pixels>>>,
    mr_toggle_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// In-memory MR Entry (list = GitLab commits; Comparison = diff_refs).
    mr_entry: Option<MrEntry>,
    /// Bumped on every scan start and every leave, so a late scan cannot repaint.
    uncommitted_generation: u64,
    /// Path scan is in flight. An empty list is not yet "clean checkout".
    uncommitted_paths_pending: bool,
    /// Refresh scan failed. The previous path list stays on the island.
    uncommitted_scan_error: Option<String>,
    /// MR kind held with no selected Entry (`Select MR…`); Comparison stays on Branch Browser.
    empty_mr: bool,
    /// Kind-switch restore in flight — failure clears to empty MR + picker.
    pending_kind_restore: bool,
    repo_menu: Option<RepoContextMenu>,
    commit_menu: Option<CommitContextMenu>,
    activation_sub: Option<gpui::Subscription>,
    bounds_sub: Option<gpui::Subscription>,
    /// Ephemeral; paths in set are collapsed. Default empty = all expanded.
    collapsed_dirs: HashSet<String>,
    /// Reset collapsed_dirs when this no longer matches current ChangedPath list.
    tree_path_fingerprint: Vec<String>,
    sidebar_width: f32,
    sidebar_resize_state: Rc<ResizeState>,
    files_width: f32,
    files_resize_state: Rc<ResizeState>,
    head_meta_height: f32,
    /// Once the user drags the tree↔head-meta splitter, stop auto-following 40%.
    head_meta_height_user_set: bool,
    head_meta_resize_state: Rc<ResizeState>,
    mr_detail_height: f32,
    /// Once the user drags the MR↔commit splitter, stop auto-following half height.
    mr_detail_height_user_set: bool,
    mr_detail_resize_state: Rc<ResizeState>,
    #[cfg(target_os = "macos")]
    window_vibrancy: Option<ColumnVibrancy>,
}

#[derive(Clone, Copy)]
enum RepoMenuKind {
    /// Unpin | Remove
    Pin,
    /// Pin | Remove
    Repositories,
}

struct RepoContextMenu {
    path: PathBuf,
    position: Point<Pixels>,
    kind: RepoMenuKind,
}

struct CommitContextMenu {
    position: Point<Pixels>,
}

enum MainState {
    Empty,
    Ready(LoadedBrowser),
    Error(String),
}

impl AppView {
    pub fn new(
        boot: Option<BranchBrowser>,
        gitlab_connection: Rc<RefCell<GitLabConnection>>,
        restore_diff: Option<DiffReopen>,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = match boot {
            Some(browser) => MainState::Ready(LoadedBrowser::install(browser)),
            None => MainState::Empty,
        };
        gitlab_connection::spawn_refresh_connection(gitlab_connection, cx.entity().downgrade(), cx);
        // ADR-0012: opening restores the selected Entry. Shell open/closed is app-global.
        let mut view = Self {
            focus: cx.focus_handle(),
            repos_collapsed: window_geometry_store::snapshot().repos_collapsed,
            state,
            store: workspace_store::load(),
            diff_window: None,
            branch_picker: None,
            mr_picker: None,
            branch_toggle_bounds: Rc::new(Cell::new(Bounds::default())),
            mr_toggle_bounds: Rc::new(Cell::new(Bounds::default())),
            mr_entry: None,
            uncommitted_generation: 0,
            uncommitted_paths_pending: false,
            uncommitted_scan_error: None,
            empty_mr: false,
            pending_kind_restore: false,
            repo_menu: None,
            commit_menu: None,
            activation_sub: None,
            bounds_sub: None,
            collapsed_dirs: HashSet::new(),
            tree_path_fingerprint: Vec::new(),
            sidebar_width: splitter::default_sidebar_width(),
            sidebar_resize_state: Rc::new(ResizeState::with_drag_latch()),
            files_width: splitter::default_files_width(),
            files_resize_state: Rc::new(ResizeState::default()),
            head_meta_height: splitter::MIN_HEAD_META_HEIGHT,
            head_meta_height_user_set: false,
            head_meta_resize_state: Rc::new(ResizeState::default()),
            mr_detail_height: splitter::DEFAULT_MR_DETAIL_HEIGHT,
            mr_detail_height_user_set: false,
            mr_detail_resize_state: Rc::new(ResizeState::default()),
            #[cfg(target_os = "macos")]
            window_vibrancy: None,
        };
        view.restore_selected_entry(cx);
        if let Some(reopen) = restore_diff {
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| {
                    this.try_restore_diff(reopen, cx);
                })
                .ok();
            })
            .detach();
        }
        view
    }

    fn try_restore_diff(&mut self, reopen: DiffReopen, cx: &mut Context<Self>) {
        let Some(snapshot) = rebuild_diff_snapshot(&reopen) else {
            return;
        };
        open_or_update_diff(&mut self.diff_window, snapshot, &mut **cx);
    }

    fn sidebar_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |raw, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                if this.sidebar_resize_state.drag_consumed() {
                    return;
                }
                let available = f32::from(window.viewport_size().width);
                let origin = this.sidebar_resize_state.drag_origin().unwrap_or(raw);
                match splitter::leading_rail_drag(
                    raw,
                    origin,
                    !this.repos_collapsed,
                    available,
                    splitter::MIN_COMMITS_WIDTH,
                ) {
                    splitter::RailDrag::Stay => {}
                    splitter::RailDrag::Reveal => {
                        this.sidebar_resize_state.consume_drag();
                        this.repos_collapsed = false;
                        window_geometry_store::set_repos_collapsed(false);
                        cx.notify();
                    }
                    splitter::RailDrag::Show(width) => {
                        let changed = this.repos_collapsed || this.sidebar_width != width;
                        if this.repos_collapsed {
                            window_geometry_store::set_repos_collapsed(false);
                        }
                        this.repos_collapsed = false;
                        this.sidebar_width = width;
                        if changed {
                            cx.notify();
                        }
                    }
                    splitter::RailDrag::Hide => {
                        if !this.repos_collapsed {
                            this.repos_collapsed = true;
                            window_geometry_store::set_repos_collapsed(true);
                            cx.notify();
                        }
                    }
                }
            })
            .ok();
        })
    }

    fn files_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |requested, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                let available = f32::from(window.viewport_size().width);
                let sidebar = if this.repos_collapsed {
                    0.
                } else {
                    this.sidebar_width
                };
                // HorizontalTrailing reports distance to viewport right; float is inset.
                let width = splitter::clamp_files_width(
                    requested - theme::CHANGES_INSET,
                    available,
                    sidebar,
                );
                if this.files_width != width {
                    this.files_width = width;
                    cx.notify();
                }
            })
            .ok();
        })
    }

    fn head_meta_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |height, _, cx: &mut App| {
            view.update(cx, |this, cx| {
                this.head_meta_height_user_set = true;
                if this.head_meta_height != height {
                    this.head_meta_height = height;
                    cx.notify();
                }
            })
            .ok();
        })
    }

    fn mr_detail_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |size, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                let available = mr_detail_column_available(window);
                let height =
                    splitter::clamp_mr_detail_height(size - mr_detail_chrome_offset(), available);
                this.mr_detail_height_user_set = true;
                if this.mr_detail_height != height {
                    this.mr_detail_height = height;
                    cx.notify();
                }
            })
            .ok();
        })
    }

    /// Until the user drags the splitter, keep MR detail at half the island column.
    fn sync_default_mr_detail_height(&mut self, window: &Window) {
        if self.mr_detail_height_user_set || self.mr_entry.is_none() || !gitlab_chrome_visible(self)
        {
            return;
        }
        let next = splitter::default_mr_detail_height(mr_detail_column_available(window));
        if (self.mr_detail_height - next).abs() > 0.5 {
            self.mr_detail_height = next;
        }
    }

    /// Until the user drags the splitter, keep head-meta at 40% of the Changes island.
    fn sync_default_head_meta_height(&mut self, window: &Window) {
        if self.head_meta_height_user_set {
            return;
        }
        let next = splitter::default_head_meta_height(changes_island_height(window));
        if (self.head_meta_height - next).abs() > 0.5 {
            self.head_meta_height = next;
        }
    }

    fn sync_collapsed_dirs(&mut self) {
        let next = match &self.state {
            MainState::Ready(loaded) => loaded
                .changed_paths()
                .iter()
                .map(|p| p.path.clone())
                .collect(),
            MainState::Empty | MainState::Error(_) => Vec::new(),
        };
        if next != self.tree_path_fingerprint {
            self.tree_path_fingerprint = next;
            self.collapsed_dirs.clear();
        }
    }

    fn refresh_store(&mut self) {
        self.store = workspace_store::load();
    }

    fn remember_current(&mut self) {
        let mr = self.current_mr_label();
        let saved = match &self.state {
            MainState::Ready(loaded) => Some((
                loaded.repository().path().to_path_buf(),
                loaded.branch_name().to_string(),
            )),
            MainState::Empty | MainState::Error(_) => None,
        };
        let Some((path, branch)) = saved else {
            return;
        };
        workspace_store::remember_with_mr(&path, &branch, mr);
        self.refresh_store();
    }

    fn current_mr_label(&self) -> Option<MrEntryLabel> {
        let entry = self.mr_entry.as_ref()?;
        let project = entry.project.clone()?;
        Some(MrEntryLabel {
            project,
            iid: entry.summary.iid,
        })
    }

    /// Clears the selected MR Entry label only; last-MR memory is preserved (ADR-0011).
    fn clear_selected_mr_label(&mut self) {
        let path = match &self.state {
            MainState::Ready(loaded) => loaded.repository().path().to_path_buf(),
            MainState::Empty | MainState::Error(_) => return,
        };
        workspace_store::clear_selected_mr(&path);
        self.refresh_store();
    }

    /// Restore MR Entry from a label.
    /// `from_kind_switch` uses the empty-MR picker on failure; opening a Workspace does not.
    fn begin_restore_mr(
        &mut self,
        label: MrEntryLabel,
        from_kind_switch: bool,
        cx: &mut Context<Self>,
    ) {
        if !gitlab_chrome_visible(self) {
            return;
        }
        let iid = label.iid;
        self.empty_mr = false;
        self.pending_kind_restore = from_kind_switch;
        self.mr_entry = Some(MrEntry {
            summary: MergeRequestSummary {
                iid,
                title: format!("!{iid}"),
                source_branch: String::new(),
                target_branch: String::new(),
            },
            detail: MrDetailState::Loading,
            project: Some(label.project),
        });
        self.spawn_mr_activate(iid, cx);
    }

    /// Selected `mr` label for the open Workspace. `last_mr` is not the selected Entry.
    fn stored_selected_mr(&self) -> Option<MrEntryLabel> {
        let MainState::Ready(loaded) = &self.state else {
            return None;
        };
        let path = loaded.repository().path();
        self.store
            .workspaces
            .iter()
            .find(|e| e.path == path)
            .and_then(|e| e.mr.clone())
    }

    /// Launch or choosing a Workspace: activate `mr`, or show Branch without writing the label back.
    fn restore_selected_entry(&mut self, cx: &mut Context<Self>) {
        let stored = self.stored_selected_mr();
        let open = entry_chrome::opening_workspace(stored.as_ref(), gitlab_chrome_visible(self));
        let OpenEntry::ActivateMr(label) = open.entry else {
            return;
        };
        let from_kind_switch = open.refetch_failure == RestoreFailureAction::EnterEmptyMrOpenPicker;
        self.begin_restore_mr(label, from_kind_switch, cx);
    }

    fn current_last_mr(&self) -> Option<MrEntryLabel> {
        let MainState::Ready(loaded) = &self.state else {
            return None;
        };
        let path = loaded.repository().path();
        self.store
            .workspaces
            .iter()
            .find(|e| e.path == path)
            .and_then(|e| e.last_mr.clone())
    }

    /// Open MR picker if closed (kind-switch empty enter / failed restore).
    fn ensure_mr_picker_open(&mut self, cx: &mut Context<Self>) {
        if self.mr_picker.is_none() {
            self.toggle_mr_picker(cx);
        }
    }

    fn clear_uncommitted(&mut self) {
        if let MainState::Ready(loaded) = &mut self.state {
            loaded.uncover();
        }
        self.uncommitted_paths_pending = false;
        self.uncommitted_scan_error = None;
        self.uncommitted_generation = self.uncommitted_generation.wrapping_add(1);
    }

    /// HEAD is read here so the kind switches immediately. The workdir scan runs
    /// off the UI thread; a large checkout otherwise freezes the window.
    fn spawn_uncommitted_paths(
        &mut self,
        repository: Repository,
        refresh: bool,
        cx: &mut Context<Self>,
    ) {
        self.uncommitted_generation = self.uncommitted_generation.wrapping_add(1);
        let generation = self.uncommitted_generation;
        self.uncommitted_paths_pending = true;
        self.uncommitted_scan_error = None;
        cx.spawn(async move |this, cx| {
            let scanned = cx
                .background_executor()
                .spawn(async move { git::load_uncommitted(&repository) })
                .await;
            this.update(cx, |this, cx| {
                if this.uncommitted_generation != generation {
                    return;
                }
                this.uncommitted_paths_pending = false;
                match scanned {
                    Ok(entry) => {
                        if let MainState::Ready(loaded) = &mut this.state {
                            loaded.replace_cover(entry);
                        }
                    }
                    Err(e) => {
                        if refresh {
                            this.uncommitted_scan_error = Some(e.0);
                        } else {
                            this.clear_uncommitted();
                            this.state = MainState::Error(e.0);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Hold empty MR kind (`Select MR…`) and open the picker.
    fn enter_empty_mr(&mut self, cx: &mut Context<Self>) {
        self.clear_uncommitted();
        self.branch_picker = None;
        self.mr_entry = None;
        self.empty_mr = true;
        self.pending_kind_restore = false;
        self.ensure_mr_picker_open(cx);
    }

    /// Hard-exclusive Entry kind switch (kind-track hits).
    fn select_entry_kind(&mut self, target: EntryKind, cx: &mut Context<Self>) {
        if target == EntryKind::Mr && !gitlab_chrome_visible(self) {
            return;
        }
        let current = entry_chrome::active_entry_kind(
            self.mr_entry.is_some(),
            self.empty_mr,
            self.covering(),
        );
        let last_mr = self.current_last_mr();
        match entry_chrome::kind_switch_action(current, target, last_mr.as_ref()) {
            KindSwitchAction::Stay => {}
            KindSwitchAction::SelectBranch => {
                self.clear_uncommitted();
                self.branch_picker = None;
                self.mr_picker = None;
                self.empty_mr = false;
                self.pending_kind_restore = false;
                self.clear_mr_entry(cx);
            }
            KindSwitchAction::SelectUncommitted => self.enter_uncommitted(cx),
            KindSwitchAction::RestoreMr(label) => {
                self.clear_uncommitted();
                self.branch_picker = None;
                self.mr_picker = None;
                self.begin_restore_mr(label, true, cx);
            }
            KindSwitchAction::EnterEmptyMr => {
                self.enter_empty_mr(cx);
            }
        }
        cx.notify();
    }

    fn enter_uncommitted(&mut self, cx: &mut Context<Self>) {
        let repository = match &self.state {
            MainState::Ready(loaded) => loaded.repository().clone(),
            MainState::Empty | MainState::Error(_) => return,
        };
        let shell = match git::uncommitted_head(&repository) {
            Ok(entry) => entry,
            Err(e) => {
                self.state = MainState::Error(e.0);
                return;
            }
        };
        self.branch_picker = None;
        self.mr_picker = None;
        if let MainState::Ready(loaded) = &mut self.state {
            loaded.cover(shell);
        }
        self.spawn_uncommitted_paths(repository, false, cx);
        if self.mr_entry.is_some() {
            self.clear_mr_entry(cx);
        } else {
            self.empty_mr = false;
            self.pending_kind_restore = false;
        }
    }

    fn refresh_uncommitted(&mut self, cx: &mut Context<Self>) {
        let repository = match &self.state {
            MainState::Ready(loaded) if loaded.covering() => loaded.repository().clone(),
            _ => return,
        };
        self.spawn_uncommitted_paths(repository, true, cx);
        cx.notify();
    }

    fn select_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let MainState::Ready(loaded) = &self.state {
            if loaded.repository().path() == path.as_path() {
                return;
            }
        }
        let (branch, mr, last_mr) = self
            .store
            .workspaces
            .iter()
            .find(|e| e.path == path)
            .map(|e| (e.branch.clone(), e.mr.clone(), e.last_mr.clone()))
            .unwrap_or_else(|| ("HEAD".into(), None, None));
        match BranchBrowser::open_workspace(&WorkspaceEntry {
            path: path.clone(),
            branch,
            mr,
            last_mr,
        }) {
            Ok(bb) => {
                self.state = MainState::Ready(LoadedBrowser::install(bb));
                self.refresh_store();
                self.branch_picker = None;
                self.mr_picker = None;
                self.mr_entry = None;
                self.clear_uncommitted();
                self.empty_mr = false;
                self.pending_kind_restore = false;
                self.restore_selected_entry(cx);
            }
            Err(_) => {
                // Stale entry already dropped by open_workspace; keep current Ready if any.
                self.refresh_store();
            }
        }
        cx.notify();
    }

    fn toggle_branch_picker(&mut self, cx: &mut Context<Self>) {
        self.mr_picker = None;
        if self.branch_picker.is_some() {
            self.branch_picker = None;
            cx.notify();
            return;
        }
        let bounds = self.branch_toggle_bounds.get();
        let (branches, current) = match &self.state {
            MainState::Ready(loaded) => (
                git::list_branches(loaded.repository().path()).unwrap_or_default(),
                loaded.branch_name().to_string(),
            ),
            MainState::Empty | MainState::Error(_) => (Vec::new(), String::new()),
        };
        self.branch_picker = Some(BranchPicker::new(branches, &current, bounds));
        cx.notify();
    }

    fn toggle_mr_picker(&mut self, cx: &mut Context<Self>) {
        if !gitlab_chrome_visible(self) {
            self.mr_picker = None;
            cx.notify();
            return;
        }
        if self.mr_picker.is_some() {
            self.mr_picker = None;
            cx.notify();
            return;
        }
        self.branch_picker = None;
        let bounds = self.mr_toggle_bounds.get();

        let repo_path = match &self.state {
            MainState::Ready(loaded) => loaded.repository().path().to_path_buf(),
            MainState::Empty | MainState::Error(_) => {
                self.mr_picker = Some(MrPicker::failed(
                    ErrorNote::plain("Open a repository to list merge requests."),
                    bounds,
                ));
                cx.notify();
                return;
            }
        };

        self.mr_picker = Some(MrPicker::loading(bounds));
        cx.notify();

        let base = settings_store::effective_base_url(&settings_store::load_file());
        let pat = settings_store::load_pat().unwrap_or_default();
        let selected_iid = self.mr_entry.as_ref().map(|e| e.summary.iid);

        cx.spawn(async move |this, cx| {
            let http = match cx.update(|app| app.http_client()) {
                Ok(client) => client,
                Err(_) => return,
            };

            let project = cx
                .background_executor()
                .spawn({
                    let http = http.clone();
                    let base = base.clone();
                    let pat = pat.clone();
                    let repo_path = repo_path.clone();
                    async move { gitlab::resolve_project(http, &base, &pat, &repo_path).await }
                })
                .await;

            let picker = match project {
                ResolveProjectResult::Err(e) => {
                    MrPicker::failed(ErrorNote::resolve_project(&e), Bounds::default())
                }
                ResolveProjectResult::Ok(identity) => {
                    let list_result = cx
                        .background_executor()
                        .spawn(async move {
                            gitlab::list_open_merge_requests(
                                http,
                                &base,
                                &pat,
                                &identity.path_with_namespace,
                            )
                            .await
                        })
                        .await;
                    match list_result {
                        ListMergeRequestsResult::Ok(mrs) => {
                            MrPicker::ready(mrs, selected_iid, Bounds::default())
                        }
                        ListMergeRequestsResult::Err(e) => MrPicker::failed(
                            ErrorNote::new(
                                gitlab::format_list_merge_requests_error(&e),
                                e.settings_fix(),
                            ),
                            Bounds::default(),
                        ),
                    }
                }
            };

            let _ = this.update(cx, |view, cx| {
                let Some(open) = &view.mr_picker else {
                    return;
                };
                let bounds = open.bounds;
                view.mr_picker = Some(picker.with_bounds(bounds));
                cx.notify();
            });
        })
        .detach();
    }

    fn select_mr(&mut self, mr: MergeRequestSummary, cx: &mut Context<Self>) {
        let iid = mr.iid;
        self.empty_mr = false;
        self.pending_kind_restore = false;
        self.mr_entry = Some(MrEntry {
            summary: mr,
            detail: MrDetailState::Loading,
            project: None,
        });
        self.mr_picker = None;
        cx.notify();
        self.spawn_mr_activate(iid, cx);
    }

    fn spawn_mr_activate(&mut self, iid: u64, cx: &mut Context<Self>) {
        let repo_path = match &self.state {
            MainState::Ready(loaded) => loaded.repository().path().to_path_buf(),
            MainState::Empty | MainState::Error(_) => return,
        };

        let base = settings_store::effective_base_url(&settings_store::load_file());
        let pat = settings_store::load_pat().unwrap_or_default();

        cx.spawn(async move |this, cx| {
            let http = match cx.update(|app| app.http_client()) {
                Ok(client) => client,
                Err(_) => return,
            };

            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let forge = GitLabForge { http };
                    let objects = SystemGitObjects;
                    mr_entry::resolve_mr_entry(&repo_path, iid, &base, &pat, &forge, &objects).await
                })
                .await;

            let result =
                outcome.map_err(|failure| ErrorNote::new(failure.message, failure.settings_target));
            let _ = this.update(cx, |view, cx| {
                apply_mr_activate_finish(view, iid, result, cx);
            });
        })
        .detach();
    }

    fn clear_mr_entry(&mut self, cx: &mut Context<Self>) {
        self.mr_entry = None;
        self.empty_mr = false;
        self.pending_kind_restore = false;
        self.clear_selected_mr_label();
        self.restore_branch_commits();
        cx.notify();
    }

    fn restore_branch_commits(&mut self) {
        let result = match &mut self.state {
            MainState::Ready(loaded) => {
                let branch = loaded.branch_name().to_string();
                loaded.switch_branch(&branch)
            }
            MainState::Empty | MainState::Error(_) => return,
        };
        if let Err(e) = result {
            self.state = MainState::Error(e.0);
        }
    }

    fn open_branch(&mut self, name: &str, cx: &mut Context<Self>) {
        self.clear_uncommitted();
        self.mr_entry = None;
        self.empty_mr = false;
        self.pending_kind_restore = false;
        let result = match &mut self.state {
            MainState::Ready(loaded) => loaded.switch_branch(name),
            MainState::Empty | MainState::Error(_) => return,
        };
        match result {
            Ok(()) => self.remember_current(),
            Err(e) => self.state = MainState::Error(e.0),
        }
        self.branch_picker = None;
        cx.notify();
    }

    fn open_repo_menu(
        &mut self,
        path: PathBuf,
        position: Point<Pixels>,
        kind: RepoMenuKind,
        cx: &mut Context<Self>,
    ) {
        self.repo_menu = Some(RepoContextMenu {
            path,
            position,
            kind,
        });
        cx.notify();
    }

    /// Right-click a commit row: select that row if outside the current
    /// Comparison, then show the commit context menu.
    fn open_commit_menu(&mut self, index: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let outside = match &self.state {
            MainState::Ready(loaded) => !loaded.in_range().get(index).copied().unwrap_or(false),
            MainState::Empty | MainState::Error(_) => return,
        };
        if outside {
            self.select_commit(index, false, cx);
        }
        self.commit_menu = Some(CommitContextMenu { position });
        cx.notify();
    }

    fn pin_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        workspace_store::pin(&path);
        self.refresh_store();
        self.repo_menu = None;
        cx.notify();
    }

    fn unpin_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        workspace_store::unpin(&path);
        self.refresh_store();
        self.repo_menu = None;
        cx.notify();
    }

    fn remove_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let removing_current = matches!(
            &self.state,
            MainState::Ready(loaded) if loaded.repository().path() == path.as_path()
        );
        workspace_store::drop_path(&path);
        self.refresh_store();
        self.repo_menu = None;
        if removing_current {
            self.state = MainState::Empty;
            self.clear_uncommitted();
            self.branch_picker = None;
            self.mr_picker = None;
            self.mr_entry = None;
            self.empty_mr = false;
            self.pending_kind_restore = false;
        }
        cx.notify();
    }

    fn handle_branch_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key.as_str() == "escape"
            && (self.repo_menu.is_some() || self.commit_menu.is_some())
        {
            self.repo_menu = None;
            self.commit_menu = None;
            cx.notify();
            return;
        }
        if let Some(picker) = &mut self.mr_picker {
            match picker.handle_key(event) {
                MrPickerAction::None => {}
                MrPickerAction::Changed => cx.notify(),
                MrPickerAction::Close => {
                    self.mr_picker = None;
                    cx.notify();
                }
                MrPickerAction::Select(mr) => self.select_mr(mr, cx),
            }
            return;
        }
        if let Some(picker) = &mut self.branch_picker {
            match event.keystroke.key.as_str() {
                "escape" => self.branch_picker = None,
                "backspace" => {
                    picker.query.pop();
                    picker.refresh();
                    picker.selected = 0;
                }
                "up" => picker.selected = picker.selected.saturating_sub(1),
                "down" => {
                    picker.selected =
                        (picker.selected + 1).min(picker.matches.len().saturating_sub(1))
                }
                "enter" => {
                    if let Some(branch) = picker.matches.get(picker.selected) {
                        let name = branch.name.clone();
                        self.open_branch(&name, cx);
                    }
                }
                _ => {
                    if event.keystroke.key.len() == 1 && !event.keystroke.modifiers.platform {
                        picker.query.push_str(&event.keystroke.key);
                        picker.refresh();
                        picker.selected = 0;
                    }
                }
            }
            cx.notify();
            return;
        }
        if self.covering() {
            return;
        }
        let mods = &event.keystroke.modifiers;
        if event.keystroke.key.as_str() == "a" && mods.secondary() && !mods.alt && !mods.shift {
            self.handle_commits_select_all(cx);
        }
    }

    /// Cmd/Ctrl+A: Branch → select all loaded + fold; MR Ready → restore `diff_refs`.
    fn handle_commits_select_all(&mut self, cx: &mut Context<Self>) {
        let result = match (&mut self.state, self.mr_entry.as_ref()) {
            (MainState::Ready(loaded), Some(entry)) => {
                if loaded.commits().is_empty() {
                    return;
                }
                match &entry.detail {
                    MrDetailState::Ready(detail) => loaded.restore_mr_diff_refs(
                        &detail.diff_refs.base_sha,
                        &detail.diff_refs.head_sha,
                    ),
                    MrDetailState::Loading | MrDetailState::Failed(_) => return,
                }
            }
            (MainState::Ready(loaded), None) => {
                if loaded.commits().is_empty() {
                    return;
                }
                loaded.select_all_commits()
            }
            (MainState::Empty | MainState::Error(_), _) => return,
        };
        match result {
            Ok(_) => {}
            Err(e) => self.state = MainState::Error(e.0),
        }
        cx.notify();
    }

    fn loaded(&self) -> Option<&LoadedBrowser> {
        match &self.state {
            MainState::Ready(loaded) => Some(loaded),
            MainState::Empty | MainState::Error(_) => None,
        }
    }

    fn covering(&self) -> bool {
        self.loaded().is_some_and(|loaded| loaded.covering())
    }

    /// Same gate as the Changes capsule Open Diff button / commit menu item.
    fn can_open_diff(&self) -> bool {
        self.loaded()
            .is_some_and(|loaded| loaded.open_diff_allowed())
    }

    fn active_paths(&self) -> Option<&[ChangedPath]> {
        self.loaded().map(|loaded| loaded.changed_paths())
    }

    fn active_comparison(&self) -> Option<&Comparison> {
        self.loaded().map(|loaded| loaded.comparison())
    }

    fn open_diff(&mut self, cx: &mut Context<Self>) {
        if !self.can_open_diff() {
            return;
        }
        let Some(paths) = self.active_paths() else {
            return;
        };
        let preferred = self.diff_window.and_then(|h| {
            h.update(cx, |view, _, _| {
                view.snapshot.as_ref().map(|s| s.selected_path.clone())
            })
            .ok()
            .flatten()
        });
        let path = preferred
            .filter(|p| paths.iter().any(|c| &c.path == p))
            .unwrap_or_else(|| paths[0].path.clone());
        self.push_diff(path, cx);
    }

    fn push_diff(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(comparison) = self.active_comparison().cloned() else {
            return;
        };
        let paths = self.active_paths().unwrap_or(&[]).to_vec();
        let status = paths
            .iter()
            .find(|p| p.path == path)
            .map(|p| p.status)
            .unwrap_or(PathStatus::Modify);
        let file = git::file_diff(&comparison, &path, status, &Default::default());
        let snapshot = DiffSnapshot {
            comparison,
            changed_paths: paths,
            selected_path: path,
            file,
        };
        open_or_update_diff(&mut self.diff_window, snapshot, &mut **cx);
        cx.notify();
    }

    fn select_commit(&mut self, index: usize, shift: bool, cx: &mut Context<Self>) {
        let result = {
            let MainState::Ready(loaded) = &mut self.state else {
                return;
            };
            loaded.select_commit(index, shift)
        };
        match result {
            Ok(()) => {}
            Err(e) => self.state = MainState::Error(e.0),
        }
        // ADR-0005: Diff stays on its own snapshot until Open Diff is clicked again.
        cx.notify();
    }

    fn open_repo(&mut self, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Repo".into()),
        });

        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };

            this.update(cx, |this, cx| {
                // A folder that is already a Workspace is choosing that Workspace (ADR-0012).
                // `remember_current` would write Branch and clear the selected MR label.
                if let Ok(canonical) = std::fs::canonicalize(&root) {
                    if this
                        .store
                        .workspaces
                        .iter()
                        .any(|entry| entry.path == canonical)
                    {
                        this.select_repo(canonical, cx);
                        return;
                    }
                }
                match BranchBrowser::open(&root) {
                    Ok(bb) => {
                        this.state = MainState::Ready(LoadedBrowser::install(bb));
                        this.mr_entry = None;
                        this.clear_uncommitted();
                        this.empty_mr = false;
                        this.pending_kind_restore = false;
                        this.mr_picker = None;
                        this.branch_picker = None;
                        let remembered = match &this.state {
                            MainState::Ready(loaded) => Some((
                                loaded.repository().path().to_path_buf(),
                                loaded.branch_name().to_string(),
                            )),
                            MainState::Empty | MainState::Error(_) => None,
                        };
                        if let Some((path, branch)) = remembered {
                            workspace_store::remember(&path, &branch);
                            this.refresh_store();
                        }
                    }
                    Err(e) => {
                        this.state = MainState::Error(e.0);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn open_or_update_diff(
    handle: &mut Option<WindowHandle<DiffView>>,
    snapshot: DiffSnapshot,
    cx: &mut App,
) {
    remember_diff_reopen_from_snapshot(&snapshot);

    if let Some(h) = *handle {
        let snap = snapshot.clone();
        if h.update(cx, |view, window, cx| {
            view.apply_snapshot(snap, cx);
            window.activate_window();
            cx.notify();
        })
        .is_ok()
        {
            return;
        }
    }

    let geometry = window_geometry_store::snapshot();
    let bounds = window_geometry::resolve_bounds(
        geometry.diff.as_ref(),
        window_geometry::diff_default_size(),
        cx,
    );
    let snap = snapshot.clone();
    match cx.open_window(
        WindowOptions {
            focus: true,
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("ReviewFox Diff".into()),
                appears_transparent: true,
                traffic_light_position: traffic_light_position(),
                ..Default::default()
            }),
            window_decorations: Some(WindowDecorations::Client),
            window_background: super::window_background_appearance(),
            ..Default::default()
        },
        move |window, cx| cx.new(|cx| DiffView::with_snapshot(snap, window, cx)),
    ) {
        Ok(h) => *handle = Some(h),
        Err(e) => eprintln!("failed to open diff window: {e}"),
    }
}

fn remember_diff_reopen_from_snapshot(snapshot: &DiffSnapshot) {
    window_geometry_store::note_diff_opened(DiffReopen {
        repository: snapshot.comparison.repository.path().to_path_buf(),
        base_oid: snapshot.comparison.base_oid.map(|o| o.to_string()),
        head_oid: snapshot.comparison.head_oid.to_string(),
        selected_path: snapshot.selected_path.clone(),
        uncommitted: snapshot.comparison.uncommitted,
    });
}

fn rebuild_diff_snapshot(reopen: &DiffReopen) -> Option<DiffSnapshot> {
    let base_oid: Option<Oid> = match &reopen.base_oid {
        Some(s) => Some(s.parse().ok()?),
        None => None,
    };
    let head_oid: Oid = reopen.head_oid.parse().ok()?;
    let comparison = Comparison {
        repository: Repository::new(reopen.repository.clone()),
        base_oid,
        head_oid,
        uncommitted: reopen.uncommitted,
    };
    let changed_paths = git::list_changed_paths_for(&comparison).ok()?;
    if changed_paths.is_empty() {
        return None;
    }
    let path = if changed_paths.iter().any(|p| p.path == reopen.selected_path) {
        reopen.selected_path.clone()
    } else {
        changed_paths[0].path.clone()
    };
    let status = changed_paths
        .iter()
        .find(|p| p.path == path)
        .map(|p| p.status)
        .unwrap_or(PathStatus::Modify);
    let file = git::file_diff(&comparison, &path, status, &Default::default());
    Some(DiffSnapshot {
        comparison,
        changed_paths,
        selected_path: path,
        file,
    })
}

#[cfg(target_os = "macos")]
fn traffic_light_position() -> Option<gpui::Point<gpui::Pixels>> {
    Some(gpui::point(
        px(theme::TRAFFIC_LIGHT_LEFT_INSET),
        px(theme::TRAFFIC_LIGHT_TOP_INSET),
    ))
}

#[cfg(not(target_os = "macos"))]
fn traffic_light_position() -> Option<gpui::Point<gpui::Pixels>> {
    None
}

impl Focusable for AppView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_default_mr_detail_height(window);
        self.sync_default_head_meta_height(window);
        if let Some(h) = self.diff_window {
            if h.update(cx, |_, _, _| ()).is_err() {
                self.diff_window = None;
                window_geometry_store::note_diff_closed_while_app_alive();
            }
        }
        if self.activation_sub.is_none() {
            self.activation_sub = Some(cx.observe_window_activation(window, |view, window, cx| {
                if !window.is_window_active()
                    && (view.branch_picker.is_some()
                        || view.mr_picker.is_some()
                        || view.repo_menu.is_some()
                        || view.commit_menu.is_some())
                {
                    view.branch_picker = None;
                    view.mr_picker = None;
                    view.repo_menu = None;
                    view.commit_menu = None;
                    cx.notify();
                }
            }));
        }
        if self.bounds_sub.is_none() {
            self.bounds_sub = Some(cx.observe_window_bounds(window, |_, window, cx| {
                window_geometry_store::set_main_bounds(window_geometry::stored_from_window(window));
                // Outside the debounced flush: the maximize glyph must flip on every bounds change.
                cx.notify();
                window_geometry::debounce_flush(cx);
            }));
        }
        self.sync_collapsed_dirs();
        let sidebar_w = if self.repos_collapsed {
            px(0.)
        } else {
            px(self.sidebar_width)
        };
        let show_sidebar_split = !self.repos_collapsed;

        #[cfg(target_os = "macos")]
        {
            ColumnVibrancy::ensure_synced_window(&mut self.window_vibrancy, window);
        }

        div()
            .id("main")
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, _, cx| this.handle_branch_key(event, cx)),
            )
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            // The window's one translucent layer; `#body`, `#repos` and `#stage`
            // all stay clear so it is not painted twice.
            .bg(theme::frost())
            .font_family(appearance::ui_font(cx))
            // Unsized UI text inherits gpui's 1rem default (16px), scaled like the rest.
            .ui_text_size(16., cx)
            .track_focus(&self.focus)
            // The only band that reaches both window edges, so it can own the whole drag
            // surface and seat the caption buttons in the corner.
            .child(render_titlebar(self, window, cx))
            .child(
                div()
                    .id("body")
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .overflow_hidden()
                    .child(render_sidebar(self, sidebar_w, cx))
                    .when(show_sidebar_split, |d| {
                        d.child(splitter::handle(
                            "sidebar-resize-handle",
                            Axis::HorizontalLeading,
                            self.sidebar_resize_handler(cx),
                            self.sidebar_resize_state.clone(),
                            true,
                        ))
                    })
                    // Frosted desk: floating capsules (Commit / MR / Changes). Stage stays
                    // clear so window vibrancy shows between islands.
                    .child(
                        div()
                            .id("stage")
                            .relative()
                            .h_full()
                            .flex_1()
                            .min_w(px(splitter::MIN_COMMITS_WIDTH))
                            .overflow_hidden()
                            .bg(theme::sidebar())
                            .when(!self.covering(), |d| d.child(render_commits(self, cx)))
                            .child(render_files(self, cx))
                            .when(self.repos_collapsed, |d| {
                                d.child(splitter::parked_leading_handle(
                                    "sidebar-parked-handle",
                                    self.sidebar_resize_handler(cx),
                                    self.sidebar_resize_state.clone(),
                                ))
                            }),
                    ),
            )
            .when(self.repo_menu.is_some(), |d| {
                d.child(render_repo_menu(self, cx))
            })
            .when(self.commit_menu.is_some(), |d| {
                d.child(render_commit_menu(self, cx))
            })
            // Outside `#stage` so overflow_hidden there cannot clip picker shadows.
            .when(self.branch_picker.is_some(), |d| {
                d.child(deferred(render_branch_picker(self, cx)))
            })
            .when(
                self.mr_picker.is_some() && gitlab_chrome_visible(self),
                |d| d.child(deferred(render_mr_picker(self, cx))),
            )
    }
}

fn render_sidebar(
    view: &AppView,
    width: gpui::Pixels,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let palette = theme::software_palette();
    let active_path = match &view.state {
        MainState::Ready(loaded) => Some(loaded.repository().path().to_path_buf()),
        MainState::Empty | MainState::Error(_) => None,
    };
    let gitlab_base = settings_store::effective_base_url(&settings_store::load_file());
    let row = |path: PathBuf| {
        let active = active_path.as_ref() == Some(&path);
        let gitlab = gitlab::repo_matches_settings_host(&path, &gitlab_base);
        let name = Repository::new(path.clone()).display_name();
        (path, name, active, gitlab)
    };
    let pinned_rows: Vec<(PathBuf, String, bool, bool)> =
        workspace_store::pinned_entries(&view.store)
            .into_iter()
            .map(|e| row(e.path.clone()))
            .collect();
    let pin_section = (!pinned_rows.is_empty()).then_some(pinned_rows);
    let has_pins = pin_section.is_some();
    let repos: Vec<(PathBuf, String, bool, bool)> =
        workspace_store::repository_entries(&view.store)
            .into_iter()
            .map(|e| row(e.path.clone()))
            .collect();

    div()
        .id("repos")
        .h_full()
        .w(width)
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(palette.surface.sidebar_backing)
        .child({
            let (scroll, sb) = scrollbar::vertical("sidebar-repos-sb", cx);
            scrollbar::overlay_flex(
                div()
                    .id("sidebar-repos-scroll")
                    .size_full()
                    .px(px(theme::SIDEBAR_ROW_INSET))
                    .pt(px(theme::SIDEBAR_SCROLL_PAD_TOP))
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .when_some(pin_section, |d, pinned_rows| {
                        d.child(sidebar_section_header("Pin", false, cx)).children(
                            pinned_rows.into_iter().enumerate().map(
                                |(i, (path, name, active, gitlab))| {
                                    sidebar_repo_row(
                                        ("pin-repo", i),
                                        path,
                                        name,
                                        active,
                                        gitlab,
                                        Some(RepoMenuKind::Pin),
                                        cx,
                                    )
                                },
                            ),
                        )
                    })
                    .child(sidebar_section_header("Repositories", has_pins, cx))
                    .children(repos.into_iter().enumerate().map(
                        |(i, (path, name, active, gitlab))| {
                            sidebar_repo_row(
                                ("repo", i),
                                path,
                                name,
                                active,
                                gitlab,
                                Some(RepoMenuKind::Repositories),
                                cx,
                            )
                        },
                    )),
                sb,
            )
        })
        .child(
            div()
                .flex_none()
                .px(px(theme::SIDEBAR_ROW_INSET))
                .pb(px(theme::SIDEBAR_SETTINGS_PAD_BOTTOM))
                .child(
                    sidebar_nav_row("open-settings", "gear.svg", "Settings", false, cx).on_click(
                        cx.listener(|_, _, window, cx| {
                            // Deferred: `cx.dispatch_action` can't re-enter this window mid-update.
                            window.dispatch_action(Box::new(OpenSettings), cx);
                        }),
                    ),
                ),
        )
}

fn sidebar_nav_row(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    label: impl Into<gpui::SharedString>,
    selected: bool,
    cx: &App,
) -> gpui::Stateful<Div> {
    let palette = theme::software_palette();
    div()
        .id(id)
        .w_full()
        .h(px(theme::SIDEBAR_ROW_HEIGHT))
        .pl(px(
            theme::SIDEBAR_ROW_PAD_X - theme::SIDEBAR_ROW_INDICATOR_IDLE_WIDTH
        ))
        .pr(px(theme::SIDEBAR_ROW_PAD_X))
        .border_l(px(theme::SIDEBAR_ROW_INDICATOR_IDLE_WIDTH))
        .border_color(palette.sidebar_row.idle_indicator)
        .rounded(px(theme::SIDEBAR_ROW_RADIUS))
        .min_w(px(0.))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(theme::SIDEBAR_ICON_LABEL_GAP))
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(palette.sidebar_row.selected)
                .border_color(palette.sidebar_row.selection_indicator)
                .hover(|d| {
                    d.bg(palette.sidebar_row.selected_hover)
                        .border_l(px(theme::SIDEBAR_ROW_INDICATOR_HOVER_WIDTH))
                        .pl(px(
                            theme::SIDEBAR_ROW_PAD_X - theme::SIDEBAR_ROW_INDICATOR_HOVER_WIDTH
                        ))
                })
                .active(|d| {
                    d.bg(palette.sidebar_row.selected_pressed)
                        .border_l(px(theme::SIDEBAR_ROW_INDICATOR_PRESSED_WIDTH))
                        .pl(px(
                            theme::SIDEBAR_ROW_PAD_X - theme::SIDEBAR_ROW_INDICATOR_PRESSED_WIDTH
                        ))
                })
        })
        .when(!selected, |d| {
            d.hover(|d| {
                d.bg(palette.sidebar_row.hover)
                    .border_color(palette.sidebar_row.hover_indicator)
            })
            .active(|d| {
                d.bg(palette.sidebar_row.pressed)
                    .border_color(palette.sidebar_row.pressed_indicator)
            })
        })
        .child(
            svg()
                .size(theme::ICON_SIZE)
                .flex_none()
                .path(icon)
                .text_color(palette.text.secondary),
        )
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .ui_label_size(13., cx)
                .text_color(palette.text.primary)
                .child(label.into()),
        )
}

fn sidebar_section_header(label: &'static str, extra_top: bool, cx: &App) -> impl IntoElement {
    div()
        .h(px(theme::SIDEBAR_SECTION_HEIGHT))
        .when(extra_top, |d| d.mt(px(theme::SIDEBAR_SECTION_GAP)))
        .pl(px(theme::SIDEBAR_ROW_PAD_X))
        .flex()
        .items_end()
        .ui_text_size(11., cx)
        .text_color(theme::software_palette().text.section)
        .child(label)
}

fn sidebar_repo_row(
    id: impl Into<gpui::ElementId>,
    path: PathBuf,
    name: String,
    active: bool,
    gitlab: bool,
    menu: Option<RepoMenuKind>,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let path_click = path.clone();
    let path_menu = path.clone();
    let icon = if gitlab { "gitlab.svg" } else { "folder.svg" };
    sidebar_nav_row(id, icon, name, active, cx)
        .mb(px(theme::SIDEBAR_ROW_GAP))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select_repo(path_click.clone(), cx);
        }))
        .when_some(menu, |d, kind| {
            d.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_repo_menu(path_menu.clone(), event.position, kind, cx);
                }),
            )
        })
        .into_any_element()
}

fn render_repo_menu(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(menu) = &view.repo_menu else {
        return div().into_any_element();
    };
    let path = menu.path.clone();
    type MenuAction = fn(&mut AppView, PathBuf, &mut Context<AppView>);
    let items: Vec<(&str, &str, MenuAction)> = match menu.kind {
        RepoMenuKind::Pin => vec![
            ("pin_off.svg", "Unpin", AppView::unpin_repo),
            ("trash.svg", "Remove", AppView::remove_repo),
        ],
        RepoMenuKind::Repositories => vec![
            ("pin.svg", "Pin", AppView::pin_repo),
            ("trash.svg", "Remove", AppView::remove_repo),
        ],
    };
    let position = menu.position;

    deferred(
        anchored()
            .position(position)
            .anchor(Corner::TopLeft)
            .snap_to_window()
            .child(
                div()
                    .id("repo-context-menu")
                    .min_w(px(200.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .bg(theme::white())
                    .rounded_lg()
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.repo_menu = None;
                        cx.notify();
                    }))
                    .children(
                        items
                            .into_iter()
                            .enumerate()
                            .map(|(i, (icon, label, action))| {
                                let path = path.clone();
                                div()
                                    .id(("repo-menu-item", i))
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .hover(|d| d.bg(theme::hover()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        action(this, path.clone(), cx);
                                    }))
                                    .child(
                                        svg()
                                            .size(theme::ICON_SIZE)
                                            .flex_none()
                                            .path(icon)
                                            .text_color(theme::muted()),
                                    )
                                    .child(
                                        div()
                                            .ui_label_size(14., cx)
                                            .text_color(theme::text())
                                            .child(label),
                                    )
                            }),
                    ),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

fn render_commit_menu(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(menu) = &view.commit_menu else {
        return div().into_any_element();
    };
    let position = menu.position;
    let can_open = view.can_open_diff();

    deferred(
        anchored()
            .position(position)
            .anchor(Corner::TopLeft)
            .snap_to_window()
            .child(
                div()
                    .id("commit-context-menu")
                    .min_w(px(200.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .bg(theme::white())
                    .rounded_lg()
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.commit_menu = None;
                        cx.notify();
                    }))
                    .child({
                        let item = div()
                            .id("commit-menu-open-diff")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .gap_2();
                        let item = if can_open {
                            item.cursor_pointer()
                                .hover(|d| d.bg(theme::hover()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.commit_menu = None;
                                    this.open_diff(cx);
                                }))
                        } else {
                            item
                        };
                        item.child(
                            svg()
                                .size(theme::ICON_SIZE)
                                .flex_none()
                                .path("diff_title.svg")
                                .text_color(if can_open {
                                    theme::muted()
                                } else {
                                    theme::faint()
                                }),
                        )
                        .child(
                            div()
                                .ui_label_size(14., cx)
                                .text_color(if can_open {
                                    theme::text()
                                } else {
                                    theme::faint()
                                })
                                .child("Open Diff"),
                        )
                    }),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

/// The window's one full-width band. Every gap in it drags, and on Windows the caption
/// buttons close it out flush against the right edge — no floating overlay, no dead strip.
fn render_titlebar(view: &AppView, window: &Window, cx: &mut Context<AppView>) -> impl IntoElement {
    let mono = appearance::code_font(cx);
    let (branch, label, checkout) = match &view.state {
        MainState::Ready(loaded) => {
            let branch = loaded.branch_name().to_string();
            let label = loaded.comparison().label();
            let checkout = loaded.checkout_label().unwrap_or(&branch).to_string();
            (branch, label, checkout)
        }
        MainState::Empty | MainState::Error(_) => ("—".into(), "—".into(), "—".into()),
    };
    let show_gitlab = gitlab_chrome_visible(view);
    let entry_chrome =
        render_entry_chrome(view, &branch, &checkout, show_gitlab, cx).into_any_element();

    // Leading zone spans exactly what sits left of the stage, so the pills after it start on
    // the stage's left edge and share a left edge with the islands below.
    let leading_w = if view.repos_collapsed {
        px(collapsed_leading_width())
    } else {
        px(view.sidebar_width + theme::CHANGES_SHADOW_GAP)
    };

    div()
        .id("titlebar")
        .h(theme::TITLEBAR_HEIGHT)
        .flex_none()
        .flex()
        .items_center()
        .child(
            div()
                .id("titlebar-leading")
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
                .child(toggle_button(
                    "main-sidebar-toggle",
                    view.repos_collapsed,
                    cx,
                ))
                .child(open_repo_button("open-repo", cx))
                .child(
                    div()
                        .id("titlebar-drag-leading")
                        .h_full()
                        .flex_1()
                        .min_w(px(0.))
                        .window_control_area(WindowControlArea::Drag)
                        .occlude(),
                ),
        )
        .child(
            div()
                .id("titlebar-main")
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .items_center()
                // Balances the pills against the island below; see `titlebar-leading`.
                .pt(px(theme::CHANGES_TOP_INSET))
                .gap(px(theme::CHROME_GAP))
                // Same left edge as the island below. See `theme::left_island_inset`.
                .pl(px(theme::left_island_inset(!view.repos_collapsed)))
                .child(entry_chrome)
                .child(
                    div()
                        .id("titlebar-drag")
                        .h_full()
                        .flex_1()
                        .min_w(px(0.))
                        .window_control_area(WindowControlArea::Drag)
                        .occlude(),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(mono.clone())
                        .text_xs()
                        .text_color(theme::muted())
                        .child(label),
                )
                .child(
                    // Doubles as the trailing inset when no caption buttons follow.
                    div()
                        .id("titlebar-drag-trailing")
                        .h_full()
                        .w(px(theme::CHANGES_INSET))
                        .flex_none()
                        .window_control_area(WindowControlArea::Drag)
                        .occlude(),
                ),
        )
        .children(window_controls(window))
}

/// Kind track + value pill (ADR-0011, ADR-0013). MR is omitted when GitLab chrome is off.
fn render_entry_chrome(
    view: &AppView,
    branch: &str,
    checkout: &str,
    show_gitlab: bool,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let kind =
        entry_chrome::active_entry_kind(view.mr_entry.is_some(), view.empty_mr, view.covering());
    let mr_iid = view.mr_entry.as_ref().map(|e| e.summary.iid);
    let value = entry_chrome::value_label(kind, branch, mr_iid, checkout);
    let picker_open = view.branch_picker.is_some() || view.mr_picker.is_some();
    let hide_value = entry_chrome::value_pill_hidden(picker_open);
    let branch_track = view.branch_toggle_bounds.clone();
    let mr_track = view.mr_toggle_bounds.clone();
    let value_icon = match kind {
        EntryKind::Branch => "branch.svg",
        EntryKind::Mr => "gitlab.svg",
        EntryKind::Uncommitted => "folder.svg",
    };

    div()
        .id("entry-chrome")
        .flex()
        .items_center()
        .gap(px(theme::CHROME_GAP))
        .flex_none()
        .min_w(px(0.))
        .max_w(px(480.))
        .child(
            div()
                .id("entry-kind-track")
                .h(theme::TOGGLE_SIZE)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(entry_kind_hit(
                    "entry-kind-branch",
                    "Branch",
                    "branch.svg",
                    kind == EntryKind::Branch,
                    EntryKind::Branch,
                    cx,
                ))
                .when(show_gitlab, |d| {
                    d.child(entry_kind_hit(
                        "entry-kind-mr",
                        "MR",
                        "gitlab.svg",
                        kind == EntryKind::Mr,
                        EntryKind::Mr,
                        cx,
                    ))
                })
                .child(entry_kind_hit(
                    "entry-kind-uncommitted",
                    "Uncommitted",
                    "folder.svg",
                    kind == EntryKind::Uncommitted,
                    EntryKind::Uncommitted,
                    cx,
                )),
        )
        .child(
            div()
                .id("entry-kind-divider")
                .w(px(1.))
                .h(px(14.))
                .flex_none()
                .bg(theme::faint()),
        )
        .child(
            div()
                .id("entry-value-pill")
                .relative()
                .h(theme::TOGGLE_SIZE)
                .flex_1()
                .min_w(px(0.))
                .max_w(px(260.))
                .px_2()
                .rounded_full()
                .bg(theme::capsule())
                .flex()
                .items_center()
                .gap_1()
                .overflow_hidden()
                .when(hide_value, |d| d.opacity(0.))
                .when(!hide_value && kind != EntryKind::Uncommitted, |d| {
                    d.hover(|d| d.bg(theme::capsule_track_hover()))
                })
                .child(
                    canvas(
                        move |bounds, _, _| {
                            branch_track.set(bounds);
                            mr_track.set(bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .when(kind != EntryKind::Uncommitted, |d| {
                    d.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| match kind {
                            EntryKind::Branch => this.toggle_branch_picker(cx),
                            EntryKind::Mr => this.toggle_mr_picker(cx),
                            EntryKind::Uncommitted => {}
                        }))
                })
                .when(kind == EntryKind::Mr && view.mr_entry.is_some(), |el| {
                    el.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| this.clear_mr_entry(cx)),
                    )
                })
                .when(kind == EntryKind::Uncommitted, |el| {
                    el.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            this.select_entry_kind(EntryKind::Branch, cx);
                        }),
                    )
                })
                .child(
                    svg()
                        .size(theme::ICON_SIZE)
                        .flex_none()
                        .path(value_icon)
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .ui_label_size(12., cx)
                        .text_color(theme::text())
                        .child(value),
                )
                .when(kind != EntryKind::Uncommitted, |d| {
                    d.child(
                        svg()
                            .size(theme::ICON_SIZE_SM)
                            .flex_none()
                            .path("chevron_down.svg")
                            .text_color(theme::faint()),
                    )
                }),
        )
}

fn entry_kind_hit(
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    selected: bool,
    target: EntryKind,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let fg = if selected {
        theme::on_sidebar_selected()
    } else {
        theme::muted()
    };
    div()
        .id(id)
        .h(theme::TOGGLE_SIZE)
        .px_2()
        .rounded_full()
        .flex()
        .items_center()
        .gap_1()
        .flex_none()
        .cursor_pointer()
        .when(selected, |d| d.bg(theme::sidebar_selected()))
        .when(!selected, |d| {
            d.bg(theme::capsule()).hover(|d| d.bg(theme::hover()))
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select_entry_kind(target, cx);
        }))
        .child(
            svg()
                .size(theme::ICON_SIZE)
                .flex_none()
                .path(icon)
                .text_color(fg),
        )
        .child(
            div()
                .ui_label_size(12., cx)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(fg)
                .child(label),
        )
}

/// Width the leading chrome reserves when the sidebar is collapsed. The islands do not follow
/// it: with nothing left of the stage they start at the window edge.
fn collapsed_leading_width() -> f32 {
    let controls = 12. + f32::from(theme::TOGGLE_SIZE) * 2. + theme::CHROME_GAP;
    #[cfg(target_os = "macos")]
    let controls = controls + theme::TRAFFIC_LIGHTS_WIDTH + theme::CHROME_GAP;
    controls
}

fn render_commits(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let show_gitlab = gitlab_chrome_visible(view);
    // Clear Changes. Pills share the island's left edge: the rail seam when the
    // sidebar is open, `CHANGES_INSET` when it is collapsed.
    let float_gap = px(theme::changes_float_clearance(view.files_width));
    let inset = px(theme::CHANGES_INSET);
    let show_mr = show_gitlab && view.mr_entry.is_some();

    let islands = div()
        .id("commit-islands")
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        // Seam is outside the stage while the rail is open; don't pad it twice.
        .pl(px(theme::left_island_inset(!view.repos_collapsed)))
        // No right padding: `changes_float_clearance` already ends this column one
        // gap short of the Changes island, so padding here would count it twice.
        .pt(px(theme::CHANGES_TOP_INSET))
        .pb(inset)
        .when(show_mr, |d| {
            d.child(render_mr_entry_detail(
                view.mr_entry.as_ref().unwrap(),
                view.mr_detail_height,
                cx,
            ))
            .child(splitter::handle(
                "mr-detail-resize-handle",
                Axis::VerticalNorth,
                view.mr_detail_resize_handler(cx),
                view.mr_detail_resize_state.clone(),
                true,
            ))
        })
        .child(render_commit_capsule(view, cx));

    let shared_column = div()
        .id("commits-column")
        .flex_1()
        .h_full()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .child(islands);

    div().id("commits").absolute().inset_0().child(
        div()
            .id("commits-content")
            .absolute()
            .inset_0()
            .right(float_gap)
            .flex()
            .items_start()
            .child(shared_column),
    )
}

fn render_commit_capsule(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mono = appearance::code_font(cx);
    let body = match &view.state {
        MainState::Empty => div().flex_1().into_any_element(),
        MainState::Error(msg) => div()
            .flex_1()
            .px_3()
            .py_3()
            .ui_text_size(14., cx)
            .text_color(rgb(0xb42318))
            .child(msg.clone())
            .into_any_element(),
        MainState::Ready(loaded) => {
            let (scroll, sb) = scrollbar::vertical("commit-list-sb", cx);
            // Same shape as file-tree: overlay_flex must be a flex_col child so
            // flex_1 gets a viewport height; nesting under a non-flex wrapper made
            // the scroller grow with content → max_offset stayed 0 → no thumb.
            scrollbar::overlay_flex(
                div()
                    .id("commit-list")
                    .size_full()
                    .pt_1()
                    .pb_1()
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .children(loaded.commits().iter().enumerate().map(|(i, commit)| {
                        let in_range = loaded.in_range().get(i).copied().unwrap_or(false);
                        let summary = commit.summary.clone();
                        let meta = format!(
                            "{} · {} · {}",
                            commit.oid.short(),
                            commit.author,
                            commit.time_label
                        );
                        div()
                            .id(("commit", i))
                            .mx_1()
                            .my_0p5()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .cursor_pointer()
                            .when(in_range, |d| d.bg(theme::range()))
                            .hover(move |d| {
                                if in_range {
                                    d.bg(theme::range())
                                } else {
                                    d.bg(theme::hover())
                                }
                            })
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                // Double-click: always fold Comparison to this single
                                // commit, then open Diff (ignores any Shift range).
                                if event.click_count() >= 2 {
                                    this.select_commit(i, false, cx);
                                    this.open_diff(cx);
                                } else {
                                    this.select_commit(i, event.modifiers().shift, cx);
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.open_commit_menu(i, event.position, cx);
                                }),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w(px(0.))
                                            .ui_text_size(14., cx)
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(theme::text())
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(summary),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w(px(0.))
                                            .font_family(mono.clone())
                                            .text_xs()
                                            .text_color(theme::muted())
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(meta),
                                    ),
                            )
                    })),
                sb,
            )
            .into_any_element()
        }
    };

    div()
        .id("commit-capsule")
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(theme::white())
        .rounded(px(theme::CHANGES_RADIUS))
        .overflow_hidden()
        .child(body)
}

struct MrEntry {
    summary: MergeRequestSummary,
    detail: MrDetailState,
    /// GitLab `path_with_namespace` once known (for Workspace label).
    project: Option<String>,
}

type MrActivateReady = mr_entry::Ready;

fn finish_mr_activate(
    view: &mut AppView,
    iid: u64,
    result: Result<MrActivateReady, ErrorNote>,
) -> bool {
    let Some(entry) = view.mr_entry.as_mut() else {
        return false;
    };
    if entry.summary.iid != iid {
        return false;
    }
    match result {
        Ok(ready) => {
            view.pending_kind_restore = false;
            view.empty_mr = false;
            entry.summary = MergeRequestSummary {
                iid: ready.detail.iid,
                title: ready.detail.title.clone(),
                source_branch: ready.detail.source_branch.clone(),
                target_branch: ready.detail.target_branch.clone(),
            };
            entry.project = Some(ready.project);
            let base_sha = ready.detail.diff_refs.base_sha.clone();
            let head_sha = ready.detail.diff_refs.head_sha.clone();
            entry.detail = MrDetailState::Ready(ready.detail);
            if let MainState::Ready(loaded) = &mut view.state {
                if let Err(e) = loaded.apply_mr_commits(ready.commit_infos, &base_sha, &head_sha) {
                    entry.detail = MrDetailState::Failed(ErrorNote::plain(e.0));
                }
            }
            view.remember_current();
            false
        }
        Err(msg) => match entry_chrome::restore_failure_action(view.pending_kind_restore) {
            RestoreFailureAction::KeepFailedDetail => {
                view.pending_kind_restore = false;
                entry.detail = MrDetailState::Failed(msg);
                false
            }
            RestoreFailureAction::EnterEmptyMrOpenPicker => {
                view.pending_kind_restore = false;
                view.mr_entry = None;
                view.empty_mr = true;
                true
            }
        },
    }
}

fn apply_mr_activate_finish(
    view: &mut AppView,
    iid: u64,
    result: Result<MrActivateReady, ErrorNote>,
    cx: &mut Context<AppView>,
) {
    if finish_mr_activate(view, iid, result) {
        view.ensure_mr_picker_open(cx);
    }
    cx.notify();
}

/// Titlebar + Changes top inset — viewport Y offset before the island column.
fn mr_detail_chrome_offset() -> f32 {
    f32::from(theme::TITLEBAR_HEIGHT) + theme::CHANGES_TOP_INSET
}

/// Vertical room for MR detail + frost gap + commit list inside the island column.
fn mr_detail_column_available(window: &Window) -> f32 {
    f32::from(window.viewport_size().height) - mr_detail_chrome_offset() - theme::CHANGES_INSET
}

/// `#files-slot` spans CHANGES_TOP_INSET below the titlebar to CHANGES_INSET above the bottom.
fn changes_island_height(window: &Window) -> f32 {
    f32::from(window.viewport_size().height)
        - f32::from(theme::TITLEBAR_HEIGHT)
        - theme::CHANGES_TOP_INSET
        - theme::CHANGES_INSET
}

/// Show MR picker only when a remote host matches Settings.
fn gitlab_chrome_visible(view: &AppView) -> bool {
    let MainState::Ready(loaded) = &view.state else {
        return false;
    };
    let base = settings_store::effective_base_url(&settings_store::load_file());
    gitlab::repo_matches_settings_host(loaded.repository().path(), &base)
}

enum MrDetailState {
    Loading,
    Ready(MergeRequestDetail),
    Failed(ErrorNote),
}

/// Failure text plus the Settings field that fixes it, if any; that target
/// gets an "Open Settings" link (token / Base URL fixes only).
struct ErrorNote {
    message: String,
    open_settings: Option<SettingsTarget>,
}

impl ErrorNote {
    fn new(message: impl Into<String>, open_settings: Option<SettingsTarget>) -> Self {
        Self {
            message: message.into(),
            open_settings,
        }
    }

    fn plain(message: impl Into<String>) -> Self {
        Self::new(message, None)
    }

    fn resolve_project(e: &gitlab::ResolveProjectError) -> Self {
        Self::new(gitlab::format_resolve_project_error(e), e.settings_fix())
    }
}

/// Red failure text, with an "Open Settings" link when the fix lives there.
/// `close_mr_picker`: the MR picker is a transient overlay, so leave it on click.
fn render_error_note(
    id: &'static str,
    note: &ErrorNote,
    close_mr_picker: bool,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_color(rgb(0xb42318)).child(note.message.clone()))
        .when_some(note.open_settings, |d, target| {
            d.child(
                div()
                    .id(id)
                    .cursor_pointer()
                    .text_color(theme::accent())
                    .hover(|d| d.underline())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if close_mr_picker {
                            this.mr_picker = None;
                            cx.notify();
                        }
                        // Deferred: opening / focusing Settings runs outside this window's update.
                        cx.defer(move |cx| settings::open_or_focus_settings(Some(target), cx));
                    }))
                    .child("Open Settings"),
            )
        })
        .into_any_element()
}

fn render_mr_entry_detail(
    entry: &MrEntry,
    height: f32,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let (scroll, sb) = scrollbar::vertical("mr-detail-sb", cx);
    div()
        .id("mr-entry-detail")
        .h(px(height))
        .flex_none()
        .flex()
        .flex_col()
        .px_3()
        .py_2()
        .bg(theme::white())
        .rounded(px(theme::CHANGES_RADIUS))
        .overflow_hidden()
        .ui_text_size(12., cx)
        .text_color(theme::muted())
        .child(scrollbar::overlay_flex(
            div()
                .id("mr-entry-detail-body")
                .size_full()
                .track_scroll(&scroll)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1p5()
                .children(match &entry.detail {
                    MrDetailState::Loading => {
                        vec![
                            div()
                                .flex_1()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(loading_row(
                                    "mr-detail-loading-spin",
                                    "Loading MR detail…",
                                    cx,
                                ))
                                .into_any_element(),
                        ]
                    }
                    MrDetailState::Failed(note) => {
                        vec![render_error_note(
                            "mr-detail-open-settings",
                            note,
                            false,
                            cx,
                        )]
                    }
                    MrDetailState::Ready(detail) => mr_entry_ready_lines(detail, cx),
                }),
            sb,
        ))
}

fn mr_entry_ready_lines(detail: &MergeRequestDetail, cx: &mut App) -> Vec<gpui::AnyElement> {
    let mut lines: Vec<gpui::AnyElement> = vec![
        div()
            .ui_text_size(14., cx)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme::text())
            .child(detail.title.clone())
            .into_any_element(),
        metadata::row(mr_metadata_items(detail), cx).into_any_element(),
    ];

    if let Some(desc) = detail.description.as_deref() {
        lines.push(
            selectable_markdown::view(format!("mr-desc-{}", detail.iid), desc.to_owned(), cx)
                .into_any_element(),
        );
    }

    lines
}

fn mr_metadata_items(detail: &MergeRequestDetail) -> Vec<metadata::Item> {
    let mut items = vec![
        metadata::Item {
            label: "ID",
            value: format!("!{}", detail.iid),
        },
        metadata::Item {
            label: "Status",
            value: detail.state.clone(),
        },
        metadata::Item {
            label: "Author",
            value: detail.author_username.clone(),
        },
        metadata::Item {
            label: "Branches",
            value: format!("{} → {}", detail.source_branch, detail.target_branch),
        },
    ];
    if let Some(ms) = &detail.merge_status {
        items.push(metadata::Item {
            label: "Merge",
            value: ms.clone(),
        });
    }
    if let Some(pipe) = &detail.check_state.pipeline_status {
        items.push(metadata::Item {
            label: "Pipeline",
            value: pipe.clone(),
        });
    }
    if let Some(label) = &detail.check_state.approvals_label {
        items.push(metadata::Item {
            label: "Approvals",
            value: label.clone(),
        });
    } else if detail.check_state.approved == Some(true) {
        items.push(metadata::Item {
            label: "Approvals",
            value: "approved".into(),
        });
    }
    items
}

fn render_mr_picker(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(picker) = &view.mr_picker else {
        return div().into_any_element();
    };

    const WIDTH: f32 = 380.;
    const HEIGHT: f32 = 420.;
    let seed = picker.bounds.size;

    // Match branch picker: fixed size, one overlay_flex, filter + rows in the same scroll.
    let body = match &picker.body {
        MrPickerBody::Loading => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(loading_row(
                "mr-picker-loading-spin",
                "Loading open merge requests…",
                cx,
            ))
            .into_any_element(),
        MrPickerBody::Failed(note) => div()
            .size_full()
            .px_2()
            .py_3()
            .ui_text_size(14., cx)
            .child(render_error_note("mr-picker-open-settings", note, true, cx))
            .into_any_element(),
        MrPickerBody::Ready {
            query,
            matches,
            selected,
            ..
        } => {
            let (scroll, sb) = scrollbar::vertical("mr-picker-sb", cx);
            let filter = if query.is_empty() {
                "Open merge requests (type to filter)".to_string()
            } else {
                format!("Filter: {query}")
            };
            // Explicit px size (not size_full %): percentage width under animated/anchored
            // parents re-triggers the text_ellipsis → few-glyphs collapse (a59dbf2).
            let inner_w = WIDTH - 8.;
            let inner_h = HEIGHT - 8.;
            picker_scroll_area(
                "mr-picker-scroll",
                inner_w,
                inner_h,
                div()
                    .id("mr-picker-scroll")
                    .w(px(inner_w))
                    .h(px(inner_h))
                    .bg(theme::white())
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .ui_text_size(12., cx)
                            .text_color(theme::muted())
                            .child(filter),
                    )
                    .when(matches.is_empty(), |d| {
                        d.child(
                            div()
                                .px_3()
                                .py_2()
                                .ui_text_size(14., cx)
                                .text_color(theme::muted())
                                .child("No matching merge requests."),
                        )
                    })
                    .children(matches.iter().enumerate().map(|(i, mr)| {
                        let select_mr = mr.clone();
                        let title = format!("!{} · {}", mr.iid, mr.title);
                        let branches = format!("{} → {}", mr.source_branch, mr.target_branch);
                        div()
                            .id(("mr", i))
                            .w(px(inner_w))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .cursor_pointer()
                            .when(i == *selected, |d| d.bg(theme::range()))
                            .hover(|d| d.bg(theme::hover()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_mr(select_mr.clone(), cx);
                            }))
                            .child(picker_line(theme::text(), true, title, cx))
                            .child(picker_line(theme::muted(), false, branches, cx))
                    })),
                sb,
            )
            .into_any_element()
        }
    };

    // Window-anchored outside `#stage` so stage overflow cannot clip the soft cast.
    let origin = view.mr_toggle_bounds.get().origin;

    anchored()
        .position(origin)
        .anchor(Corner::TopLeft)
        .snap_to_window()
        .child(picker_clip_shell(
            "mr-picker",
            "mr-picker-open",
            seed,
            WIDTH,
            HEIGHT,
            cx.listener(|this, _, _, cx| {
                this.mr_picker = None;
                cx.notify();
            }),
            body,
        ))
        .into_any_element()
}

fn render_branch_picker(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(picker) = &view.branch_picker else {
        return div().into_any_element();
    };
    const WIDTH: f32 = 320.;
    const HEIGHT: f32 = 420.;
    let seed = picker.bounds.size;
    let (scroll, sb) = scrollbar::vertical("branch-picker-sb", cx);
    let inner_w = WIDTH - 8.;
    let inner_h = HEIGHT - 8.;
    let body = picker_scroll_area(
        "branch-picker-scroll",
        inner_w,
        inner_h,
        div()
            .id("branch-picker-scroll")
            .w(px(inner_w))
            .h(px(inner_h))
            .bg(theme::white())
            .track_scroll(&scroll)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .ui_text_size(12., cx)
                    .text_color(theme::muted())
                    .child(format!("Filter: {}", picker.query)),
            )
            .children(picker.matches.iter().enumerate().map(|(i, branch)| {
                let name = branch.name.clone();
                div()
                    .id(("branch", i))
                    .w(px(inner_w))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(i == picker.selected, |d| d.bg(theme::range()))
                    .hover(|d| d.bg(theme::hover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_branch(&name, cx);
                    }))
                    .child(picker_line(theme::text(), true, branch.name.clone(), cx))
                    .child(picker_line(
                        theme::muted(),
                        false,
                        format!(
                            "{} · {} · {}",
                            branch.tip.author, branch.tip.time_label, branch.tip.summary
                        ),
                        cx,
                    ))
            })),
        sb,
    );

    // Window-anchored outside `#stage` so stage overflow cannot clip the soft cast.
    // Top-left = branch pill; the islands only share its left edge while the sidebar is open.
    let origin = view.branch_toggle_bounds.get().origin;

    anchored()
        .position(origin)
        .anchor(Corner::TopLeft)
        .snap_to_window()
        .child(picker_clip_shell(
            "branch-picker",
            "branch-picker-open",
            seed,
            WIDTH,
            HEIGHT,
            cx.listener(|this, _, _, cx| {
                this.branch_picker = None;
                cx.notify();
            }),
            body,
        ))
        .into_any_element()
}

/// Outer shell carries the soft cast; inner clip grows from capsule seed.
/// (Shadow + overflow_hidden on one node clips the cast → hard edge against frost.)
fn picker_clip_shell(
    id: &'static str,
    anim_id: &'static str,
    seed: Size<Pixels>,
    width: f32,
    height: f32,
    on_down_out: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    body: impl IntoElement,
) -> impl IntoElement {
    let seed_w = f32::from(seed.width).max(1.);
    let seed_h = f32::from(seed.height).max(1.);
    div()
        .id(id)
        .rounded(px(theme::CHANGES_RADIUS))
        .shadow(theme::picker_shadow())
        .occlude()
        .on_mouse_down_out(on_down_out)
        .child(
            div()
                .size_full()
                .overflow_hidden()
                .bg(theme::white())
                .rounded(px(theme::CHANGES_RADIUS))
                .child(div().w(px(width)).h(px(height)).p_1().child(body)),
        )
        .with_animation(
            anim_id,
            Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
            move |this, t| {
                this.w(px(seed_w + (width - seed_w) * t))
                    .h(px(seed_h + (height - seed_h) * t))
            },
        )
}

fn picker_scroll_area(
    _id: &'static str,
    width: f32,
    height: f32,
    content: impl IntoElement,
    scrollbar: gpui::Entity<scrollbar::VerticalScrollbar>,
) -> Div {
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .child(
            div()
                .absolute()
                .inset_0()
                .w(px(width))
                .h(px(height))
                .child(content),
        )
        .child(div().absolute().inset_0().child(scrollbar))
}

/// Commit-list pattern: outer overflow clip + inner ellipsis, both with definite width chain.
fn picker_line(
    color: gpui::Rgba,
    primary: bool,
    text: impl Into<gpui::SharedString>,
    cx: &App,
) -> Div {
    div().w_full().min_w(px(0.)).overflow_hidden().child(
        div()
            .w_full()
            .min_w(px(0.))
            .when(primary, |d| d.ui_text_size(14., cx))
            .when(!primary, |d| d.ui_text_size(12., cx))
            .text_color(color)
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .child(text.into()),
    )
}

fn render_files(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mono = appearance::code_font(cx);
    let uncommitted = view.covering();
    let paths = view
        .loaded()
        .map(|loaded| loaded.changed_paths().to_vec())
        .unwrap_or_default();
    let can_open = view.can_open_diff();
    let awaiting_paths = uncommitted && view.uncommitted_paths_pending && paths.is_empty();
    let refresh_pending = uncommitted && view.uncommitted_paths_pending && !paths.is_empty();
    let scan_error = if uncommitted {
        view.uncommitted_scan_error.clone()
    } else {
        None
    };
    let rows = file_tree::flatten(&paths, &view.collapsed_dirs);
    let mr_diff = view.mr_entry.as_ref().and_then(|e| match &e.detail {
        MrDetailState::Ready(d) => {
            let base = d.diff_refs.base_sha.parse::<Oid>().ok()?;
            let head = d.diff_refs.head_sha.parse::<Oid>().ok()?;
            Some((base, head))
        }
        MrDetailState::Loading | MrDetailState::Failed(_) => None,
    });
    let head_meta = if uncommitted {
        None
    } else {
        view.loaded()
            .and_then(|loaded| head_commit_meta(loaded, mr_diff))
    };
    let inset = px(theme::CHANGES_INSET);
    // Commit island is gone, so there is no Commit|Changes gap and no splitter.
    // Left edge is the Commit island's, which is also the kind pills' left edge.
    let slot_w = if uncommitted {
        view.files_width
    } else {
        view.files_width + theme::CHANGES_SHADOW_GAP
    };

    // Slot spans the frost gap + Changes island so the trailing handle can sit
    // between the two floats (outside `#files` overflow_hidden).
    div()
        .id("files-slot")
        .absolute()
        .top(px(theme::CHANGES_TOP_INSET))
        .bottom(inset)
        .w(px(slot_w))
        .when(uncommitted, |d| {
            d.left(px(theme::left_island_inset(!view.repos_collapsed)))
        })
        .when(!uncommitted, |d| d.right(inset))
        .flex()
        .flex_row()
        .when(!uncommitted, |d| {
            d.child(splitter::handle(
                "files-resize-handle",
                Axis::HorizontalTrailing,
                view.files_resize_handler(cx),
                view.files_resize_state.clone(),
                true,
            ))
        })
        .child(
            div()
                .id("files")
                .w(px(view.files_width))
                .h_full()
                .flex()
                .flex_col()
                .bg(theme::white())
                .rounded(px(theme::CHANGES_RADIUS))
                .overflow_hidden()
                // No titlebar — faint section label + Open Diff in the corner.
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .pt_2()
                        .pl_3()
                        .pr(px(if uncommitted { 64. } else { 36. }))
                        .pb_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .ui_text_size(12., cx)
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(theme::faint())
                                        .child(if awaiting_paths {
                                            "Changes".to_string()
                                        } else {
                                            format!("Changes ({})", paths.len())
                                        }),
                                )
                                .when(refresh_pending, |d| {
                                    d.child(loading_row("uncommitted-loading-spin", "Loading", cx))
                                }),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(6.))
                                .right(px(8.))
                                .flex()
                                .gap_1()
                                .when(uncommitted, |d| d.child(refresh_uncommitted_button(cx)))
                                .child(open_diff_button(can_open, cx)),
                        ),
                )
                .when_some(scan_error, |d, message| {
                    d.child(
                        div()
                            .px_3()
                            .py_1()
                            .ui_text_size(12., cx)
                            .text_color(theme::error())
                            .child(message),
                    )
                })
                .when(awaiting_paths, |d| {
                    d.child(
                        div()
                            .id("uncommitted-loading")
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(loading_row("uncommitted-loading-spin", "Loading", cx)),
                    )
                })
                .when(!awaiting_paths, |d| {
                    d.child(render_file_tree(view, &mono, &rows, cx))
                })
                .when_some(head_meta, |d, meta| {
                    d.child(splitter::handle(
                        "head-meta-resize-handle",
                        Axis::Vertical,
                        view.head_meta_resize_handler(cx),
                        view.head_meta_resize_state.clone(),
                        true,
                    ))
                    .child(render_head_meta(
                        &meta,
                        view.files_width - 24.,
                        view.head_meta_height,
                        cx,
                    ))
                }),
        )
}

struct HeadMeta {
    commit: CommitInfo,
    range_label: Option<String>,
}

fn head_commit_meta(loaded: &LoadedBrowser, mr_diff: Option<(Oid, Oid)>) -> Option<HeadMeta> {
    let comparison = loaded.comparison();
    let commit = loaded
        .commits()
        .iter()
        .find(|c| c.oid == comparison.head_oid)
        .cloned()?;
    let selected = loaded.in_range().iter().filter(|&&b| b).count();
    let on_mr_full = mr_diff.is_some_and(|(base, head)| {
        comparison.base_oid == Some(base) && comparison.head_oid == head
    });
    let range_label = if on_mr_full {
        None
    } else if mr_diff.is_some() || selected > 1 {
        Some(comparison.label())
    } else {
        None
    };
    Some(HeadMeta {
        commit,
        range_label,
    })
}

/// `width` is the inner px width: an indefinite width chain lets GPUI shape text
/// at a layout-probe width, collapsing the title to a few glyphs.
fn render_head_meta(
    meta: &HeadMeta,
    width: f32,
    height: f32,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let mono = appearance::code_font(cx);
    let full_oid = meta.commit.oid.to_string();
    let short = meta.commit.oid.short();
    let body = meta.commit.body.clone();
    let has_body = !body.is_empty();

    div()
        .id("head-meta")
        .h(px(height))
        .flex_none()
        .flex()
        .flex_col()
        .px_3()
        .pt_2()
        .pb_2()
        .gap_1()
        .bg(theme::white())
        // Parent overflow_hidden+rounded still paints square at the south edge in
        // GPUI; match capsule radii on the footer so the bottom corners read round.
        .rounded_b(px(theme::CHANGES_RADIUS))
        .child(
            div()
                .w(px(width))
                .flex_none()
                .ui_text_size(12., cx)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(meta.commit.summary.clone()),
        )
        .child(
            div()
                .w(px(width))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_1()
                .ui_text_size(12., cx)
                .text_color(theme::muted())
                .child(
                    div()
                        .id("head-meta-hash")
                        .flex_none()
                        .font_family(mono.clone())
                        // Own size: the UI text around it scales, Code Font chrome does not.
                        .text_xs()
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::accent()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(full_oid.clone()));
                        }))
                        .child(short),
                )
                .child(div().flex_none().child("·"))
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_shrink()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(meta.commit.author.clone()),
                )
                .child(div().flex_none().child("·"))
                .child(div().flex_none().child(meta.commit.time_label.clone()))
                .when_some(meta.range_label.clone(), |d, label| {
                    d.child(div().flex_none().child("·")).child(
                        div()
                            .flex_none()
                            .font_family(mono.clone())
                            // Own size: the UI text around it scales, Code Font chrome does not.
                            .text_xs()
                            .text_color(theme::faint())
                            .child(label),
                    )
                }),
        )
        .when(has_body, |d| {
            let (scroll, sb) = scrollbar::vertical("head-meta-body-sb", cx);
            d.child(scrollbar::overlay_flex(
                div()
                    .id("head-meta-body")
                    .size_full()
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .ui_text_size(12., cx)
                    .text_color(theme::muted())
                    .child(body),
                sb,
            ))
        })
}

enum MrPickerAction {
    None,
    Changed,
    Close,
    Select(MergeRequestSummary),
}

enum MrPickerBody {
    Loading,
    Failed(ErrorNote),
    Ready {
        all: Vec<MergeRequestSummary>,
        matches: Vec<MergeRequestSummary>,
        query: String,
        selected: usize,
    },
}

struct MrPicker {
    body: MrPickerBody,
    /// Capsule window bounds at open — panel top-left locks here and grows over it.
    bounds: Bounds<Pixels>,
}

impl MrPicker {
    fn loading(bounds: Bounds<Pixels>) -> Self {
        Self {
            body: MrPickerBody::Loading,
            bounds,
        }
    }

    fn failed(note: ErrorNote, bounds: Bounds<Pixels>) -> Self {
        Self {
            body: MrPickerBody::Failed(note),
            bounds,
        }
    }

    fn ready(
        all: Vec<MergeRequestSummary>,
        selected_iid: Option<u64>,
        bounds: Bounds<Pixels>,
    ) -> Self {
        let selected = selected_iid
            .and_then(|iid| all.iter().position(|mr| mr.iid == iid))
            .unwrap_or(0);
        Self {
            body: MrPickerBody::Ready {
                matches: all.clone(),
                all,
                query: String::new(),
                selected,
            },
            bounds,
        }
    }

    fn with_bounds(mut self, bounds: Bounds<Pixels>) -> Self {
        self.bounds = bounds;
        self
    }

    fn refresh(&mut self) {
        let MrPickerBody::Ready {
            all,
            matches,
            query,
            selected,
        } = &mut self.body
        else {
            return;
        };
        let q = query.to_lowercase();
        *matches = all
            .iter()
            .filter(|mr| {
                q.is_empty()
                    || mr.title.to_lowercase().contains(&q)
                    || mr.source_branch.to_lowercase().contains(&q)
                    || mr.target_branch.to_lowercase().contains(&q)
                    || mr.iid.to_string().contains(&q)
            })
            .cloned()
            .collect();
        *selected = (*selected).min(matches.len().saturating_sub(1));
    }

    fn handle_key(&mut self, event: &KeyDownEvent) -> MrPickerAction {
        let MrPickerBody::Ready {
            matches,
            query,
            selected,
            ..
        } = &mut self.body
        else {
            return match event.keystroke.key.as_str() {
                "escape" => MrPickerAction::Close,
                _ => MrPickerAction::None,
            };
        };

        match event.keystroke.key.as_str() {
            "escape" => MrPickerAction::Close,
            "backspace" => {
                query.pop();
                self.refresh();
                MrPickerAction::Changed
            }
            "up" => {
                *selected = selected.saturating_sub(1);
                MrPickerAction::Changed
            }
            "down" => {
                *selected = (*selected + 1).min(matches.len().saturating_sub(1));
                MrPickerAction::Changed
            }
            "enter" => matches
                .get(*selected)
                .cloned()
                .map(MrPickerAction::Select)
                .unwrap_or(MrPickerAction::None),
            _ => {
                if event.keystroke.key.len() == 1 && !event.keystroke.modifiers.platform {
                    query.push_str(&event.keystroke.key);
                    self.refresh();
                    MrPickerAction::Changed
                } else {
                    MrPickerAction::None
                }
            }
        }
    }
}

struct BranchPicker {
    all: Vec<BranchInfo>,
    matches: Vec<BranchInfo>,
    query: String,
    selected: usize,
    /// Capsule window bounds at open — panel top-left locks here and grows over it.
    bounds: Bounds<Pixels>,
}

impl BranchPicker {
    fn new(branches: Vec<BranchInfo>, current: &str, bounds: Bounds<Pixels>) -> Self {
        let selected = branches.iter().position(|b| b.name == current).unwrap_or(0);
        Self {
            matches: branches.clone(),
            all: branches,
            query: String::new(),
            selected,
            bounds,
        }
    }

    fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = self
            .all
            .iter()
            .filter(|branch| query.is_empty() || branch.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
    }
}

/// One revolution per 2s, matching Zed's activity-indicator spinner.
/// `animation_id` must be unique per visible instance; GPUI keys animation state by it.
fn loading_row(animation_id: &'static str, label: &'static str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .flex_none()
        .child(
            svg()
                .path("refresh.svg")
                .size(theme::ICON_SIZE_SM)
                .flex_none()
                .text_color(theme::muted())
                .with_animation(
                    animation_id,
                    Animation::new(Duration::from_secs(2)).repeat(),
                    |icon, delta| {
                        icon.with_transformation(Transformation::rotate(percentage(delta)))
                    },
                ),
        )
        .child(
            div()
                .ui_text_size(12., cx)
                .text_color(theme::muted())
                .child(label),
        )
}

fn render_file_tree(
    view: &AppView,
    mono: &gpui::SharedString,
    rows: &[TreeRow],
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let (scroll, sb) = scrollbar::vertical("file-tree-sb", cx);
    let pane_width = view.files_width;
    scrollbar::overlay_flex(
        div()
            .id("file-tree")
            .size_full()
            .px_1()
            .track_scroll(&scroll)
            .overflow_y_scroll()
            .children(rows.iter().enumerate().map(|(i, row)| match row {
                TreeRow::Dir { depth, name, path } => {
                    let collapsed = view.collapsed_dirs.contains(path);
                    let toggle_path = path.clone();
                    file_tree_rows::dir_row(
                        ("dir", i),
                        *depth,
                        name.clone(),
                        collapsed,
                        RowSurface::Island,
                        pane_width,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed_dirs.remove(&toggle_path) {
                            this.collapsed_dirs.insert(toggle_path.clone());
                        }
                        cx.notify();
                    }))
                }
                TreeRow::File { depth, path } => file_tree_rows::file_row(
                    ("file", i),
                    *depth,
                    path,
                    false,
                    RowSurface::Island,
                    mono.clone(),
                    pane_width,
                    cx,
                ),
            })),
        sb,
    )
}

fn refresh_uncommitted_button(cx: &mut Context<AppView>) -> impl IntoElement {
    IconButton::new("refresh-uncommitted", "refresh.svg", "Refresh")
        .on_click(cx.listener(|this, _, _, cx| this.refresh_uncommitted(cx)))
}

fn open_diff_button(enabled: bool, cx: &mut Context<AppView>) -> impl IntoElement {
    IconButton::new("open-diff", "diff_title.svg", "Open Diff")
        .disabled(!enabled)
        .on_click(cx.listener(|this, _, _, cx| {
            this.open_diff(cx);
        }))
}

fn open_repo_button(id: &'static str, cx: &mut Context<AppView>) -> impl IntoElement {
    IconButton::new(id, "folder.svg", "Open Repo").on_click(cx.listener(|this, _, _, cx| {
        this.open_repo(cx);
    }))
}

fn toggle_button(id: &'static str, collapsed: bool, cx: &mut Context<AppView>) -> impl IntoElement {
    let label = if collapsed {
        "Show Repositories"
    } else {
        "Hide Repositories"
    };
    IconButton::new(id, "sidebar_title.svg", label)
        .pressed(collapsed)
        .on_click(cx.listener(|this, _, _, cx| {
            this.repos_collapsed = !this.repos_collapsed;
            window_geometry_store::set_repos_collapsed(this.repos_collapsed);
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

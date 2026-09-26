use gpui::{
    anchored, deferred, App, ClickEvent, ClipboardItem, Context, Corner, Div, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, Render, StatefulInteractiveElement, Styled, TitlebarOptions,
    Window, WindowBounds, WindowControlArea, WindowDecorations, WindowHandle, WindowOptions, div,
    prelude::*, px, rgb, size, svg, Bounds,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use crate::domain::{PathStatus, Repository};
use crate::git::{self, BranchBrowser, BranchInfo, CommitInfo};
use crate::workspace_store::{self, WorkspaceEntry, WorkspaceStore};
use super::diff_window::{DiffSnapshot, DiffView};
use super::file_tree::{self, TreeRow};
use super::scrollbar;
use super::splitter::{self, Axis, ResizeState};
use super::theme;

pub struct AppView {
    focus: FocusHandle,
    repos_collapsed: bool,
    state: MainState,
    /// Workspace set + last + pinned (mirrors disk).
    store: WorkspaceStore,
    diff_window: Option<WindowHandle<DiffView>>,
    branch_picker: Option<BranchPicker>,
    repo_menu: Option<RepoContextMenu>,
    activation_sub: Option<gpui::Subscription>,
    /// Ephemeral; paths in set are collapsed. Default empty = all expanded.
    collapsed_dirs: HashSet<String>,
    /// Reset collapsed_dirs when this no longer matches current ChangedPath list.
    tree_path_fingerprint: Vec<String>,
    sidebar_width: f32,
    sidebar_resize_state: Rc<ResizeState>,
    files_width: f32,
    files_resize_state: Rc<ResizeState>,
    head_meta_height: f32,
    head_meta_resize_state: Rc<ResizeState>,
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

enum MainState {
    Empty,
    Ready(BranchBrowser),
    Error(String),
}

impl AppView {
    pub fn new(boot: Option<BranchBrowser>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            repos_collapsed: false,
            state: match boot {
                Some(loaded) => MainState::Ready(loaded),
                None => MainState::Empty,
            },
            store: workspace_store::load(),
            diff_window: None,
            branch_picker: None,
            repo_menu: None,
            activation_sub: None,
            collapsed_dirs: HashSet::new(),
            tree_path_fingerprint: Vec::new(),
            sidebar_width: splitter::default_sidebar_width(),
            sidebar_resize_state: Rc::new(ResizeState::default()),
            files_width: splitter::default_files_width(),
            files_resize_state: Rc::new(ResizeState::default()),
            head_meta_height: splitter::DEFAULT_HEAD_META_HEIGHT,
            head_meta_resize_state: Rc::new(ResizeState::default()),
        }
    }

    fn sidebar_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |requested, window, cx: &mut App| {
            view.update(cx, |this, cx| {
                let available = f32::from(window.viewport_size().width);
                let width = splitter::clamp_sidebar_width(requested, available);
                if this.sidebar_width != width {
                    this.sidebar_width = width;
                    cx.notify();
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
                if this.head_meta_height != height {
                    this.head_meta_height = height;
                    cx.notify();
                }
            })
            .ok();
        })
    }

    fn sync_collapsed_dirs(&mut self) {
        let next = match &self.state {
            MainState::Ready(loaded) => {
                loaded.changed_paths.iter().map(|p| p.path.clone()).collect()
            }
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
        if let MainState::Ready(loaded) = &self.state {
            workspace_store::remember(
                loaded.comparison.repository.path(),
                &loaded.branch,
            );
            self.refresh_store();
        }
    }

    fn select_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let MainState::Ready(loaded) = &self.state {
            if loaded.comparison.repository.path() == path.as_path() {
                return;
            }
        }
        let branch = self
            .store
            .workspaces
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.branch.clone())
            .unwrap_or_else(|| "HEAD".into());
        match BranchBrowser::open_workspace(&WorkspaceEntry { path: path.clone(), branch }) {
            Ok(bb) => {
                self.state = MainState::Ready(bb);
                self.refresh_store();
                self.branch_picker = None;
            }
            Err(_) => {
                // Stale entry already dropped by open_workspace; keep current Ready if any.
                self.refresh_store();
            }
        }
        cx.notify();
    }

    fn toggle_branch_picker(&mut self, cx: &mut Context<Self>) {
        let (branches, current) = match &self.state {
            MainState::Ready(loaded) => (
                git::list_branches(loaded.comparison.repository.path()).unwrap_or_default(),
                loaded.branch.clone(),
            ),
            MainState::Empty | MainState::Error(_) => (Vec::new(), String::new()),
        };
        self.branch_picker = Some(BranchPicker::new(branches, &current));
        cx.notify();
    }

    fn open_branch(&mut self, name: &str, cx: &mut Context<Self>) {
        let result = match &mut self.state {
            MainState::Ready(bb) => bb.switch_branch(name),
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
            MainState::Ready(loaded) if loaded.comparison.repository.path() == path.as_path()
        );
        workspace_store::drop_path(&path);
        self.refresh_store();
        self.repo_menu = None;
        if removing_current {
            self.state = MainState::Empty;
            self.branch_picker = None;
        }
        cx.notify();
    }

    fn handle_branch_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key.as_str() == "escape" && self.repo_menu.is_some() {
            self.repo_menu = None;
            cx.notify();
            return;
        }
        let Some(picker) = &mut self.branch_picker else { return };
        match event.keystroke.key.as_str() {
            "escape" => self.branch_picker = None,
            "backspace" => { picker.query.pop(); picker.refresh(); picker.selected = 0; }
            "up" => picker.selected = picker.selected.saturating_sub(1),
            "down" => picker.selected = (picker.selected + 1).min(picker.matches.len().saturating_sub(1)),
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
    }

    fn open_diff(&mut self, cx: &mut Context<Self>) {
        let MainState::Ready(loaded) = &self.state else {
            return;
        };
        if loaded.changed_paths.is_empty() {
            return;
        }
        let preferred = self
            .diff_window
            .and_then(|h| {
                h.update(cx, |view, _, _| {
                    view.snapshot
                        .as_ref()
                        .map(|s| s.selected_path.clone())
                })
                .ok()
                .flatten()
            });
        let path = preferred
            .filter(|p| loaded.changed_paths.iter().any(|c| &c.path == p))
            .unwrap_or_else(|| loaded.changed_paths[0].path.clone());
        self.push_diff(path, cx);
    }

    fn push_diff(&mut self, path: String, cx: &mut Context<Self>) {
        let MainState::Ready(loaded) = &self.state else {
            return;
        };
        let status = loaded
            .changed_paths
            .iter()
            .find(|p| p.path == path)
            .map(|p| p.status)
            .unwrap_or(PathStatus::Modify);
        let file = git::file_diff(
            &loaded.comparison,
            &path,
            status,
            &Default::default(),
        );
        let snapshot = DiffSnapshot {
            comparison: loaded.comparison.clone(),
            changed_paths: loaded.changed_paths.clone(),
            selected_path: path,
            file,
        };
        open_or_update_diff(&mut self.diff_window, snapshot, &mut **cx);
        cx.notify();
    }

    fn select_commit(&mut self, index: usize, shift: bool, cx: &mut Context<Self>) {
        let result = {
            let MainState::Ready(bb) = &mut self.state else {
                return;
            };
            bb.select_commit(index, shift)
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
                match BranchBrowser::open(&root) {
                    Ok(bb) => {
                        this.state = MainState::Ready(bb);
                        this.remember_current();
                    }
                    Err(e) => this.state = MainState::Error(e.0),
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
    if let Some(h) = *handle {
        let snap = snapshot.clone();
        if h.update(cx, |view, window, cx| {
            view.apply_snapshot(snap);
            window.activate_window();
            cx.notify();
        })
        .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
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
            window_background: window_background_appearance(),
            ..Default::default()
        },
        move |_, cx| cx.new(|cx| DiffView::with_snapshot(snap, cx)),
    ) {
        Ok(h) => *handle = Some(h),
        Err(e) => eprintln!("failed to open diff window: {e}"),
    }
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn window_background_appearance() -> gpui::WindowBackgroundAppearance {
    gpui::WindowBackgroundAppearance::Blurred
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn window_background_appearance() -> gpui::WindowBackgroundAppearance {
    gpui::WindowBackgroundAppearance::Opaque
}

impl Focusable for AppView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation_sub.is_none() {
            self.activation_sub = Some(cx.observe_window_activation(window, |view, window, cx| {
                if !window.is_window_active()
                    && (view.branch_picker.is_some() || view.repo_menu.is_some())
                {
                    view.branch_picker = None;
                    view.repo_menu = None;
                    cx.notify();
                }
            }));
        }
        self.sync_collapsed_dirs();
        let sidebar_w = if self.repos_collapsed {
            px(0.)
        } else {
            px(self.sidebar_width)
        };
        let show_sidebar_split = !self.repos_collapsed;

        div()
            .id("main")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.handle_branch_key(event, cx)))
            .size_full()
            .flex()
            .overflow_hidden()
            .bg(theme::white())
            .font_family(theme::UI_FONT)
            .track_focus(&self.focus)
            .child(render_sidebar(self, sidebar_w, cx))
            .when(show_sidebar_split, |d| {
                d.child(splitter::handle(
                    "sidebar-resize-handle",
                    Axis::HorizontalLeading,
                    self.sidebar_resize_handler(cx),
                    self.sidebar_resize_state.clone(),
                ))
            })
            // Stage: commits full-bleed; Changes floats on top (capsule-changes A).
            .child(
                div()
                    .id("stage")
                    .relative()
                    .h_full()
                    .flex_1()
                    .min_w(px(splitter::MIN_COMMITS_WIDTH))
                    .overflow_hidden()
                    .child(render_commits(self, cx))
                    .child(render_files(self, cx)),
            )
            .when(self.repo_menu.is_some(), |d| d.child(render_repo_menu(self, cx)))
    }
}

fn render_sidebar(view: &AppView, width: gpui::Pixels, cx: &mut Context<AppView>) -> impl IntoElement {
    let active_path = match &view.state {
        MainState::Ready(loaded) => Some(loaded.comparison.repository.path().to_path_buf()),
        MainState::Empty | MainState::Error(_) => None,
    };
    let recent_row = workspace_store::last_entry(&view.store).map(|e| {
        let active = active_path.as_ref() == Some(&e.path);
        (
            e.path.clone(),
            Repository::new(e.path.clone()).display_name(),
            e.path.display().to_string(),
            active,
        )
    });
    let pinned_rows: Vec<(PathBuf, String, String, bool)> = workspace_store::pinned_entries(&view.store)
        .into_iter()
        .map(|e| {
            let active = active_path.as_ref() == Some(&e.path);
            (
                e.path.clone(),
                Repository::new(e.path.clone()).display_name(),
                e.path.display().to_string(),
                active,
            )
        })
        .collect();
    let pin_section = (!pinned_rows.is_empty()).then_some(pinned_rows);
    let repos: Vec<(PathBuf, String, String, bool)> = workspace_store::repository_entries(&view.store)
        .into_iter()
        .map(|e| {
            let active = active_path.as_ref() == Some(&e.path);
            (
                e.path.clone(),
                Repository::new(e.path.clone()).display_name(),
                e.path.display().to_string(),
                active,
            )
        })
        .collect();

    div()
        .id("repos")
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
                .child(toggle_button("main-sidebar-toggle", false, cx))
                .child(open_repo_button("open-repo", cx))
                .child(
                    div()
                        .id("main-drag")
                        .h_full()
                        .flex_1()
                        .window_control_area(WindowControlArea::Drag),
                ),
        )
        .child({
            let (scroll, sb) = scrollbar::vertical("sidebar-repos-sb", cx);
            scrollbar::overlay_flex(
                div()
                    .id("sidebar-repos-scroll")
                    .size_full()
                    .px_2()
                    .pt_1()
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme::faint())
                            .child("Recent"),
                    )
                    .children(recent_row.into_iter().map(|(path, name, path_str, active)| {
                        sidebar_repo_row("recent-repo", path, name, path_str, active, None, cx)
                    }))
                    .when_some(pin_section, |d, pinned_rows| {
                        d.child(
                            div()
                                .px_2()
                                .pt_1()
                                .pb_1()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme::faint())
                                .child("Pin"),
                        )
                        .children(pinned_rows.into_iter().enumerate().map(
                            |(i, (path, name, path_str, active))| {
                                sidebar_repo_row(
                                    ("pin-repo", i),
                                    path,
                                    name,
                                    path_str,
                                    active,
                                    Some(RepoMenuKind::Pin),
                                    cx,
                                )
                            },
                        ))
                    })
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .pb_1()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme::faint())
                            .child("Repositories"),
                    )
                    .children(repos.into_iter().enumerate().map(|(i, (path, name, path_str, active))| {
                        sidebar_repo_row(
                            ("repo", i),
                            path,
                            name,
                            path_str,
                            active,
                            Some(RepoMenuKind::Repositories),
                            cx,
                        )
                    })),
                sb,
            )
        })
}

fn sidebar_repo_row(
    id: impl Into<gpui::ElementId>,
    path: PathBuf,
    name: String,
    path_str: String,
    active: bool,
    menu: Option<RepoMenuKind>,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let path_click = path.clone();
    let path_menu = path.clone();
    div()
        .id(id)
        .mb_1()
        .px_2()
        .py_1()
        .rounded_lg()
        .min_w(px(0.))
        .overflow_hidden()
        .cursor_pointer()
        .when(active, |d| d.bg(theme::capsule()))
        .hover(|d| d.bg(theme::hover()))
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
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(name),
        )
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_xs()
                .text_color(theme::muted())
                .child(path_str),
        )
        .into_any_element()
}

fn render_repo_menu(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(menu) = &view.repo_menu else {
        return div().into_any_element();
    };
    let path = menu.path.clone();
    let items: Vec<(&str, fn(&mut AppView, PathBuf, &mut Context<AppView>))> = match menu.kind {
        RepoMenuKind::Pin => vec![
            ("Unpin", AppView::unpin_repo),
            ("Remove", AppView::remove_repo),
        ],
        RepoMenuKind::Repositories => vec![
            ("Pin", AppView::pin_repo),
            ("Remove", AppView::remove_repo),
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
                    .children(items.into_iter().enumerate().map(|(i, (label, action))| {
                        let path = path.clone();
                        div()
                            .id(("repo-menu-item", i))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::hover()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                action(this, path.clone(), cx);
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::text())
                                    .child(label),
                            )
                    })),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

fn render_commits(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let (branch, label) = match &view.state {
        MainState::Ready(loaded) => (loaded.branch.clone(), loaded.comparison.label()),
        MainState::Empty | MainState::Error(_) => ("—".into(), "—".into()),
    };
    // Definite right edge so text_ellipsis sees the pane width (same idea as
    // overlay_flex absolute inset). Leave shadow clearance so the thumb does
    // not sit under the Changes cast.
    let float_gap = px(theme::changes_float_clearance(view.files_width));

    div()
        .id("commits")
        .absolute()
        .inset_0()
        .right(float_gap)
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
                .when(view.repos_collapsed, |row| {
                    // Keep toggle at original spot (after traffic lights); branch shifts right.
                    row.pl(px(12.))
                        .pr_3()
                        .children(traffic_lights_space())
                        .child(toggle_button("main-sidebar-toggle-collapsed", true, cx))
                        .child(open_repo_button("open-repo-collapsed", cx))
                })
                .when(!view.repos_collapsed, |row| row.px_3())
                .child(
                    div()
                        .id("branch-picker-toggle")
                        .px_1()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::hover()))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_branch_picker(cx)))
                        .child(
                            svg()
                                .size_4()
                                .flex_none()
                                .path("branch.svg")
                                .text_color(theme::muted()),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(theme::text())
                                .child(branch),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .font_family(theme::MONO_FONT)
                        .text_xs()
                        .text_color(theme::muted())
                        .child(label),
                ),
        )
        .child(match &view.state {
            MainState::Empty => div().flex_1().into_any_element(),
            MainState::Error(msg) => div()
                .flex_1()
                .px_3()
                .py_3()
                .text_sm()
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
                        .track_scroll(&scroll)
                        .overflow_y_scroll()
                        .children(loaded.commits.iter().enumerate().map(|(i, commit)| {
                            let in_range = loaded.in_range.get(i).copied().unwrap_or(false);
                            let summary = commit.summary.clone();
                            let meta = format!(
                                "{} · {} · {}",
                                commit.oid.short(),
                                commit.author,
                                commit.time_label
                            );
                            div()
                                .id(("commit", i))
                                .mx_2()
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
                                        d.bg(rgb(0xf6f8fb))
                                    }
                                })
                                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                    this.select_commit(i, event.modifiers().shift, cx);
                                }))
                                .child(
                                    div()
                                        .w_full()
                                        .min_w(px(0.))
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .w_full()
                                                .min_w(px(0.))
                                                .text_sm()
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
                                                .font_family(theme::MONO_FONT)
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
        })
        .when(view.branch_picker.is_some(), |d| {
            // Ponytail: last child paints last → popover above the commit list
            d.child(render_branch_picker(view, cx))
        })
}

fn render_branch_picker(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(picker) = &view.branch_picker else {
        return div().into_any_element();
    };
    let (scroll, sb) = scrollbar::vertical("branch-picker-sb", cx);
    div()
        .id("branch-picker")
        .absolute()
        .top(theme::CHROME_HEIGHT)
        .left(px(4.))
        .w(px(320.))
        .h(px(420.))
        .p_1()
        .bg(theme::white())
        .rounded_lg()
        .shadow_lg()
        // block clicks from reaching the commit list beneath; close on outside click
        .occlude()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.branch_picker = None;
            cx.notify();
        }))
        .child(scrollbar::overlay_box(
            div()
                .id("branch-picker-scroll")
                .size_full()
                .track_scroll(&scroll)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(format!("Filter: {}", picker.query)),
                )
                .children(picker.matches.iter().enumerate().map(|(i, branch)| {
                    let name = branch.name.clone();
                    div()
                        .id(("branch", i))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .when(i == picker.selected, |d| d.bg(theme::range()))
                        .hover(|d| d.bg(theme::hover()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_branch(&name, cx);
                        }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::text())
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(branch.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(format!(
                                    "{} · {} · {}",
                                    branch.tip.author, branch.tip.time_label, branch.tip.summary
                                )),
                        )
                })),
            sb,
        ))
        .into_any_element()
}

fn render_files(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let paths = match &view.state {
        MainState::Ready(loaded) => loaded.changed_paths.clone(),
        MainState::Empty | MainState::Error(_) => Vec::new(),
    };
    let can_open = !paths.is_empty();
    let rows = file_tree::flatten(&paths, &view.collapsed_dirs);
    let head_meta = match &view.state {
        MainState::Ready(loaded) => head_commit_meta(loaded),
        MainState::Empty | MainState::Error(_) => None,
    };
    let inset = px(theme::CHANGES_INSET);

    div()
        .id("files")
        .absolute()
        .top(inset)
        .right(inset)
        .bottom(inset)
        .w(px(view.files_width))
        .flex()
        .flex_col()
        .bg(theme::white())
        .rounded(px(theme::CHANGES_RADIUS))
        .shadow(theme::changes_capsule_shadow())
        .overflow_hidden()
        // Left-edge resize (HorizontalTrailing measures from viewport right).
        .child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .bottom(px(0.))
                .w(px(5.))
                .child(splitter::handle(
                    "files-resize-handle",
                    Axis::HorizontalTrailing,
                    view.files_resize_handler(cx),
                    view.files_resize_state.clone(),
                )),
        )
        // No titlebar — faint section label + Open Diff in the corner.
        .child(
            div()
                .relative()
                .flex_none()
                .pt_2()
                .pl_3()
                .pr(px(36.))
                .pb_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child(format!("Changes ({})", paths.len())),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .right(px(8.))
                        .child(open_diff_button(can_open, cx)),
                ),
        )
        .child({
            let (scroll, sb) = scrollbar::vertical("file-tree-sb", cx);
            scrollbar::overlay_flex(
                div()
                    .id("file-tree")
                    .size_full()
                    .px_1()
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .children(rows.into_iter().enumerate().map(|(i, row)| match row {
                    TreeRow::Dir { depth, name, path } => {
                        let collapsed = view.collapsed_dirs.contains(&path);
                        let toggle_path = path.clone();
                        div()
                            .id(("dir", i))
                            .mx_1()
                            .h(px(22.))
                            .pl(px(8. + depth as f32 * 12.))
                            .pr_1()
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
                        let status = path.status;
                        let name = path.file_name().to_string();
                        let add = path.additions;
                        let del = path.deletions;
                        div()
                            .id(("file", i))
                            .mx_1()
                            .h(px(22.))
                            .pl(px(8. + depth as f32 * 12.))
                            .pr_1()
                            .rounded_lg()
                            .flex()
                            .items_center()
                            .gap_1()
                            .hover(|d| d.bg(rgb(0xf6f8fb)))
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
        .when_some(head_meta, |d, meta| {
            d.child(splitter::handle(
                "head-meta-resize-handle",
                Axis::Vertical,
                view.head_meta_resize_handler(cx),
                view.head_meta_resize_state.clone(),
            ))
            .child(render_head_meta(&meta, view.head_meta_height, cx))
        })
}

struct HeadMeta {
    commit: CommitInfo,
    range_label: Option<String>,
}

fn head_commit_meta(loaded: &BranchBrowser) -> Option<HeadMeta> {
    let commit = loaded
        .commits
        .iter()
        .find(|c| c.oid == loaded.comparison.head_oid)
        .cloned()?;
    let selected = loaded.in_range.iter().filter(|&&b| b).count();
    let range_label = (selected > 1).then(|| loaded.comparison.label());
    Some(HeadMeta { commit, range_label })
}

fn render_head_meta(meta: &HeadMeta, height: f32, cx: &mut Context<AppView>) -> impl IntoElement {
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
        .border_t_1()
        .border_color(theme::line())
        .bg(theme::white())
        // Match capsule bottom radii (parent overflow clip alone still reads square).
        .rounded_b(px(theme::CHANGES_RADIUS))
        .child(
            div()
                .min_w(px(0.))
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .overflow_hidden()
                .text_ellipsis()
                .child(meta.commit.summary.clone()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(theme::muted())
                .child(
                    div()
                        .id("head-meta-hash")
                        .font_family(theme::MONO_FONT)
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::accent()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(full_oid.clone()));
                        }))
                        .child(short),
                )
                .child(div().child("·"))
                .child(div().overflow_hidden().text_ellipsis().child(meta.commit.author.clone()))
                .child(div().child("·"))
                .child(div().child(meta.commit.time_label.clone()))
                .when_some(meta.range_label.clone(), |d, label| {
                    d.child(div().child("·")).child(
                        div()
                            .font_family(theme::MONO_FONT)
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
                    .text_xs()
                    .text_color(theme::muted())
                    .child(body),
                sb,
            ))
        })
}

struct BranchPicker {
    all: Vec<BranchInfo>,
    matches: Vec<BranchInfo>,
    query: String,
    selected: usize,
}

impl BranchPicker {
    fn new(branches: Vec<BranchInfo>, current: &str) -> Self {
        let selected = branches.iter().position(|b| b.name == current).unwrap_or(0);
        Self { matches: branches.clone(), all: branches, query: String::new(), selected }
    }

    fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = self.all.iter().filter(|branch| {
            query.is_empty() || branch.name.to_lowercase().contains(&query)
        }).cloned().collect();
    }
}

fn open_diff_button(enabled: bool, cx: &mut Context<AppView>) -> impl IntoElement {
    // BeadsViewer toolbar control UX (hit target + hover/active); glyph is dual-pane Diff.
    div()
        .id("open-diff")
        .w(theme::TOGGLE_SIZE)
        .h(theme::TOGGLE_SIZE)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .text_color(if enabled {
            theme::muted()
        } else {
            theme::faint()
        })
        .tooltip(|_, cx| cx.new(|_| OpenDiffTooltip).into())
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|button| button.bg(theme::hover()))
                .active(|button| button.bg(rgb(0xdfe3e9)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_diff(cx);
                }))
                .child(
                    svg()
                        .size_4()
                        .path("diff_title.svg")
                        .text_color(theme::muted()),
                )
        })
        .when(!enabled, |button| {
            button.child(
                svg()
                    .size_4()
                    .path("diff_title.svg")
                    .text_color(theme::faint()),
            )
        })
}

struct OpenDiffTooltip;

impl Render for OpenDiffTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x273142))
            .text_xs()
            .text_color(theme::white())
            .child("Open Diff")
    }
}

struct OpenRepoTooltip;

impl Render for OpenRepoTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x273142))
            .text_xs()
            .text_color(theme::white())
            .child("Open Repo")
    }
}

fn open_repo_button(id: &'static str, cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id(id)
        .w(theme::TOGGLE_SIZE)
        .h(theme::TOGGLE_SIZE)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_color(theme::muted())
        .tooltip(|_, cx| cx.new(|_| OpenRepoTooltip).into())
        .hover(|button| button.bg(theme::hover()))
        .active(|button| button.bg(rgb(0xdfe3e9)))
        .on_click(cx.listener(|this, _, _, cx| {
            this.open_repo(cx);
        }))
        .child(
            svg()
                .size_4()
                .path("folder.svg")
                .text_color(theme::muted()),
        )
}

fn toggle_button(
    id: &'static str,
    collapsed: bool,
    cx: &mut Context<AppView>,
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
            this.repos_collapsed = !this.repos_collapsed;
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

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, KeyBinding, Render,
    ScrollHandle, SharedString, Window, WindowBackgroundAppearance, WindowHandle, actions, div,
    point, prelude::*, px, size,
};

use std::path::PathBuf;

use crate::gitlab::{self, ResolveProjectResult, VerifyError, VerifyResult};
use crate::settings_store::{self, SettingsFile};
use std::cell::RefCell;
use std::rc::Rc;

use super::SectionHeader;
use super::nav::{NavItem, SettingsNav};
use super::nav_tree::{NavEntry, NavPage, NavState};
use crate::ui::current_repo;
use crate::ui::gitlab_connection::{self, GitLabConnection};
use crate::ui::scrollbar;
use crate::ui::text_field::TextField;
use crate::ui::theme;

actions!(
    settings,
    [
        SaveSettings,
        VerifyGitLab,
        ClearPat,
        RefreshGitLabProject,
        CloseSettings,
        FocusNextControl,
        FocusPrevControl,
        NavUp,
        NavDown,
        NavExpand,
        NavCollapse,
    ]
);

/// Key context on the whole Settings view.
const CONTEXT: &str = "Settings";
/// Key context on the nav tree (only while it has focus).
const NAV_CONTEXT: &str = "SettingsNav";
const CONTENT_SCROLL_ID: &str = "settings-content-sb";

#[cfg(target_os = "macos")]
const CLOSE_KEY: &str = "cmd-w";
#[cfg(not(target_os = "macos"))]
const CLOSE_KEY: &str = "ctrl-w";

/// Settings-window bindings, scoped to [`CONTEXT`] / [`NAV_CONTEXT`].
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("escape", CloseSettings, Some(CONTEXT)),
        KeyBinding::new(CLOSE_KEY, CloseSettings, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNextControl, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevControl, Some(CONTEXT)),
        KeyBinding::new("up", NavUp, Some(NAV_CONTEXT)),
        KeyBinding::new("down", NavDown, Some(NAV_CONTEXT)),
        KeyBinding::new("right", NavExpand, Some(NAV_CONTEXT)),
        KeyBinding::new("left", NavCollapse, Some(NAV_CONTEXT)),
    ]
}

/// Level-2 nav entries; each renders one block of its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    GitLab,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::GitLab => "GitLab",
        }
    }
}

/// The nav tree. A new page or section is a new entry here plus its render arm.
const PAGES: &[NavPage<Section>] = &[NavPage {
    title: "Accounts",
    sections: &[Section::GitLab],
    expanded: true,
}];

#[derive(Clone, Debug, PartialEq, Eq)]
enum VerifyStatus {
    Idle,
    Running,
    Ok(String),
    Network(String),
    Unauthorized,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GitLabProjectLine {
    Idle,
    NoRepo,
    Resolving,
    Ok { path: String, verified: bool },
    Err(String),
}

pub struct SettingsView {
    focus: FocusHandle,
    base_url: Entity<TextField>,
    pat: Entity<TextField>,
    status: SharedString,
    verify: VerifyStatus,
    gitlab_project: GitLabProjectLine,
    gitlab_connection: Rc<RefCell<GitLabConnection>>,
    nav_focus: FocusHandle,
    nav: NavState,
    /// Last nav interaction came from the keyboard: show the focus border.
    nav_keyboard: bool,
    content_scroll: ScrollHandle,
}

impl SettingsView {
    pub fn new(gitlab_connection: Rc<RefCell<GitLabConnection>>, cx: &mut Context<Self>) -> Self {
        let file = settings_store::load_file();
        let base = settings_store::effective_base_url(&file);
        let base_url = cx.new(|cx| TextField::new("https://gitlab.com", false, cx).tab_index(1));
        base_url.update(cx, |field, cx| field.set_content(base, cx));

        let pat =
            cx.new(|cx| TextField::new("Personal access token (read_api)", true, cx).tab_index(2));
        if let Some(stored) = settings_store::load_pat() {
            pat.update(cx, |field, cx| field.set_content(stored, cx));
        }

        // The scroll handle lives in a global registry and outlives the window.
        let (content_scroll, _) = scrollbar::vertical(CONTENT_SCROLL_ID, cx);
        content_scroll.set_offset(point(px(0.), px(0.)));

        let mut view = Self {
            focus: cx.focus_handle(),
            base_url,
            pat,
            status: "Save does not contact GitLab; use Verify to test credentials.".into(),
            verify: VerifyStatus::Idle,
            gitlab_project: GitLabProjectLine::Idle,
            gitlab_connection,
            nav_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            nav: NavState::new(PAGES),
            nav_keyboard: false,
            content_scroll,
        };
        view.refresh_gitlab_project(current_repo::current_repo_path(), cx);
        gitlab_connection::spawn_refresh_connection(
            view.gitlab_connection.clone(),
            cx.entity().downgrade(),
            cx,
        );
        view
    }

    pub fn refresh_gitlab_project(&mut self, repo_path: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(repo_path) = repo_path else {
            self.gitlab_project = GitLabProjectLine::NoRepo;
            cx.notify();
            return;
        };
        if matches!(self.gitlab_project, GitLabProjectLine::Resolving) {
            return;
        }
        let base = settings_store::normalize_base_url(self.base_url.read(cx).content());
        let pat = self.pat.read(cx).content().to_string();
        self.gitlab_project = GitLabProjectLine::Resolving;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let http = match cx.update(|app| app.http_client()) {
                Ok(client) => client,
                Err(_) => return,
            };
            let had_pat = !pat.trim().is_empty();
            let result = cx
                .background_executor()
                .spawn(async move { gitlab::resolve_project(http, &base, &pat, &repo_path).await })
                .await;

            let _ = this.update(cx, |view, cx| {
                view.gitlab_project = match result {
                    ResolveProjectResult::Ok(identity) => GitLabProjectLine::Ok {
                        path: identity.path_with_namespace,
                        verified: had_pat,
                    },
                    ResolveProjectResult::Err(e) => {
                        GitLabProjectLine::Err(gitlab::format_resolve_project_error(&e))
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let raw_base = self.base_url.read(cx).content().to_string();
        let normalized = settings_store::normalize_base_url(&raw_base);
        self.base_url
            .update(cx, |field, cx| field.set_content(normalized.clone(), cx));

        let pat = self.pat.read(cx).content().to_string();
        settings_store::save_file(&SettingsFile {
            gitlab_base_url: Some(normalized),
        });

        let pat_result = if pat.is_empty() {
            settings_store::clear_pat()
        } else {
            settings_store::save_pat(&pat)
        };

        self.status = match pat_result {
            Ok(()) => "Saved.".into(),
            Err(e) => format!("Saved URL; keychain error: {e}").into(),
        };
        self.verify = VerifyStatus::Idle;
        gitlab_connection::spawn_refresh_connection(
            self.gitlab_connection.clone(),
            cx.entity().downgrade(),
            cx,
        );
        cx.notify();
    }

    fn clear_pat(&mut self, cx: &mut Context<Self>) {
        self.pat.update(cx, |field, cx| field.set_content("", cx));
        if let Err(e) = settings_store::clear_pat() {
            self.status = format!("Could not clear keychain entry: {e}").into();
        } else {
            self.status = "Personal access token cleared.".into();
        }
        self.verify = VerifyStatus::Idle;
        *self.gitlab_connection.borrow_mut() = GitLabConnection::NoPat;
        self.refresh_gitlab_project(current_repo::current_repo_path(), cx);
        cx.notify();
    }

    fn verify(&mut self, _: &VerifyGitLab, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.verify, VerifyStatus::Running) {
            return;
        }
        let base = settings_store::normalize_base_url(self.base_url.read(cx).content());
        let pat = self.pat.read(cx).content().to_string();
        self.verify = VerifyStatus::Running;
        self.status = "Verifying…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let http = match cx.update(|app| app.http_client()) {
                Ok(client) => client,
                Err(_) => return,
            };
            let result = cx
                .background_executor()
                .spawn(async move { gitlab::verify_pat(http, &base, &pat).await })
                .await;

            let _ = this.update(cx, |view, cx| {
                gitlab_connection::apply_verify_result(&view.gitlab_connection, &result);
                view.verify = match &result {
                    VerifyResult::Ok { username } => VerifyStatus::Ok(username.clone()),
                    VerifyResult::Err(VerifyError::Network(msg)) => {
                        VerifyStatus::Network(msg.clone())
                    }
                    VerifyResult::Err(VerifyError::Unauthorized) => VerifyStatus::Unauthorized,
                    VerifyResult::Err(VerifyError::Other { status, detail }) => {
                        VerifyStatus::Other(format!("HTTP {status}: {detail}"))
                    }
                };
                view.status = match &view.verify {
                    VerifyStatus::Idle => SharedString::default(),
                    VerifyStatus::Running => "Verifying…".into(),
                    VerifyStatus::Ok(user) => format!("Verified as {user}.").into(),
                    VerifyStatus::Network(msg) => format!("Network error: {msg}").into(),
                    VerifyStatus::Unauthorized => {
                        "Unauthorized (401). Check token and scopes.".into()
                    }
                    VerifyStatus::Other(msg) => msg.clone().into(),
                };
                cx.notify();
            });
        })
        .detach();
    }
}

// Nav tree and window keyboard.
impl SettingsView {
    /// Scroll the content to the selected entry: a page to the top, a section
    /// to its header. Child 0 of the scroll column is the page title.
    fn reveal_selected(&self) {
        match self.nav.selected() {
            NavEntry::Page(_) => self.content_scroll.set_offset(point(px(0.), px(0.))),
            NavEntry::Section { section, .. } => {
                self.content_scroll.scroll_to_top_of_item(section + 1)
            }
        }
    }

    fn click_nav(&mut self, entry: NavEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.nav.select(entry);
        self.nav_keyboard = false;
        window.focus(&self.nav_focus);
        self.reveal_selected();
        cx.notify();
    }

    fn toggle_nav(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.nav.toggle(page) {
            self.reveal_selected();
        }
        cx.notify();
    }

    /// Arrow keys in the tree; `moved` is what the [`NavState`] move returned.
    fn nav_key(&mut self, moved: bool, cx: &mut Context<Self>) {
        self.nav_keyboard = true;
        if moved {
            self.reveal_selected();
        }
        cx.notify();
    }

    fn focus_control(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if next {
            window.focus_next();
        } else {
            window.focus_prev();
        }
        self.nav_keyboard = self.nav_focus.is_focused(window);
        cx.notify();
    }

    fn render_nav(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.nav.selected();
        let focused = self.nav_keyboard && self.nav_focus.is_focused(window);
        let items = self
            .nav
            .visible()
            .into_iter()
            .map(|entry| {
                let item = match entry {
                    NavEntry::Page(page) => NavItem::page(
                        ("settings-nav-page", page),
                        PAGES[page].title,
                        self.nav.is_expanded(page),
                    )
                    .on_toggle(cx.listener(move |view, _, _, cx| view.toggle_nav(page, cx))),
                    NavEntry::Section { page, section } => NavItem::section(
                        ElementId::Name(format!("settings-nav-{page}-{section}").into()),
                        PAGES[page].sections[section].title(),
                    ),
                };
                item.selected(entry == selected)
                    .focused(focused && entry == selected)
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.click_nav(entry, window, cx)),
                    )
            })
            .collect();

        div()
            .id("settings-nav")
            .key_context(NAV_CONTEXT)
            .track_focus(&self.nav_focus)
            .flex_none()
            .h_full()
            .on_action(cx.listener(|view, _: &NavUp, _, cx| {
                let moved = view.nav.select_prev();
                view.nav_key(moved, cx);
            }))
            .on_action(cx.listener(|view, _: &NavDown, _, cx| {
                let moved = view.nav.select_next();
                view.nav_key(moved, cx);
            }))
            .on_action(cx.listener(|view, _: &NavExpand, _, cx| {
                let moved = view.nav.expand();
                view.nav_key(moved, cx);
            }))
            .on_action(cx.listener(|view, _: &NavCollapse, _, cx| {
                let moved = view.nav.collapse();
                view.nav_key(moved, cx);
            }))
            .child(SettingsNav::new(items))
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = &PAGES[self.nav.selected().page()];
        let (scroll, scrollbar) = scrollbar::vertical(CONTENT_SCROLL_ID, cx);
        let sections: Vec<_> = page
            .sections
            .iter()
            .map(|&section| {
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .child(SectionHeader::new(section.title()))
                    .child(match section {
                        Section::GitLab => self.render_gitlab_section(cx),
                    })
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(theme::white())
            .child(scrollbar::overlay_flex(
                div()
                    .id("settings-content")
                    .size_full()
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .px(px(32.))
                    .pt(px(24.))
                    .child(
                        div()
                            .flex_none()
                            .pt(px(8.))
                            .pb(px(12.))
                            .text_size(px(16.))
                            .text_color(theme::text())
                            .child(page.title),
                    )
                    .children(sections),
                scrollbar,
            ))
    }

    /// The pre-redesign GitLab form, unchanged until settings-redesign issue 03.
    fn render_gitlab_section(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .pt_3()
            .pb(px(40.))
            .child(field_label("GitLab Base URL"))
            .child(self.base_url.clone())
            .child(field_label("Personal Access Token"))
            .child(self.pat.clone())
            .child(field_label("GitLab project (open repo)"))
            .child(gitlab_project_line(&self.gitlab_project))
            .child(field_label("Connection"))
            .child(connection_line(&self.gitlab_connection.borrow()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .id("settings-save")
                            .px_3()
                            .py_1()
                            .rounded(px(6.))
                            .bg(theme::line())
                            .text_size(px(13.))
                            .cursor_pointer()
                            .child("Save")
                            .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
                    )
                    .child(
                        div()
                            .id("settings-verify")
                            .px_3()
                            .py_1()
                            .rounded(px(6.))
                            .bg(theme::line())
                            .text_size(px(13.))
                            .cursor_pointer()
                            .when(matches!(self.verify, VerifyStatus::Running), |el| {
                                el.opacity(0.6)
                            })
                            .child("Verify")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.verify(&VerifyGitLab, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("settings-clear-pat")
                            .px_3()
                            .py_1()
                            .rounded(px(6.))
                            .bg(theme::line())
                            .text_size(px(13.))
                            .cursor_pointer()
                            .child("Clear token")
                            .on_click(cx.listener(|view, _, _, cx| view.clear_pat(cx))),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::muted())
                    .child(self.status.clone()),
            )
            .into_any_element()
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_row()
            .size_full()
            .bg(theme::white())
            .font_family(theme::UI_FONT)
            .text_color(theme::text())
            .child(self.render_nav(window, cx))
            .child(self.render_content(cx))
            .on_action(cx.listener(|_, _: &CloseSettings, window, _| window.remove_window()))
            .on_action(cx.listener(|view, _: &FocusNextControl, window, cx| {
                view.focus_control(true, window, cx)
            }))
            .on_action(cx.listener(|view, _: &FocusPrevControl, window, cx| {
                view.focus_control(false, window, cx)
            }))
            .on_action(cx.listener(|view, _: &SaveSettings, _, cx| view.save(cx)))
            .on_action(cx.listener(SettingsView::verify))
            .on_action(cx.listener(|view, _: &ClearPat, _, cx| view.clear_pat(cx)))
            .on_action(cx.listener(|view, _: &RefreshGitLabProject, _, cx| {
                view.refresh_gitlab_project(current_repo::current_repo_path(), cx);
            }))
    }
}

fn connection_line(state: &GitLabConnection) -> impl IntoElement {
    let text = match state {
        GitLabConnection::Idle => "Not checked yet.".into(),
        GitLabConnection::NoPat => "No token saved. Add a PAT and Verify.".into(),
        GitLabConnection::Checking => "Checking credentials…".into(),
        GitLabConnection::Connected { username } => format!("Signed in as {username}."),
        GitLabConnection::Failed(msg) => format!("Connection failed: {msg}"),
    };
    div()
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(text)
}

fn gitlab_project_line(line: &GitLabProjectLine) -> impl IntoElement {
    let text = match line {
        GitLabProjectLine::Idle => "Open a repository, then reopen Settings.".into(),
        GitLabProjectLine::NoRepo => "No repository open.".into(),
        GitLabProjectLine::Resolving => "Resolving…".into(),
        GitLabProjectLine::Ok {
            path,
            verified: true,
        } => format!("GitLab project: {path}"),
        GitLabProjectLine::Ok {
            path,
            verified: false,
        } => {
            format!("GitLab project: {path} (add PAT and Save to verify via API)")
        }
        GitLabProjectLine::Err(msg) => msg.clone(),
    };
    div()
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(text)
}

fn field_label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(text)
}

pub fn open_or_focus_settings(
    handle: &mut Option<WindowHandle<SettingsView>>,
    gitlab_connection: Rc<RefCell<GitLabConnection>>,
    cx: &mut App,
) {
    if let Some(h) = *handle {
        if h.update(cx, |view, window, cx| {
            window.activate_window();
            view.refresh_gitlab_project(current_repo::current_repo_path(), cx);
            cx.notify();
        })
        .is_ok()
        {
            return;
        }
    }

    let bounds = gpui::Bounds::centered(None, size(px(760.), px(520.)), cx);
    match cx.open_window(
        gpui::WindowOptions {
            focus: true,
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("ReviewFox Settings".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            window_min_size: Some(size(px(640.), px(400.))),
            // Opaque nav + white content; no vibrancy layer to show through.
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| SettingsView::new(gitlab_connection.clone(), cx));
            // Arrow keys work in the tree straight away; no focus border until used.
            let nav_focus = view.read(cx).nav_focus.clone();
            window.focus(&nav_focus);
            view
        },
    ) {
        Ok(h) => {
            *handle = Some(h);
            let repo = current_repo::current_repo_path();
            let _ = h.update(cx, |view, _, cx| view.refresh_gitlab_project(repo, cx));
        }
        Err(e) => eprintln!("failed to open settings: {e}"),
    }
}

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window, WindowHandle,
    actions, div, prelude::*, px,
};

use std::path::PathBuf;

use crate::gitlab::{self, ResolveProjectResult, VerifyError, VerifyResult};
use crate::settings_store::{self, SettingsFile};
use std::cell::RefCell;
use std::rc::Rc;

use super::current_repo;
use super::gitlab_connection::{self, GitLabConnection};
use super::text_field::TextField;
use super::theme;

actions!(settings, [SaveSettings, VerifyGitLab, ClearPat, RefreshGitLabProject]);

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
}

impl SettingsView {
    pub fn new(gitlab_connection: Rc<RefCell<GitLabConnection>>, cx: &mut Context<Self>) -> Self {
        let file = settings_store::load_file();
        let base = settings_store::effective_base_url(&file);
        let base_url = cx.new(|cx| TextField::new("https://gitlab.com", false, cx));
        base_url.update(cx, |field, cx| field.set_content(base, cx));

        let pat = cx.new(|cx| {
            TextField::new("Personal access token (read_api)", true, cx)
        });
        if let Some(stored) = settings_store::load_pat() {
            pat.update(cx, |field, cx| field.set_content(stored, cx));
        }

        let mut view = Self {
            focus: cx.focus_handle(),
            base_url,
            pat,
            status: "Save does not contact GitLab; use Verify to test credentials.".into(),
            verify: VerifyStatus::Idle,
            gitlab_project: GitLabProjectLine::Idle,
            gitlab_connection,
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
        self.pat
            .update(cx, |field, cx| field.set_content("", cx));
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

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("settings")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .size_full()
            .bg(theme::white())
            .font_family(theme::UI_FONT)
            .text_color(theme::text())
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
        GitLabProjectLine::Ok { path, verified: true } => format!("GitLab project: {path}"),
        GitLabProjectLine::Ok { path, verified: false } => {
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

    let bounds = gpui::Bounds::centered(None, gpui::size(px(480.), px(420.)), cx);
    match cx.open_window(
        gpui::WindowOptions {
            focus: true,
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("ReviewFox Settings".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            window_background: super::window_background_appearance(),
            ..Default::default()
        },
        |_, cx| cx.new(|cx| SettingsView::new(gitlab_connection.clone(), cx)),
    ) {
        Ok(h) => {
            *handle = Some(h);
            let repo = current_repo::current_repo_path();
            let _ = h.update(cx, |view, _, cx| view.refresh_gitlab_project(repo, cx));
        }
        Err(e) => eprintln!("failed to open settings: {e}"),
    }
}

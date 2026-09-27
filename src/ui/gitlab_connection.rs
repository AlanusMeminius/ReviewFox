//! Shared GitLab auth display state (verify via `/user`, cached for shell + Settings).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, WeakEntity};

use crate::gitlab::{VerifyError, VerifyResult, verify_pat};
use crate::settings_store;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitLabConnection {
    Idle,
    NoPat,
    Checking,
    Connected { username: String },
    Failed(String),
}

impl Default for GitLabConnection {
    fn default() -> Self {
        GitLabConnection::Idle
    }
}

impl GitLabConnection {
    pub fn chrome_label(&self) -> &'static str {
        match self {
            GitLabConnection::Idle | GitLabConnection::Checking => "GitLab…",
            GitLabConnection::NoPat => "GitLab: add token",
            GitLabConnection::Connected { .. } => "GitLab",
            GitLabConnection::Failed(_) => "GitLab: error",
        }
    }

}

pub fn apply_verify_result(connection: &Rc<RefCell<GitLabConnection>>, result: &VerifyResult) {
    let mut slot = connection.borrow_mut();
    *slot = match result {
        VerifyResult::Ok { username } => GitLabConnection::Connected {
            username: username.clone(),
        },
        VerifyResult::Err(VerifyError::Network(msg)) => {
            GitLabConnection::Failed(format!("Network: {msg}"))
        }
        VerifyResult::Err(VerifyError::Unauthorized) => {
            GitLabConnection::Failed("Unauthorized (401)".into())
        }
        VerifyResult::Err(VerifyError::Other { status, detail }) => {
            GitLabConnection::Failed(format!("HTTP {status}: {detail}"))
        }
    };
}

pub fn spawn_refresh_connection<E: 'static>(
    connection: Rc<RefCell<GitLabConnection>>,
    notify: WeakEntity<E>,
    cx: &mut Context<E>,
) {
    {
        let pat = settings_store::load_pat().unwrap_or_default();
        if pat.trim().is_empty() {
            *connection.borrow_mut() = GitLabConnection::NoPat;
            cx.notify();
            return;
        }
        *connection.borrow_mut() = GitLabConnection::Checking;
    }
    cx.notify();

    cx.spawn(async move |this, cx| {
        let http: Arc<dyn gpui_http_client::HttpClient> = match cx.update(|app| app.http_client()) {
            Ok(client) => client,
            Err(_) => return,
        };
        let base = settings_store::effective_base_url(&settings_store::load_file());
        let pat = settings_store::load_pat().unwrap_or_default();
        if pat.trim().is_empty() {
            *connection.borrow_mut() = GitLabConnection::NoPat;
        } else {
            let result = cx
                .background_executor()
                .spawn(async move { verify_pat(http, &base, &pat).await })
                .await;
            apply_verify_result(&connection, &result);
        }

        let _ = this.update(cx, |_, cx| cx.notify());
        let _ = notify.update(cx, |_, cx| cx.notify());
    })
    .detach();
}

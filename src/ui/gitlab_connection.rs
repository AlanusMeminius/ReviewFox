//! Shared GitLab auth display state (verify via `/user`), shared by the main
//! window and Settings.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, WeakEntity};

use crate::gitlab::{VerifyError, VerifyResult, verify_pat};
use crate::settings_store;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GitLabConnection {
    #[default]
    Idle,
    NoPat,
    Checking,
    Connected {
        username: String,
    },
    Failed(VerifyError),
}

thread_local! {
    /// Bumped by every refresh / reset; a verify result from an older
    /// generation is dropped so a slow request cannot overwrite a newer state.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn next_generation() -> u64 {
    GENERATION.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    })
}

fn is_current(generation: u64) -> bool {
    GENERATION.with(|g| g.get() == generation)
}

fn apply_verify_result(connection: &Rc<RefCell<GitLabConnection>>, result: &VerifyResult) {
    *connection.borrow_mut() = match result {
        VerifyResult::Ok { username } => GitLabConnection::Connected {
            username: username.clone(),
        },
        VerifyResult::Err(e) => GitLabConnection::Failed(e.clone()),
    };
}

/// The token was removed: NoPat now, and any in-flight verify is discarded.
pub fn set_no_pat(connection: &Rc<RefCell<GitLabConnection>>) {
    next_generation();
    *connection.borrow_mut() = GitLabConnection::NoPat;
}

/// Re-verify the saved base URL + keychain token off the UI thread.
pub fn spawn_refresh_connection<E: 'static>(
    connection: Rc<RefCell<GitLabConnection>>,
    notify: WeakEntity<E>,
    cx: &mut Context<E>,
) {
    let generation = next_generation();
    let pat = settings_store::load_pat().unwrap_or_default();
    if pat.trim().is_empty() {
        *connection.borrow_mut() = GitLabConnection::NoPat;
        cx.notify();
        return;
    }
    *connection.borrow_mut() = GitLabConnection::Checking;
    cx.notify();

    cx.spawn(async move |this, cx| {
        let http: Arc<dyn gpui_http_client::HttpClient> = match cx.update(|app| app.http_client()) {
            Ok(client) => client,
            Err(_) => return,
        };
        let base = settings_store::effective_base_url(&settings_store::load_file());
        let result = cx
            .background_executor()
            .spawn(async move { verify_pat(http, &base, &pat).await })
            .await;
        if !is_current(generation) {
            return;
        }
        apply_verify_result(&connection, &result);

        let _ = this.update(cx, |_, cx| cx.notify());
        let _ = notify.update(cx, |_, cx| cx.notify());
    })
    .detach();
}

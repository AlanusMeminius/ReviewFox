//! Personal Access Token row: state and copy as a pure function, so every
//! state and error string is unit-tested without a window.

use crate::gitlab::VerifyError;
use crate::ui::gitlab_connection::GitLabConnection;

pub const TITLE: &str = "Personal Access Token";
pub const PLACEHOLDER: &str = "glpat-xxxxxxxxxxxxxxxxxxxx";
/// Description, split around the "GitLab access tokens" link.
pub const DESCRIPTION_BEFORE_LINK: &str =
    "Create one with the api scope to publish, or read_api to browse, in ";
pub const DESCRIPTION_LINK: &str = "GitLab access tokens";
pub const DESCRIPTION_AFTER_LINK: &str = ".";
pub const RETRY: &str = "Retry";
pub const RESET_TOKEN: &str = "Reset Token";

/// Where the description link points, for the effective base URL.
pub fn access_tokens_url(base_url: &str) -> String {
    format!(
        "{}/-/user_settings/personal_access_tokens",
        base_url.trim_end_matches('/')
    )
}

/// A keychain write (Enter) or delete (Reset Token) that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeychainError {
    Save(String),
    Clear(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardStatus {
    Verifying,
    Connected,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenRow {
    /// Masked input; Enter saves.
    Input,
    /// `ConfiguredCard` with `label`; `Retry` shows only when `can_retry`,
    /// `Reset Token` always.
    Card {
        status: CardStatus,
        label: String,
        can_retry: bool,
    },
}

pub fn verify_error_message(error: &VerifyError) -> String {
    match error {
        VerifyError::Unauthorized => "Unauthorized (401). Check token and scopes.".into(),
        VerifyError::Network(msg) => format!("Network error: {msg}"),
        VerifyError::Other { status, detail } => format!("HTTP {status}: {detail}"),
    }
}

pub fn keychain_error_message(error: &KeychainError) -> String {
    match error {
        KeychainError::Save(msg) => format!("Could not save token to the keychain: {msg}"),
        KeychainError::Clear(msg) => format!("Could not clear token from the keychain: {msg}"),
    }
}

pub fn token_row(
    has_saved_token: bool,
    connection: &GitLabConnection,
    keychain_error: Option<&KeychainError>,
) -> TokenRow {
    if let Some(error) = keychain_error {
        return failed(keychain_error_message(error), false);
    }
    if !has_saved_token {
        return TokenRow::Input;
    }
    match connection {
        // The connection has not caught up with the saved token yet.
        GitLabConnection::Idle | GitLabConnection::Checking => TokenRow::Card {
            status: CardStatus::Verifying,
            label: "Verifying…".into(),
            can_retry: false,
        },
        GitLabConnection::NoPat => TokenRow::Input,
        GitLabConnection::Connected { username } => TokenRow::Card {
            status: CardStatus::Connected,
            label: format!("Signed in as {username}"),
            can_retry: false,
        },
        GitLabConnection::Failed(error) => failed(verify_error_message(error), true),
    }
}

fn failed(label: String, can_retry: bool) -> TokenRow {
    TokenRow::Card {
        status: CardStatus::Failed,
        label,
        can_retry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(status: CardStatus, label: &str, can_retry: bool) -> TokenRow {
        TokenRow::Card {
            status,
            label: label.into(),
            can_retry,
        }
    }

    #[test]
    fn no_saved_token_is_input_whatever_the_connection() {
        for connection in [
            GitLabConnection::Idle,
            GitLabConnection::NoPat,
            GitLabConnection::Checking,
            GitLabConnection::Connected {
                username: "alice".into(),
            },
            GitLabConnection::Failed(VerifyError::Unauthorized),
        ] {
            assert_eq!(token_row(false, &connection, None), TokenRow::Input);
        }
    }

    #[test]
    fn saved_token_but_no_pat_connection_is_input() {
        assert_eq!(
            token_row(true, &GitLabConnection::NoPat, None),
            TokenRow::Input
        );
    }

    #[test]
    fn checking_and_idle_are_verifying() {
        let verifying = card(CardStatus::Verifying, "Verifying…", false);
        assert_eq!(
            token_row(true, &GitLabConnection::Checking, None),
            verifying
        );
        assert_eq!(token_row(true, &GitLabConnection::Idle, None), verifying);
    }

    #[test]
    fn connected_names_the_user() {
        let connection = GitLabConnection::Connected {
            username: "alice".into(),
        };
        assert_eq!(
            token_row(true, &connection, None),
            card(CardStatus::Connected, "Signed in as alice", false)
        );
    }

    #[test]
    fn unauthorized_copy() {
        let connection = GitLabConnection::Failed(VerifyError::Unauthorized);
        assert_eq!(
            token_row(true, &connection, None),
            card(
                CardStatus::Failed,
                "Unauthorized (401). Check token and scopes.",
                true
            )
        );
    }

    #[test]
    fn network_error_copy() {
        let connection = GitLabConnection::Failed(VerifyError::Network("dns failure".into()));
        assert_eq!(
            token_row(true, &connection, None),
            card(CardStatus::Failed, "Network error: dns failure", true)
        );
    }

    #[test]
    fn http_error_copy() {
        let connection = GitLabConnection::Failed(VerifyError::Other {
            status: 503,
            detail: "Service Unavailable".into(),
        });
        assert_eq!(
            token_row(true, &connection, None),
            card(CardStatus::Failed, "HTTP 503: Service Unavailable", true)
        );
    }

    #[test]
    fn keychain_save_error_wins_and_offers_no_retry() {
        let error = KeychainError::Save("access denied".into());
        assert_eq!(
            token_row(false, &GitLabConnection::NoPat, Some(&error)),
            card(
                CardStatus::Failed,
                "Could not save token to the keychain: access denied",
                false
            )
        );
    }

    #[test]
    fn keychain_clear_error_wins_over_connection() {
        let error = KeychainError::Clear("locked".into());
        let connection = GitLabConnection::Connected {
            username: "alice".into(),
        };
        assert_eq!(
            token_row(true, &connection, Some(&error)),
            card(
                CardStatus::Failed,
                "Could not clear token from the keychain: locked",
                false
            )
        );
    }

    #[test]
    fn access_tokens_url_uses_base() {
        assert_eq!(
            access_tokens_url("https://gitlab.example.com"),
            "https://gitlab.example.com/-/user_settings/personal_access_tokens"
        );
        assert_eq!(
            access_tokens_url("https://gitlab.com/"),
            "https://gitlab.com/-/user_settings/personal_access_tokens"
        );
    }
}

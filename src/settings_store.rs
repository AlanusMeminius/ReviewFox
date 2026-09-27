//! Settings: GitLab base URL and Appearance fonts in `settings.json`, PAT in
//! OS keychain only.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_BASE_URL: &str = "https://gitlab.com";
const FILENAME: &str = "settings.json";
const KEYRING_SERVICE: &str = "ReviewFox";
const KEYRING_ACCOUNT: &str = "gitlab_pat";

/// Every field is optional and omitted when `None`: missing means Default, so a
/// changed default reaches users who never touched the setting.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitlab_base_url: Option<String>,
    /// Stored as written; resolution (installed check, clamping) is
    /// `ui::appearance`'s job, so an uninstalled family survives a save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_font_size: Option<f32>,
}

pub fn normalize_base_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_BASE_URL.to_string();
    }

    let mut s = trimmed.trim_end_matches('/').to_string();
    if !s.contains("://") {
        s = format!("https://{s}");
    }

    if let Ok(mut url) = url::Url::parse(&s) {
        let path = url.path().trim_end_matches('/');
        if path == "/api/v4" {
            url.set_path("");
        }
        let mut out = url.to_string();
        while out.ends_with('/') {
            out.pop();
        }
        return out;
    }

    if s.ends_with("/api/v4") {
        s.truncate(s.len() - "/api/v4".len());
        while s.ends_with('/') {
            s.pop();
        }
    }
    s
}

pub fn effective_base_url(file: &SettingsFile) -> String {
    file.gitlab_base_url
        .as_deref()
        .map(normalize_base_url)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

pub fn load_file() -> SettingsFile {
    let Some(path) = store_path() else {
        return SettingsFile::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return SettingsFile::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_file(file: &SettingsFile) {
    let Some(path) = store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_vec_pretty(file) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn load_pat() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .ok()?
        .get_password()
        .ok()
}

pub fn save_pat(pat: &str) -> Result<(), keyring::Error> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?.set_password(pat)
}

pub fn clear_pat() -> Result<(), keyring::Error> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?.delete_credential()
}

pub fn store_path_for_tests(root: &std::path::Path) -> PathBuf {
    root.join("ReviewFox").join(FILENAME)
}

pub fn save_file_at(path: &std::path::Path, file: &SettingsFile) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_vec_pretty(file) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn load_file_at(path: &std::path::Path) -> SettingsFile {
    let Ok(bytes) = std::fs::read(path) else {
        return SettingsFile::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("ReviewFox").join(FILENAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn normalize_trims_and_https_and_strips_api_v4() {
        assert_eq!(
            normalize_base_url("  gitlab.com/api/v4/  "),
            "https://gitlab.com"
        );
        assert_eq!(
            normalize_base_url("https://gitlab.example.com/"),
            "https://gitlab.example.com"
        );
        assert_eq!(
            normalize_base_url("https://gitlab.com/api/v4"),
            "https://gitlab.com"
        );
    }

    #[test]
    fn normalize_empty_defaults() {
        assert_eq!(normalize_base_url(""), DEFAULT_BASE_URL);
        assert_eq!(normalize_base_url("   "), DEFAULT_BASE_URL);
    }

    #[test]
    fn settings_roundtrip_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        let file = SettingsFile {
            gitlab_base_url: Some("https://gitlab.example.com".into()),
            ..Default::default()
        };
        save_file_at(&path, &file);
        let loaded = load_file_at(&path);
        assert_eq!(loaded, file);
        fs::remove_dir_all(dir.path()).ok();
    }

    #[test]
    fn font_fields_roundtrip_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        let file = SettingsFile {
            ui_font_family: Some("Inter".into()),
            ui_font_size: Some(14.),
            code_font_family: Some("JetBrains Mono".into()),
            code_font_size: Some(16.),
            ..Default::default()
        };
        save_file_at(&path, &file);
        assert_eq!(load_file_at(&path), file);
        fs::remove_dir_all(dir.path()).ok();
    }

    #[test]
    fn missing_font_fields_load_as_default_and_are_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{ "gitlab_base_url": "https://gitlab.example.com" }"#).unwrap();
        let loaded = load_file_at(&path);
        assert_eq!(loaded.ui_font_family, None);
        assert_eq!(loaded.code_font_size, None);

        save_file_at(&path, &loaded);
        let json = fs::read_to_string(&path).unwrap();
        assert!(!json.contains("font"), "unset font fields must not be written: {json}");
        fs::remove_dir_all(dir.path()).ok();
    }
}

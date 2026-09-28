//! Settings: GitLab base URL and Appearance fonts in `settings.json`, PAT in
//! OS keychain only (read once per process; see [`load_pat`]).

use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

pub const DEFAULT_BASE_URL: &str = "https://gitlab.com";
const FILENAME: &str = "settings.json";
const KEYRING_SERVICE: &str = "ReviewFox";
const KEYRING_ACCOUNT: &str = "gitlab_pat";

/// Every field is optional and omitted when `None`: missing means Default, so a
/// changed default reaches users who never touched the setting. A field of the
/// wrong type loads as `None` rather than failing the whole file, so one bad
/// hand edit cannot wipe the others on the next save.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub gitlab_base_url: Option<String>,
    /// Stored as written; resolution (installed check, clamping) is
    /// `ui::appearance`'s job, so an uninstalled family survives a save.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub ui_font_family: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub ui_font_size: Option<f32>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub code_font_family: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub code_font_size: Option<f32>,
}

/// `Some` when the value parses as `T`, `None` otherwise (never an error).
fn lenient<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// Why [`save_file`] wrote nothing.
#[derive(Debug)]
pub enum SaveError {
    /// The file on disk is not a JSON object; saving over it would lose
    /// whatever the user was editing.
    Unreadable,
    #[allow(dead_code)] // For `Debug`; no caller reports the cause yet.
    Io(std::io::Error),
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
    store_path().map_or_else(SettingsFile::default, |path| load_file_at(&path))
}

pub fn save_file(file: &SettingsFile) -> Result<(), SaveError> {
    let path = store_path().ok_or(SaveError::Io(std::io::ErrorKind::NotFound.into()))?;
    save_file_at(&path, file)
}

// ponytail: one keychain read per process — unsigned Mac builds still ACL-prompt on that first get.
/// `None` = not fetched yet; `Some(None)` = known empty; `Some(Some(_))` = value.
static PAT_CACHE: Mutex<Option<Option<String>>> = Mutex::new(None);

fn pat_cache() -> std::sync::MutexGuard<'static, Option<Option<String>>> {
    PAT_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn load_pat() -> Option<String> {
    if let Some(cached) = pat_cache().as_ref() {
        return cached.clone();
    }
    let pat = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .ok()
        .and_then(|e| e.get_password().ok());
    *pat_cache() = Some(pat.clone());
    pat
}

pub fn save_pat(pat: &str) -> Result<(), keyring::Error> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?.set_password(pat)?;
    *pat_cache() = Some(Some(pat.to_string()));
    Ok(())
}

pub fn clear_pat() -> Result<(), keyring::Error> {
    let result = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?.delete_credential();
    if result.is_ok() || matches!(&result, Err(keyring::Error::NoEntry)) {
        *pat_cache() = Some(None);
    }
    result
}

pub fn store_path_for_tests(root: &std::path::Path) -> PathBuf {
    root.join("ReviewFox").join(FILENAME)
}

/// Refuses to replace an existing file that is not a JSON object.
pub fn save_file_at(path: &std::path::Path, file: &SettingsFile) -> Result<(), SaveError> {
    if let Ok(bytes) = std::fs::read(path)
        && !matches!(
            serde_json::from_slice::<serde_json::Value>(&bytes),
            Ok(serde_json::Value::Object(_))
        )
    {
        return Err(SaveError::Unreadable);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(SaveError::Io)?;
    }
    let json = serde_json::to_vec_pretty(file).map_err(|e| SaveError::Io(e.into()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json).map_err(SaveError::Io)?;
    std::fs::rename(&tmp, path).map_err(SaveError::Io)
}

/// Missing or unreadable is Default; see [`SettingsFile`] for bad fields.
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
        save_file_at(&path, &file).unwrap();
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
        save_file_at(&path, &file).unwrap();
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

        save_file_at(&path, &loaded).unwrap();
        let json = fs::read_to_string(&path).unwrap();
        assert!(!json.contains("font"), "unset font fields must not be written: {json}");
        fs::remove_dir_all(dir.path()).ok();
    }

    #[test]
    fn a_malformed_field_is_dropped_and_the_rest_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{ "gitlab_base_url": "https://gitlab.example.com", "ui_font_size": "14", "code_font_family": 7, "code_font_size": 16 }"#,
        )
        .unwrap();
        let loaded = load_file_at(&path);
        assert_eq!(
            loaded,
            SettingsFile {
                gitlab_base_url: Some("https://gitlab.example.com".into()),
                code_font_size: Some(16.),
                ..Default::default()
            }
        );
        fs::remove_dir_all(dir.path()).ok();
    }

    #[test]
    fn save_never_overwrites_a_file_that_is_not_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let broken = r#"{ "gitlab_base_url": "https://gitlab.example.com", "#;
        fs::write(&path, broken).unwrap();

        let mut file = load_file_at(&path);
        file.ui_font_size = Some(14.);
        assert!(save_file_at(&path, &file).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
        fs::remove_dir_all(dir.path()).ok();
    }
}

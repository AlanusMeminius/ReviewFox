//! Global OS window geometry for main + Diff (ADR-0007). Not Workspace-scoped.
//! Splitters stay session-only (ADR-0004). Settings window is not persisted.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const FILENAME: &str = "window_geometry.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Diff auto-reopen binds Comparison identity + last selected path (ADR-0005 / ADR-0007).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReopen {
    pub repository: PathBuf,
    /// Base commit OID; `None` (JSON `null`) = empty tree (root-commit Comparison).
    /// Older files always carry a string, which loads as `Some`.
    #[serde(default)]
    pub base_oid: Option<String>,
    pub head_oid: String,
    pub selected_path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometryFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<StoredBounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<StoredBounds>,
    /// True when Diff was open at last persistence while the app was still using it.
    #[serde(default)]
    pub diff_open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_reopen: Option<DiffReopen>,
}

static STATE: Mutex<Option<WindowGeometryFile>> = Mutex::new(None);
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);
static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn load() -> WindowGeometryFile {
    let file = load_from_disk();
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file.clone());
    file
}

pub fn snapshot() -> WindowGeometryFile {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(load_from_disk());
    }
    guard.clone().unwrap_or_default()
}

pub fn set_main_bounds(bounds: StoredBounds) {
    with_state(|s| s.main = Some(bounds));
}

pub fn set_diff_bounds(bounds: StoredBounds) {
    with_state(|s| s.diff = Some(bounds));
}

pub fn note_diff_opened(reopen: DiffReopen) {
    with_state(|s| {
        s.diff_open = true;
        s.diff_reopen = Some(reopen);
    });
    flush();
}

pub fn note_diff_selected_path(path: String) {
    with_state(|s| {
        if let Some(reopen) = s.diff_reopen.as_mut() {
            reopen.selected_path = path;
        }
    });
}

/// Diff closed while another window is still alive — clear auto-reopen flag.
pub fn note_diff_closed_while_app_alive() {
    if QUITTING.load(Ordering::SeqCst) {
        return;
    }
    with_state(|s| s.diff_open = false);
    flush();
}

pub fn begin_quit() {
    QUITTING.store(true, Ordering::SeqCst);
}

pub fn next_save_generation() -> u64 {
    SAVE_GEN.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn current_save_generation() -> u64 {
    SAVE_GEN.load(Ordering::SeqCst)
}

pub fn flush() {
    let file = snapshot();
    save_to_disk(&file);
}

pub fn flush_if_generation(generation: u64) {
    if current_save_generation() == generation {
        flush();
    }
}

fn with_state(f: impl FnOnce(&mut WindowGeometryFile)) {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(load_from_disk());
    }
    if let Some(state) = guard.as_mut() {
        f(state);
    }
}

fn load_from_disk() -> WindowGeometryFile {
    let Some(path) = store_path() else {
        return WindowGeometryFile::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return WindowGeometryFile::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn save_to_disk(file: &WindowGeometryFile) {
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

fn store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("ReviewFox").join(FILENAME))
}

#[cfg(test)]
pub fn store_path_for_tests(root: &std::path::Path) -> PathBuf {
    root.join("ReviewFox").join(FILENAME)
}

#[cfg(test)]
pub fn save_at(path: &std::path::Path, file: &WindowGeometryFile) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_vec_pretty(file).unwrap();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

#[cfg(test)]
pub fn load_at(path: &std::path::Path) -> WindowGeometryFile {
    let Ok(bytes) = std::fs::read(path) else {
        return WindowGeometryFile::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Pure intersection check used by UI restore (no GPUI in unit tests).
pub fn bounds_intersects_any(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    displays: &[(f32, f32, f32, f32)],
) -> bool {
    if width <= 0. || height <= 0. {
        return false;
    }
    let right = x + width;
    let bottom = y + height;
    displays.iter().any(|&(dx, dy, dw, dh)| {
        x < dx + dw && right > dx && y < dy + dh && bottom > dy
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_json() {
        let file = WindowGeometryFile {
            main: Some(StoredBounds {
                x: 10.,
                y: 20.,
                width: 1280.,
                height: 820.,
            }),
            diff: Some(StoredBounds {
                x: 100.,
                y: 80.,
                width: 1100.,
                height: 720.,
            }),
            diff_open: true,
            diff_reopen: Some(DiffReopen {
                repository: PathBuf::from("/repo"),
                base_oid: Some("a".repeat(40)),
                head_oid: "b".repeat(40),
                selected_path: "src/a.rs".into(),
            }),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        save_at(&path, &file);
        let loaded = load_at(&path);
        assert_eq!(loaded, file);
    }

    #[test]
    fn empty_tree_base_round_trips() {
        let file = WindowGeometryFile {
            diff_open: true,
            diff_reopen: Some(DiffReopen {
                repository: PathBuf::from("/repo"),
                base_oid: None,
                head_oid: "b".repeat(40),
                selected_path: "README.md".into(),
            }),
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        save_at(&path, &file);
        assert_eq!(load_at(&path), file);
    }

    #[test]
    fn old_format_string_base_oid_loads() {
        let old = format!(
            r#"{{"diff_open":true,"diff_reopen":{{"repository":"/repo","base_oid":"{}","head_oid":"{}","selected_path":"a.rs"}}}}"#,
            "a".repeat(40),
            "b".repeat(40)
        );
        let loaded: WindowGeometryFile = serde_json::from_str(&old).unwrap();
        let reopen = loaded.diff_reopen.expect("diff_reopen");
        assert_eq!(reopen.base_oid, Some("a".repeat(40)));
        assert_eq!(reopen.head_oid, "b".repeat(40));
        assert!(loaded.diff_open);
    }

    #[test]
    fn missing_file_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_for_tests(dir.path());
        assert_eq!(load_at(&path), WindowGeometryFile::default());
    }

    #[test]
    fn intersects_visible_display() {
        let displays = [(0., 0., 1440., 900.)];
        assert!(bounds_intersects_any(100., 100., 800., 600., &displays));
        assert!(!bounds_intersects_any(2000., 100., 800., 600., &displays));
        assert!(!bounds_intersects_any(0., 0., 0., 600., &displays));
    }
}

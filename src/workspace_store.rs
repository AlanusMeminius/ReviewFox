//! Workspace set + last + pinned: (repo path, branch label) in app data dir. No Review/Comparison persistence.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FILENAME: &str = "workspaces.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStore {
    pub last: Option<PathBuf>,
    /// Newest pin first (UI order matches this vector).
    #[serde(default)]
    pub pinned: Vec<PathBuf>,
    pub workspaces: Vec<WorkspaceEntry>,
}

pub fn load() -> WorkspaceStore {
    let Some(path) = store_path() else {
        return WorkspaceStore::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return WorkspaceStore::default();
    };
    if let Ok(store) = serde_json::from_slice::<WorkspaceStore>(&bytes) {
        return store;
    }
    // Old format: bare JSON array of {path, branch}
    if let Ok(entries) = serde_json::from_slice::<Vec<WorkspaceEntry>>(&bytes) {
        return migrate_array(entries);
    }
    WorkspaceStore::default()
}

pub fn save(store: &WorkspaceStore) {
    let Some(path) = store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_vec_pretty(store) else {
        return;
    };
    // ponytail: rename-over-write is enough atomicity for this JSON
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Upsert by path (no display reorder), set `last`, write immediately.
pub fn remember(path: &Path, branch: &str) {
    let mut store = load();
    remember_in(&mut store, path.to_path_buf(), branch.to_string());
    save(&store);
}

/// Entry matching `last`, if any.
pub fn last_entry(store: &WorkspaceStore) -> Option<&WorkspaceEntry> {
    let last = store.last.as_ref()?;
    store.workspaces.iter().find(|e| &e.path == last)
}

/// Pin path (newest first). Path should already be in `workspaces`.
pub fn pin(path: &Path) {
    let mut store = load();
    pin_in(&mut store, path);
    save(&store);
}

/// Remove from `pinned` only; workspace stays.
pub fn unpin(path: &Path) {
    let mut store = load();
    unpin_in(&mut store, path);
    save(&store);
}

/// Hard-delete workspace by path; clear from `pinned` and `last` if present.
pub fn drop_path(path: &Path) {
    let mut store = load();
    drop_path_in(&mut store, path);
    save(&store);
}

/// Pinned entries in pin order (newest first). Orphan pin paths are skipped.
pub fn pinned_entries(store: &WorkspaceStore) -> Vec<WorkspaceEntry> {
    store
        .pinned
        .iter()
        .filter_map(|p| store.workspaces.iter().find(|e| &e.path == p).cloned())
        .collect()
}

/// Non-pinned workspaces, alpha by display name (case-insensitive), path tiebreak.
pub fn repository_entries(store: &WorkspaceStore) -> Vec<WorkspaceEntry> {
    let non_pinned: Vec<_> = store
        .workspaces
        .iter()
        .filter(|e| !store.pinned.iter().any(|p| p == &e.path))
        .cloned()
        .collect();
    sorted_workspaces(&non_pinned)
}

/// Alphabetical by display name (case-insensitive), tiebreak canonical path.
pub fn sorted_workspaces(workspaces: &[WorkspaceEntry]) -> Vec<WorkspaceEntry> {
    let mut out = workspaces.to_vec();
    out.sort_by(|a, b| {
        display_name(&a.path)
            .to_lowercase()
            .cmp(&display_name(&b.path).to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

fn remember_in(store: &mut WorkspaceStore, path: PathBuf, branch: String) {
    if let Some(entry) = store.workspaces.iter_mut().find(|e| e.path == path) {
        entry.branch = branch;
    } else {
        store.workspaces.push(WorkspaceEntry {
            path: path.clone(),
            branch,
        });
    }
    store.last = Some(path);
}

fn pin_in(store: &mut WorkspaceStore, path: &Path) {
    store.pinned.retain(|p| p != path);
    store.pinned.insert(0, path.to_path_buf());
}

fn unpin_in(store: &mut WorkspaceStore, path: &Path) {
    store.pinned.retain(|p| p != path);
}

fn drop_path_in(store: &mut WorkspaceStore, path: &Path) {
    store.workspaces.retain(|e| e.path != path);
    store.pinned.retain(|p| p != path);
    if store.last.as_ref().is_some_and(|p| p == path) {
        store.last = None;
    }
}

fn migrate_array(entries: Vec<WorkspaceEntry>) -> WorkspaceStore {
    let last = entries.first().map(|e| e.path.clone());
    WorkspaceStore {
        last,
        pinned: Vec::new(),
        workspaces: entries,
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("ReviewFox").join(FILENAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, branch: &str) -> WorkspaceEntry {
        WorkspaceEntry {
            path: PathBuf::from(path),
            branch: branch.into(),
        }
    }

    #[test]
    fn remember_sets_last_without_reordering_workspaces() {
        let mut store = WorkspaceStore {
            last: Some(PathBuf::from("/a")),
            pinned: Vec::new(),
            workspaces: vec![entry("/a", "main"), entry("/b", "dev")],
        };
        remember_in(&mut store, PathBuf::from("/b"), "feature".into());
        assert_eq!(store.last, Some(PathBuf::from("/b")));
        assert_eq!(store.workspaces[0].path, PathBuf::from("/a"));
        assert_eq!(store.workspaces[1].path, PathBuf::from("/b"));
        assert_eq!(store.workspaces[1].branch, "feature");
    }

    #[test]
    fn remember_inserts_new_path() {
        let mut store = WorkspaceStore::default();
        remember_in(&mut store, PathBuf::from("/z"), "main".into());
        assert_eq!(store.last, Some(PathBuf::from("/z")));
        assert_eq!(store.workspaces.len(), 1);
    }

    #[test]
    fn migrate_bare_array_sets_last_to_first() {
        let store = migrate_array(vec![entry("/first", "main"), entry("/second", "dev")]);
        assert_eq!(store.last, Some(PathBuf::from("/first")));
        assert!(store.pinned.is_empty());
        assert_eq!(store.workspaces.len(), 2);
    }

    #[test]
    fn migrate_empty_array_last_is_none() {
        let store = migrate_array(Vec::new());
        assert_eq!(store.last, None);
        assert!(store.pinned.is_empty());
        assert!(store.workspaces.is_empty());
    }

    #[test]
    fn sorted_by_display_name_case_insensitive_path_tiebreak() {
        let entries = vec![
            entry("/z/Banana", "a"),
            entry("/a/apple", "b"),
            entry("/b/Apple", "c"),
        ];
        let sorted = sorted_workspaces(&entries);
        assert_eq!(sorted[0].path, PathBuf::from("/a/apple"));
        assert_eq!(sorted[1].path, PathBuf::from("/b/Apple"));
        assert_eq!(sorted[2].path, PathBuf::from("/z/Banana"));
    }

    #[test]
    fn last_entry_finds_matching_workspace() {
        let store = WorkspaceStore {
            last: Some(PathBuf::from("/b")),
            pinned: Vec::new(),
            workspaces: vec![entry("/a", "main"), entry("/b", "dev")],
        };
        let e = last_entry(&store).unwrap();
        assert_eq!(e.path, PathBuf::from("/b"));
        assert_eq!(e.branch, "dev");
    }

    #[test]
    fn pin_newest_first_and_excludes_from_repositories() {
        let mut store = WorkspaceStore {
            last: None,
            pinned: Vec::new(),
            workspaces: vec![entry("/a", "main"), entry("/b", "dev"), entry("/c", "x")],
        };
        pin_in(&mut store, Path::new("/a"));
        pin_in(&mut store, Path::new("/c"));
        assert_eq!(
            store.pinned,
            vec![PathBuf::from("/c"), PathBuf::from("/a")]
        );
        let pins = pinned_entries(&store);
        assert_eq!(pins[0].path, PathBuf::from("/c"));
        assert_eq!(pins[1].path, PathBuf::from("/a"));
        let repos = repository_entries(&store);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].path, PathBuf::from("/b"));
    }

    #[test]
    fn pin_again_moves_to_front() {
        let mut store = WorkspaceStore {
            last: None,
            pinned: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            workspaces: vec![entry("/a", "main"), entry("/b", "dev")],
        };
        pin_in(&mut store, Path::new("/b"));
        assert_eq!(
            store.pinned,
            vec![PathBuf::from("/b"), PathBuf::from("/a")]
        );
    }

    #[test]
    fn unpin_keeps_workspace_returns_to_repositories() {
        let mut store = WorkspaceStore {
            last: Some(PathBuf::from("/a")),
            pinned: vec![PathBuf::from("/a")],
            workspaces: vec![entry("/a", "main"), entry("/b", "dev")],
        };
        unpin_in(&mut store, Path::new("/a"));
        assert!(store.pinned.is_empty());
        assert_eq!(store.workspaces.len(), 2);
        assert_eq!(store.last, Some(PathBuf::from("/a")));
        let repos = repository_entries(&store);
        assert_eq!(repos.len(), 2);
    }

    #[test]
    fn drop_path_clears_workspace_pinned_and_last() {
        let mut store = WorkspaceStore {
            last: Some(PathBuf::from("/a")),
            pinned: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            workspaces: vec![entry("/a", "main"), entry("/b", "dev")],
        };
        drop_path_in(&mut store, Path::new("/a"));
        assert_eq!(store.last, None);
        assert_eq!(store.pinned, vec![PathBuf::from("/b")]);
        assert_eq!(store.workspaces.len(), 1);
        assert_eq!(store.workspaces[0].path, PathBuf::from("/b"));
    }

    #[test]
    fn missing_pinned_deserializes_as_empty() {
        let json = r#"{"last":"/a","workspaces":[{"path":"/a","branch":"main"}]}"#;
        let store: WorkspaceStore = serde_json::from_str(json).unwrap();
        assert!(store.pinned.is_empty());
        assert_eq!(store.last, Some(PathBuf::from("/a")));
    }
}

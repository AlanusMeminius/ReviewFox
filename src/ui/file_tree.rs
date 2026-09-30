//! Nested ChangedPath tree projection for main + Diff panes.

use std::collections::HashSet;

use crate::domain::ChangedPath;

#[derive(Clone, Debug)]
pub enum TreeRow {
    Dir {
        depth: u32,
        name: String,
        /// Full dir path (e.g. `src/ui`) used as collapse key.
        path: String,
    },
    File {
        depth: u32,
        path: ChangedPath,
    },
}

#[derive(Default)]
struct Node {
    dirs: std::collections::BTreeMap<String, Node>,
    files: std::collections::BTreeMap<String, ChangedPath>,
}

fn insert(node: &mut Node, parts: &[&str], file: ChangedPath) {
    if parts.len() == 1 {
        node.files.insert(parts[0].to_string(), file);
        return;
    }
    insert(
        node.dirs.entry(parts[0].to_string()).or_default(),
        &parts[1..],
        file,
    );
}

fn emit(
    node: &Node,
    depth: u32,
    prefix: &str,
    collapsed: &HashSet<String>,
    out: &mut Vec<TreeRow>,
) {
    for (name, kid) in &node.dirs {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let is_collapsed = collapsed.contains(&path);
        out.push(TreeRow::Dir {
            depth,
            name: name.clone(),
            path: path.clone(),
        });
        if !is_collapsed {
            emit(kid, depth + 1, &path, collapsed, out);
        }
    }
    for file in node.files.values() {
        out.push(TreeRow::File {
            depth,
            path: file.clone(),
        });
    }
}

/// Nested rows; dirs before files at each level, sorted by name.
/// Paths in `collapsed` skip their descendants (default empty = all expanded).
pub fn flatten(paths: &[ChangedPath], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::default();
    for p in paths {
        let parts: Vec<&str> = p.path.split('/').filter(|s| !s.is_empty()).collect();
        if !parts.is_empty() {
            insert(&mut root, &parts, p.clone());
        }
    }
    let mut out = Vec::new();
    emit(&root, 0, "", collapsed, &mut out);
    out
}

/// [`flatten`], after dropping files whose path does not contain `query`.
/// Empty `query` is the full tree. Matching is Unicode case-insensitive.
/// Directories remain only when a kept file sits under them. Collapse still applies.
pub fn flatten_query(
    paths: &[ChangedPath],
    collapsed: &HashSet<String>,
    query: &str,
) -> Vec<TreeRow> {
    let Some(q) = normalized_query(query) else {
        return flatten(paths, collapsed);
    };
    let matched: Vec<ChangedPath> = paths
        .iter()
        .filter(|p| path_matches(&p.path, &q))
        .cloned()
        .collect();
    flatten(&matched, collapsed)
}

/// File paths in the order the fully expanded tree lists them.
pub fn file_order(paths: &[ChangedPath]) -> Vec<String> {
    flatten(paths, &HashSet::new())
        .into_iter()
        .filter_map(|row| match row {
            TreeRow::File { path, .. } => Some(path.path),
            TreeRow::Dir { .. } => None,
        })
        .collect()
}

/// [`file_order`] limited to paths that contain `query`.
/// Empty `query` is the full order. Collapse is ignored: a folded directory
/// still leaves its files in the nav list.
pub fn file_order_query(paths: &[ChangedPath], query: &str) -> Vec<String> {
    let order = file_order(paths);
    let Some(q) = normalized_query(query) else {
        return order;
    };
    order.into_iter().filter(|p| path_matches(p, &q)).collect()
}

/// Neighbour of `selected` in `order`.
/// When `selected` is absent, forward lands on the first path and backward on the last.
/// Present `selected` stops at either end.
pub fn step_file(order: &[String], selected: &str, dir: i32) -> Option<String> {
    match order.iter().position(|p| p == selected) {
        Some(index) => {
            let target = if dir < 0 {
                index.checked_sub(1)
            } else {
                index.checked_add(1)
            };
            target.and_then(|i| order.get(i).cloned())
        }
        None if dir < 0 => order.last().cloned(),
        None => order.first().cloned(),
    }
}

/// Trimmed query, lowercased. `None` means "no filter".
fn normalized_query(query: &str) -> Option<String> {
    let q = query.trim();
    if q.is_empty() {
        None
    } else {
        Some(q.to_lowercase())
    }
}

/// Whether a path contains a non-empty query, case-insensitively.
pub fn path_matches_query(path: &str, query: &str) -> bool {
    normalized_query(query).is_some_and(|q| path_matches(path, &q))
}

fn path_matches(path: &str, q: &str) -> bool {
    path.to_lowercase().contains(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PathStatus;

    fn cp(path: &str) -> ChangedPath {
        ChangedPath {
            path: path.into(),
            status: PathStatus::Modify,
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn collapsed_skips_descendants() {
        let paths = vec![cp("a/b/c.rs"), cp("a/d.rs")];
        let mut collapsed = HashSet::new();
        collapsed.insert("a".into());
        let rows = flatten(&paths, &collapsed);
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            TreeRow::Dir { path, name, depth } => {
                assert_eq!(path, "a");
                assert_eq!(name, "a");
                assert_eq!(*depth, 0);
            }
            TreeRow::File { .. } => panic!("expected dir"),
        }
    }

    #[test]
    fn dir_rows_carry_full_path() {
        let paths = vec![cp("src/ui/mod.rs")];
        let rows = flatten(&paths, &HashSet::new());
        let dirs: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::Dir { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(dirs, ["src", "src/ui"]);
    }

    #[test]
    fn query_drops_non_matching_files_and_their_empty_dirs() {
        let paths = vec![cp("src/a.rs"), cp("docs/b.md")];
        let labels: Vec<_> = flatten_query(&paths, &HashSet::new(), "A.RS")
            .into_iter()
            .map(|row| match row {
                TreeRow::Dir { path, .. } => path,
                TreeRow::File { path, .. } => path.path,
            })
            .collect();
        assert_eq!(labels, ["src", "src/a.rs"]);
    }

    #[test]
    fn query_still_honors_collapsed_dirs() {
        let paths = vec![cp("src/a.rs")];
        let mut collapsed = HashSet::new();
        collapsed.insert("src".into());
        assert_eq!(flatten_query(&paths, &collapsed, "a.rs").len(), 1);
    }

    #[test]
    fn file_order_follows_tree_not_input() {
        let paths = vec![cp("z.rs"), cp("a/y.rs"), cp("a/b/x.rs")];
        assert_eq!(file_order(&paths), ["a/b/x.rs", "a/y.rs", "z.rs"]);
    }

    #[test]
    fn path_query_trims_and_matches_case_insensitively() {
        assert!(path_matches_query("src/Ä.rs", " ä.RS "));
        assert!(!path_matches_query("src/Ä.rs", "missing"));
        assert!(!path_matches_query("src/Ä.rs", "  "));
    }

    #[test]
    fn file_order_query_keeps_tree_order() {
        let paths = vec![cp("z.rs"), cp("a/y.rs"), cp("a/b/x.rs")];
        assert_eq!(file_order_query(&paths, "  "), file_order(&paths));
        assert_eq!(file_order_query(&paths, "Y"), ["a/y.rs"]);
        assert_eq!(
            file_order_query(&paths, ".rs"),
            ["a/b/x.rs", "a/y.rs", "z.rs"]
        );
    }

    #[test]
    fn step_file_stops_at_ends_and_enters_from_outside() {
        let order = vec!["a/y.rs".into(), "z.rs".into()];
        assert_eq!(step_file(&order, "a/y.rs", 1).as_deref(), Some("z.rs"));
        assert_eq!(step_file(&order, "z.rs", 1), None);
        assert_eq!(step_file(&order, "a/y.rs", -1), None);
        assert_eq!(
            step_file(&order, "missing.rs", 1).as_deref(),
            Some("a/y.rs")
        );
        assert_eq!(step_file(&order, "missing.rs", -1).as_deref(), Some("z.rs"));
        assert_eq!(step_file(&[], "missing.rs", 1), None);
    }
}

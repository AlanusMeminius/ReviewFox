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
}

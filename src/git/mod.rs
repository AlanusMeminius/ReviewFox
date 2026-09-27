//! git2 adapter: Repository / Comparison I/O → domain types. No UI chrome.

use crate::domain::{
    Alignment, AlignmentOp, ChangedPath, Comparison, DisplayRows, LineSpan, Oid, PathStatus,
    Repository, Side, ViewOptions, display_rows_folded, split_lines,
};
use crate::workspace_store::{self, WorkspaceEntry};
use similar::{DiffOp, TextDiff};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err(msg: impl Into<String>) -> Error {
    Error(msg.into())
}

fn map_git(e: git2::Error) -> Error {
    Error(e.to_string())
}

fn oid_from_git(id: git2::Oid) -> Oid {
    let mut bytes = [0u8; 20];
    bytes.copy_from_slice(id.as_bytes());
    Oid::from_bytes(bytes)
}

fn oid_to_git(oid: Oid) -> git2::Oid {
    git2::Oid::from_bytes(oid.as_bytes()).expect("20-byte oid")
}

const COMMIT_LIST_LIMIT: usize = 200;

#[derive(Clone, Debug)]
pub struct CommitInfo {
    pub oid: Oid,
    pub summary: String,
    /// Message body after the subject line; empty when subject-only.
    pub body: String,
    pub author: String,
    pub time_label: String,
}

fn commit_info_from(commit: &git2::Commit<'_>) -> CommitInfo {
    let author = commit.author();
    CommitInfo {
        oid: oid_from_git(commit.id()),
        summary: commit.summary().unwrap_or("(no subject)").to_string(),
        body: message_body(commit.message()),
        author: author.name().unwrap_or("?").to_string(),
        time_label: relative_time(author.when().seconds()),
    }
}

fn message_body(message: Option<&str>) -> String {
    let Some(msg) = message else {
        return String::new();
    };
    let mut lines = msg.split_inclusive('\n');
    let _subject = lines.next();
    lines.collect::<String>().trim().to_string()
}

#[derive(Clone, Debug)]
pub struct BranchInfo {
    pub name: String,
    pub oid: Oid,
    pub is_head: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
    pub tip: CommitInfo,
}

/// Local Branch Browser state: commits (newest first) and folded Comparison.
#[derive(Clone, Debug)]
pub struct BranchBrowser {
    pub comparison: Comparison,
    pub changed_paths: Vec<ChangedPath>,
    pub branch: String,
    pub commits: Vec<CommitInfo>,
    /// Parallel to `commits`: selected into Comparison range.
    pub in_range: Vec<bool>,
}

impl BranchBrowser {
    /// Open repo at HEAD; default Comparison = tip's parent..tip.
    pub fn open(path: &Path) -> Result<Self> {
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            err(format!("cannot resolve path {}: {e}", path.display()))
        })?;

        let repo = git2::Repository::open(&canonical).map_err(|e| {
            err(format!(
                "not a git repository ({}): {e}",
                canonical.display()
            ))
        })?;

        let repository = Repository::new(canonical);
        let branch = head_branch_name(&repo);
        let commits = list_first_parent_commits(&repo)?;
        if commits.is_empty() {
            return Err(err("repository has no commits"));
        }

        let mut in_range = vec![false; commits.len()];
        in_range[0] = true;

        let mut bb = Self {
            comparison: Comparison {
                repository,
                base_oid: commits[0].oid, // placeholder; set by fold
                head_oid: commits[0].oid,
            },
            changed_paths: Vec::new(),
            branch,
            commits,
            in_range,
        };
        apply_range_fold(&repo, &mut bb)?;
        Ok(bb)
    }

    /// Open Workspace entry; remember on success, drop_path on failure.
    pub fn open_workspace(entry: &WorkspaceEntry) -> Result<Self> {
        let result = Self::open(&entry.path).and_then(|mut bb| {
            bb.switch_branch(&entry.branch)?;
            Ok(bb)
        });
        match result {
            Ok(bb) => {
                workspace_store::remember(bb.comparison.repository.path(), &bb.branch);
                Ok(bb)
            }
            Err(e) => {
                workspace_store::drop_path(&entry.path);
                Err(e)
            }
        }
    }

    pub fn switch_branch(&mut self, name: &str) -> Result<()> {
        let repo =
            git2::Repository::open(self.comparison.repository.path()).map_err(map_git)?;
        let branches = list_branches(self.comparison.repository.path())?;
        let branch = branches
            .iter()
            .find(|b| b.name == name)
            .ok_or_else(|| err("branch not found"))?;
        let base_oid = {
            let head = repo.find_commit(oid_to_git(branch.oid)).map_err(map_git)?;
            head.parent_id(0).map(oid_from_git).map_err(map_git)?
        };
        self.branch = branch.name.clone();
        self.commits = list_commits_from(&repo, branch.oid)?;
        self.in_range = vec![false; self.commits.len()];
        if self.commits.is_empty() {
            return Err(err("branch has no commits"));
        }
        self.in_range[0] = true;
        self.comparison.base_oid = base_oid;
        self.comparison.head_oid = branch.oid;
        self.changed_paths = list_changed_paths(&repo, &self.comparison)?;
        Ok(())
    }

    /// Click / Shift+click a commit index (newest-first list) and rebuild Comparison.
    pub fn select_commit(&mut self, index: usize, shift: bool) -> Result<()> {
        if index >= self.commits.len() {
            return Err(err("commit index out of range"));
        }

        if shift {
            let first = self
                .in_range
                .iter()
                .position(|&v| v)
                .unwrap_or(index);
            let from = first.min(index);
            let to = first.max(index);
            for (i, flag) in self.in_range.iter_mut().enumerate() {
                *flag = i >= from && i <= to;
            }
        } else {
            for (i, flag) in self.in_range.iter_mut().enumerate() {
                *flag = i == index;
            }
        }

        let repo =
            git2::Repository::open(self.comparison.repository.path()).map_err(map_git)?;
        apply_range_fold(&repo, self)
    }

    /// Reload ChangedPaths for an explicit OID pair (future MR/PR entry).
    pub fn set_comparison_oids(&mut self, base_oid: Oid, head_oid: Oid) -> Result<()> {
        self.comparison.base_oid = base_oid;
        self.comparison.head_oid = head_oid;
        let repo = open_repo(&self.comparison)?;
        self.changed_paths = list_changed_paths(&repo, &self.comparison)?;
        // ponytail: in_range may disagree with OID pair; MR entry owns selection UI later
        Ok(())
    }
}

pub fn list_branches(path: &Path) -> Result<Vec<BranchInfo>> {
    let repo = git2::Repository::open(path).map_err(map_git)?;
    let mut branches = Vec::new();
    for item in repo.branches(None).map_err(map_git)? {
        let (branch, kind) = item.map_err(map_git)?;
        let Some(name) = branch.name().map_err(map_git)?.map(str::to_string) else {
            continue;
        };
        let Some(oid) = branch.get().target().map(oid_from_git) else {
            continue;
        };
        let commit = repo.find_commit(oid_to_git(oid)).map_err(map_git)?;
        let tip = commit_info_from(&commit);
        let upstream = (kind == git2::BranchType::Local)
            .then(|| branch.upstream().ok())
            .flatten()
            .and_then(|b| b.name().ok().flatten().map(str::to_string));
        branches.push(BranchInfo {
            name,
            oid,
            is_head: branch.is_head(),
            is_remote: kind == git2::BranchType::Remote,
            upstream,
            tip,
        });
    }
    let remote_upstreams: std::collections::HashSet<String> = branches
        .iter()
        .filter_map(|b| b.upstream.clone())
        .collect();
    branches.retain(|b| !b.is_remote || !remote_upstreams.contains(&format!("origin/{}", b.name)));
    branches.sort_by_key(|b| (!b.is_head, b.is_remote, std::cmp::Reverse(b.tip.oid.to_string())));
    Ok(branches)
}

fn apply_range_fold(repo: &git2::Repository, bb: &mut BranchBrowser) -> Result<()> {
    let range: Vec<usize> = bb
        .in_range
        .iter()
        .enumerate()
        .filter_map(|(i, &on)| on.then_some(i))
        .collect();
    if range.is_empty() {
        return Err(err("no commits selected"));
    }

    let head_idx = range[0];
    let last_idx = *range.last().unwrap();
    let head_oid = bb.commits[head_idx].oid;

    let base_oid = if last_idx + 1 < bb.commits.len() {
        bb.commits[last_idx + 1].oid
    } else {
        first_parent_oid(repo, bb.commits[last_idx].oid)?
    };

    bb.comparison = Comparison {
        repository: bb.comparison.repository.clone(),
        base_oid,
        head_oid,
    };
    bb.changed_paths = list_changed_paths(repo, &bb.comparison)?;
    Ok(())
}

fn first_parent_oid(repo: &git2::Repository, commit: Oid) -> Result<Oid> {
    let c = repo.find_commit(oid_to_git(commit)).map_err(map_git)?;
    if c.parent_count() == 0 {
        return Err(err(format!(
            "commit {} has no parent; cannot form Comparison",
            commit.short()
        )));
    }
    Ok(oid_from_git(c.parent_id(0).map_err(map_git)?))
}

fn list_commits_from(repo: &git2::Repository, start: Oid) -> Result<Vec<CommitInfo>> {
    let mut walk = repo.revwalk().map_err(map_git)?;
    walk.set_sorting(git2::Sort::NONE).map_err(map_git)?;
    walk.push(oid_to_git(start)).map_err(map_git)?;
    walk.simplify_first_parent().map_err(map_git)?;
    let mut out = Vec::new();
    for oid in walk.take(COMMIT_LIST_LIMIT) {
        let oid = oid.map_err(map_git)?;
        let commit = repo.find_commit(oid).map_err(map_git)?;
        out.push(commit_info_from(&commit));
    }
    Ok(out)
}

fn list_first_parent_commits(repo: &git2::Repository) -> Result<Vec<CommitInfo>> {
    let mut walk = repo.revwalk().map_err(map_git)?;
    walk.set_sorting(git2::Sort::NONE).map_err(map_git)?;
    // First-parent from HEAD tip.
    let head = repo
        .head()
        .map_err(map_git)?
        .peel_to_commit()
        .map_err(map_git)?;
    walk.push(head.id()).map_err(map_git)?;
    walk.simplify_first_parent().map_err(map_git)?;

    let mut out = Vec::new();
    for oid in walk {
        let oid = oid.map_err(map_git)?;
        let commit = repo.find_commit(oid).map_err(map_git)?;
        out.push(commit_info_from(&commit));
        if out.len() >= COMMIT_LIST_LIMIT {
            break;
        }
    }
    Ok(out)
}

fn relative_time(epoch_secs: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(epoch_secs);
    let ago = (now - epoch_secs).max(0);
    if ago < 60 {
        format!("{ago}s")
    } else if ago < 3600 {
        format!("{}m", ago / 60)
    } else if ago < 86400 {
        format!("{}h", ago / 3600)
    } else if ago < 86400 * 30 {
        format!("{}d", ago / 86400)
    } else {
        format!("{}mo", ago / (86400 * 30))
    }
}

fn head_branch_name(repo: &git2::Repository) -> String {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().map(str::to_string))
        .unwrap_or_else(|| "HEAD".into())
}

fn delta_status(delta: &git2::DiffDelta<'_>) -> Option<PathStatus> {
    match delta.status() {
        git2::Delta::Added => Some(PathStatus::Add),
        git2::Delta::Deleted => Some(PathStatus::Delete),
        git2::Delta::Modified
        | git2::Delta::Typechange
        | git2::Delta::Unreadable
        | git2::Delta::Unmodified => Some(PathStatus::Modify),
        // Copied/renamed shouldn't appear without find_similar; skip if they do.
        git2::Delta::Renamed
        | git2::Delta::Copied
        | git2::Delta::Ignored
        | git2::Delta::Untracked
        | git2::Delta::Conflicted => None,
    }
}

fn delta_path(delta: &git2::DiffDelta<'_>, status: PathStatus) -> Option<String> {
    let path = match status {
        PathStatus::Delete => delta.old_file().path(),
        _ => delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path()),
    }?;
    Some(path.to_string_lossy().replace('\\', "/"))
}

pub fn list_changed_paths(
    repo: &git2::Repository,
    comparison: &Comparison,
) -> Result<Vec<ChangedPath>> {
    let base = repo
        .find_commit(oid_to_git(comparison.base_oid))
        .map_err(map_git)?;
    let head = repo
        .find_commit(oid_to_git(comparison.head_oid))
        .map_err(map_git)?;
    let base_tree = base.tree().map_err(map_git)?;
    let head_tree = head.tree().map_err(map_git)?;

    // Renames OFF — DiffOptions default; do not call find_similar.
    let mut opts = git2::DiffOptions::new();
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))
        .map_err(map_git)?;

    let mut out: Vec<ChangedPath> = Vec::new();
    let mut line_counts: std::collections::HashMap<String, (u32, u32)> =
        std::collections::HashMap::new();

    diff.foreach(
        &mut |delta, _| {
            let Some(status) = delta_status(&delta) else {
                return true;
            };
            let Some(path) = delta_path(&delta, status) else {
                return true;
            };
            out.push(ChangedPath {
                path,
                status,
                additions: 0,
                deletions: 0,
            });
            true
        },
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            let Some(status) = delta_status(&delta) else {
                return true;
            };
            let Some(path) = delta_path(&delta, status) else {
                return true;
            };
            let entry = line_counts.entry(path).or_insert((0, 0));
            match line.origin() {
                '+' => entry.0 += 1,
                '-' => entry.1 += 1,
                _ => {}
            }
            true
        }),
    )
    .map_err(map_git)?;

    for path in &mut out {
        if let Some(&(add, del)) = line_counts.get(&path.path) {
            path.additions = add;
            path.deletions = del;
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn open_repo(comparison: &Comparison) -> Result<git2::Repository> {
    git2::Repository::open(comparison.repository.path()).map_err(map_git)
}

/// Lines of the blob on `side` of `path` within the Comparison (1-based indexing for callers).
pub fn side_lines(comparison: &Comparison, side: Side, path: &str) -> Result<Vec<String>> {
    let repo = open_repo(comparison)?;
    let oid = match side {
        Side::Old => comparison.base_oid,
        Side::New => comparison.head_oid,
    };
    let bytes = blob_text_at(&repo, oid, path)?.unwrap_or_default();
    if is_binary(&bytes) {
        return Err(err("binary file"));
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(split_lines(&text)
        .into_iter()
        .map(str::to_string)
        .collect())
}

fn blob_text_at(
    repo: &git2::Repository,
    commit_oid: Oid,
    path: &str,
) -> Result<Option<Vec<u8>>> {
    let commit = repo.find_commit(oid_to_git(commit_oid)).map_err(map_git)?;
    let tree = commit.tree().map_err(map_git)?;
    match tree.get_path(Path::new(path)) {
        Ok(entry) => {
            let obj = entry.to_object(repo).map_err(map_git)?;
            let Some(blob) = obj.as_blob() else {
                return Ok(None);
            };
            Ok(Some(blob.content().to_vec()))
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(map_git(e)),
    }
}

/// NUL or high control-byte density in the first 8KiB.
pub fn is_binary(data: &[u8]) -> bool {
    if data.contains(&0) {
        return true;
    }
    let sample = &data[..data.len().min(8000)];
    if sample.is_empty() {
        return false;
    }
    let suspicious = sample
        .iter()
        .filter(|&&b| b < 0x09 || (b > 0x0d && b < 0x20 && b != 0x1b))
        .count();
    suspicious * 100 > sample.len() * 10
}

#[derive(Clone, Debug)]
pub enum FileDiff {
    Text {
        display: DisplayRows,
        hunk_count: usize,
        alignment: Alignment,
        old_text: String,
        new_text: String,
    },
    Binary,
    Error(String),
}

/// Load old/new blobs for a path and compute Alignment under ViewOptions.
pub fn file_diff(
    comparison: &Comparison,
    path: &str,
    status: PathStatus,
    options: &ViewOptions,
) -> FileDiff {
    match file_diff_inner(comparison, path, status, options) {
        Ok(d) => d,
        Err(e) => FileDiff::Error(e.0),
    }
}

fn file_diff_inner(
    comparison: &Comparison,
    path: &str,
    status: PathStatus,
    options: &ViewOptions,
) -> Result<FileDiff> {
    let repo = open_repo(comparison)?;
    let old_bytes = match status {
        PathStatus::Add => Vec::new(),
        _ => blob_text_at(&repo, comparison.base_oid, path)?.unwrap_or_default(),
    };
    let new_bytes = match status {
        PathStatus::Delete => Vec::new(),
        _ => blob_text_at(&repo, comparison.head_oid, path)?.unwrap_or_default(),
    };

    if is_binary(&old_bytes) || is_binary(&new_bytes) {
        return Ok(FileDiff::Binary);
    }

    let old_text = String::from_utf8_lossy(&old_bytes).into_owned();
    let new_text = String::from_utf8_lossy(&new_bytes).into_owned();
    let alignment = compute_alignment(&old_text, &new_text, options);
    let hunk_count = alignment.hunks().len();
    let display = display_rows_folded(
        &old_text,
        &new_text,
        &alignment,
        &crate::domain::FoldState::collapsed(),
    );
    Ok(FileDiff::Text {
        display,
        hunk_count,
        alignment,
        old_text,
        new_text,
    })
}

pub fn compute_alignment(old_text: &str, new_text: &str, options: &ViewOptions) -> Alignment {
    // ponytail: ignore_whitespace stays false in v1 slice; wire when UI toggle exists
    let _ = options.ignore_whitespace;

    let old_lines = split_lines(old_text);
    let new_lines = split_lines(new_text);
    let diff = TextDiff::from_slices(&old_lines, &new_lines);

    let mut ops = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                ops.push(AlignmentOp::Equal {
                    old: LineSpan {
                        start: old_index as u32 + 1,
                        count: len as u32,
                    },
                    new: LineSpan {
                        start: new_index as u32 + 1,
                        count: len as u32,
                    },
                });
            }
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => {
                ops.push(AlignmentOp::Delete {
                    olds: LineSpan {
                        start: old_index as u32 + 1,
                        count: old_len as u32,
                    },
                    at_new: new_index as u32 + 1,
                });
            }
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => {
                ops.push(AlignmentOp::Insert {
                    after_old: old_index as u32,
                    news: LineSpan {
                        start: new_index as u32 + 1,
                        count: new_len as u32,
                    },
                });
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                ops.push(AlignmentOp::Replace {
                    olds: LineSpan {
                        start: old_index as u32 + 1,
                        count: old_len as u32,
                    },
                    news: LineSpan {
                        start: new_index as u32 + 1,
                        count: new_len as u32,
                    },
                });
            }
        }
    }
    Alignment { ops }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn temp_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "reviewfox-commits-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-m", "second"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("b.txt"), "new\n").unwrap();
        git(&dir, &["add", "a.txt", "b.txt"]);
        git(&dir, &["commit", "-m", "third"]);
        dir
    }

    #[test]
    fn forced_comparison_lists_paths_and_aligns() {
        let dir = temp_repo();
        let bb = BranchBrowser::open(&dir).expect("open temp repo");
        assert_eq!(bb.commits.len(), 3);
        assert!(bb.in_range[0]);
        assert!(!bb.in_range[1]);

        assert!(
            bb.changed_paths
                .iter()
                .any(|p| p.path == "a.txt" && p.status == PathStatus::Modify)
        );
        assert!(
            bb.changed_paths
                .iter()
                .any(|p| p.path == "b.txt" && p.status == PathStatus::Add && p.additions >= 1)
        );

        let diff = file_diff(
            &bb.comparison,
            "a.txt",
            PathStatus::Modify,
            &ViewOptions::default(),
        );
        match diff {
            FileDiff::Text {
                display,
                hunk_count,
                ..
            } => {
                assert!(hunk_count >= 1);
                assert!(
                    display
                        .new_rows
                        .iter()
                        .any(|r| r.kind == crate::domain::RowKind::Insert)
                );
            }
            other => panic!("expected text diff, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_commit_folds_range() {
        let dir = temp_repo();
        let mut bb = BranchBrowser::open(&dir).expect("open");
        // Select oldest non-root ("second") alone → first..second
        bb.select_commit(1, false).expect("select");
        assert!(bb.in_range[1]);
        assert!(!bb.in_range[0]);
        assert_eq!(bb.comparison.head_oid, bb.commits[1].oid);
        assert_eq!(bb.comparison.base_oid, bb.commits[2].oid);

        // Shift-extend to tip → first..third (base = first commit)
        bb.select_commit(0, true).expect("shift");
        assert!(bb.in_range[0] && bb.in_range[1]);
        assert!(!bb.in_range[2]);
        assert_eq!(bb.comparison.head_oid, bb.commits[0].oid);
        assert_eq!(bb.comparison.base_oid, bb.commits[2].oid);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_comparison_oids_reloads_changed_paths() {
        let dir = temp_repo();
        let mut bb = BranchBrowser::open(&dir).expect("open");
        let head = bb.commits[0].oid;
        let base = bb.commits[2].oid;
        bb.set_comparison_oids(base, head).expect("set oids");
        assert_eq!(bb.comparison.base_oid, base);
        assert_eq!(bb.comparison.head_oid, head);
        assert!(
            bb.changed_paths
                .iter()
                .any(|p| p.path == "a.txt" && p.status == PathStatus::Modify)
        );
        assert!(
            bb.changed_paths
                .iter()
                .any(|p| p.path == "b.txt" && p.status == PathStatus::Add)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_workspace_fails_on_missing_repo() {
        let entry = WorkspaceEntry {
            path: PathBuf::from("/tmp/reviewfox-no-such-repo-xyz"),
            branch: "main".into(),
        };
        // drop_path on failure is covered by workspace_store tests; here we only
        // assert the open path errors (and does not panic).
        let err = BranchBrowser::open_workspace(&entry).expect_err("must fail");
        assert!(!err.0.is_empty());
    }
}

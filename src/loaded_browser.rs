//! Loaded Branch Browser, plus an optional Uncommitted cover.
//! The main window is the only caller. No UI, filesystem, or MR HTTP.

use crate::domain::{ChangedPath, Comparison, Repository};
use crate::git::{BranchBrowser, CommitInfo, UncommittedEntry};

pub struct LoadedBrowser {
    browser: BranchBrowser,
    cover: Option<UncommittedEntry>,
}

impl LoadedBrowser {
    /// Starts uncovered. The caller assigns this over any previous value.
    pub fn install(browser: BranchBrowser) -> Self {
        Self {
            browser,
            cover: None,
        }
    }

    pub fn comparison(&self) -> &Comparison {
        match &self.cover {
            Some(uncommitted) => &uncommitted.comparison,
            None => &self.browser.comparison,
        }
    }

    pub fn changed_paths(&self) -> &[ChangedPath] {
        match &self.cover {
            Some(uncommitted) => &uncommitted.changed_paths,
            None => &self.browser.changed_paths,
        }
    }

    pub fn open_diff_allowed(&self) -> bool {
        !self.changed_paths().is_empty()
    }

    pub fn covering(&self) -> bool {
        self.cover.is_some()
    }

    pub fn checkout_label(&self) -> Option<&str> {
        self.cover
            .as_ref()
            .map(|uncommitted| uncommitted.checkout_label.as_str())
    }

    pub fn branch_name(&self) -> &str {
        &self.browser.branch
    }

    pub fn commits(&self) -> &[CommitInfo] {
        &self.browser.commits
    }

    pub fn in_range(&self) -> &[bool] {
        &self.browser.in_range
    }

    pub fn repository(&self) -> &Repository {
        &self.browser.comparison.repository
    }

    pub fn cover(&mut self, shell: UncommittedEntry) {
        self.cover = Some(shell);
    }

    pub fn uncover(&mut self) {
        self.cover = None;
    }

    /// Replaces the covering Uncommitted. Does nothing when uncovered.
    pub fn replace_cover(&mut self, entry: UncommittedEntry) {
        if self.cover.is_some() {
            self.cover = Some(entry);
        }
    }

    pub fn select_commit(&mut self, index: usize, shift: bool) -> crate::git::Result<()> {
        self.forward(|browser| browser.select_commit(index, shift))
    }

    pub fn apply_mr_commits(
        &mut self,
        commits: Vec<CommitInfo>,
        base_sha: &str,
        head_sha: &str,
    ) -> crate::git::Result<()> {
        self.forward(|browser| browser.apply_mr_commits(commits, base_sha, head_sha))
    }

    pub fn switch_branch(&mut self, name: &str) -> crate::git::Result<()> {
        self.forward(|browser| browser.switch_branch(name))
    }

    pub fn select_all_commits(&mut self) -> crate::git::Result<bool> {
        self.forward(|browser| browser.select_all_commits())
    }

    pub fn restore_mr_diff_refs(
        &mut self,
        base_sha: &str,
        head_sha: &str,
    ) -> crate::git::Result<bool> {
        self.forward(|browser| browser.restore_mr_diff_refs(base_sha, head_sha))
    }

    // BranchBrowser mutates, then opens a repo. Keep the clone only on Ok.
    fn forward<T>(
        &mut self,
        op: impl FnOnce(&mut BranchBrowser) -> crate::git::Result<T>,
    ) -> crate::git::Result<T> {
        let mut browser = self.browser.clone();
        let value = op(&mut browser)?;
        self.browser = browser;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::LoadedBrowser;
    use crate::domain::{ChangedPath, Comparison, Oid, PathStatus, Repository};
    use crate::git::{BranchBrowser, CommitInfo, UncommittedEntry};
    use std::path::PathBuf;

    fn oid(byte: u8) -> Oid {
        Oid::from_bytes([byte; 20])
    }

    fn repository() -> Repository {
        Repository::new(PathBuf::from("/tmp/reviewfox-repo"))
    }

    fn browser_comparison() -> Comparison {
        Comparison {
            repository: repository(),
            base_oid: Some(oid(1)),
            head_oid: oid(2),
            uncommitted: false,
        }
    }

    fn path(name: &str) -> ChangedPath {
        ChangedPath {
            path: name.into(),
            status: PathStatus::Modify,
            additions: 1,
            deletions: 0,
        }
    }

    fn commit(byte: u8, summary: &str) -> CommitInfo {
        CommitInfo {
            oid: oid(byte),
            summary: summary.into(),
            body: String::new(),
            author: "ada".into(),
            time_label: "1h".into(),
        }
    }

    fn browser(paths: Vec<ChangedPath>) -> BranchBrowser {
        BranchBrowser {
            comparison: browser_comparison(),
            changed_paths: paths,
            branch: "main".into(),
            commits: vec![commit(2, "tip"), commit(3, "parent")],
            in_range: vec![true, false],
        }
    }

    fn uncommitted(head: u8, paths: Vec<ChangedPath>, label: &str) -> UncommittedEntry {
        UncommittedEntry {
            comparison: Comparison {
                repository: repository(),
                base_oid: Some(oid(head)),
                head_oid: oid(head),
                uncommitted: true,
            },
            changed_paths: paths,
            checkout_label: label.into(),
        }
    }

    #[test]
    fn browser_with_paths_is_selected_and_open_diff_is_allowed() {
        let paths = vec![path("src/a.rs")];
        let loaded = LoadedBrowser::install(browser(paths.clone()));

        assert_eq!(loaded.comparison(), &browser_comparison());
        assert_eq!(loaded.changed_paths(), paths.as_slice());
        assert!(loaded.open_diff_allowed());
    }

    #[test]
    fn browser_without_paths_keeps_its_comparison_and_open_diff_stays_off() {
        let loaded = LoadedBrowser::install(browser(Vec::new()));

        assert_eq!(loaded.comparison(), &browser_comparison());
        assert!(loaded.changed_paths().is_empty());
        assert!(!loaded.open_diff_allowed());
    }

    #[test]
    fn cover_with_empty_paths_selects_the_uncommitted_and_keeps_the_browser() {
        let mut loaded = LoadedBrowser::install(browser(vec![path("src/a.rs")]));
        loaded.cover(uncommitted(9, Vec::new(), "feature"));

        assert!(loaded.covering());
        assert_eq!(loaded.checkout_label(), Some("feature"));
        assert_eq!(loaded.comparison().head_oid, oid(9));
        assert!(loaded.comparison().uncommitted);
        assert!(loaded.changed_paths().is_empty());
        assert!(!loaded.open_diff_allowed());
        assert_eq!(loaded.branch_name(), "main");
        assert_eq!(
            loaded
                .commits()
                .iter()
                .map(|c| c.summary.as_str())
                .collect::<Vec<_>>(),
            ["tip", "parent"]
        );
        assert_eq!(loaded.in_range(), &[true, false]);
        assert_eq!(loaded.repository().path(), repository().path());
    }

    #[test]
    fn replacing_the_cover_selects_the_new_head_and_allows_open_diff() {
        let mut loaded = LoadedBrowser::install(browser(Vec::new()));
        loaded.cover(uncommitted(9, Vec::new(), "feature"));
        let scanned = uncommitted(8, vec![path("src/b.rs")], "0808080");
        loaded.replace_cover(scanned);

        assert!(loaded.covering());
        assert_eq!(loaded.comparison().head_oid, oid(8));
        assert_eq!(loaded.checkout_label(), Some("0808080"));
        assert_eq!(
            loaded
                .changed_paths()
                .iter()
                .map(|p| p.path.as_str())
                .collect::<Vec<_>>(),
            ["src/b.rs"]
        );
        assert!(loaded.open_diff_allowed());
        assert_eq!(loaded.branch_name(), "main");
    }

    #[test]
    fn select_commit_out_of_range_while_covered_leaves_the_uncommitted_selected() {
        let mut loaded = LoadedBrowser::install(browser(vec![path("src/a.rs")]));
        loaded.cover(uncommitted(9, Vec::new(), "feature"));

        let err = loaded.select_commit(9, false).expect_err("out of range");
        assert!(!err.to_string().is_empty());

        assert!(loaded.covering());
        assert_eq!(loaded.comparison().head_oid, oid(9));
        assert!(loaded.comparison().uncommitted);
        assert!(loaded.changed_paths().is_empty());
        assert_eq!(loaded.checkout_label(), Some("feature"));
        assert_eq!(loaded.branch_name(), "main");
        assert_eq!(
            loaded.commits().iter().map(|c| c.oid).collect::<Vec<_>>(),
            [oid(2), oid(3)]
        );
        assert_eq!(loaded.in_range(), &[true, false]);
    }

    #[test]
    fn apply_mr_commits_with_no_commits_while_covered_leaves_the_uncommitted_selected() {
        let mut loaded = LoadedBrowser::install(browser(vec![path("src/a.rs")]));
        loaded.cover(uncommitted(9, Vec::new(), "feature"));

        loaded
            .apply_mr_commits(Vec::new(), "not-used", "not-used")
            .expect_err("empty commit list");

        assert!(loaded.covering());
        assert_eq!(loaded.comparison().head_oid, oid(9));
        assert!(loaded.comparison().uncommitted);
        assert_eq!(loaded.branch_name(), "main");
        assert_eq!(
            loaded
                .commits()
                .iter()
                .map(|c| c.summary.as_str())
                .collect::<Vec<_>>(),
            ["tip", "parent"]
        );
        assert_eq!(loaded.in_range(), &[true, false]);
    }

    #[test]
    fn uncover_selects_the_browser_comparison_again() {
        let mut loaded = LoadedBrowser::install(browser(vec![path("src/a.rs")]));
        loaded.cover(uncommitted(9, Vec::new(), "feature"));
        loaded.uncover();

        assert!(!loaded.covering());
        assert_eq!(loaded.checkout_label(), None);
        assert_eq!(loaded.comparison(), &browser_comparison());
        assert_eq!(
            loaded
                .changed_paths()
                .iter()
                .map(|p| p.path.as_str())
                .collect::<Vec<_>>(),
            ["src/a.rs"]
        );
        assert!(loaded.open_diff_allowed());
        assert_eq!(loaded.branch_name(), "main");
        assert_eq!(loaded.in_range(), &[true, false]);
    }
}

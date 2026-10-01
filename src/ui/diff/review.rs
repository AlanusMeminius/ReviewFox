//! The open Diff's one Review for its Comparison, and the unsaved line dock.
//! The dock is ephemeral and not a domain term. No GPUI.

use crate::domain::{Anchor, Comparison, DraftComment, LineSpan, Review, Side};
use crate::export::{CommentContext, ReviewContexts};
use crate::{
    publication::{PublicationStore, ReviewOrigin},
    review_store::ReviewStore,
};
use std::sync::Arc;

/// What one operation left on the current path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathView {
    /// Line and file DraftComments on the remembered path.
    pub comments: Vec<DraftComment>,
    /// Set while the unsaved line dock is open.
    pub dock: Option<LineDock>,
    /// Set only by [`OpenReview::begin_edit`] when that call opens the dock.
    pub body: Option<String>,
}

/// The open unsaved dock: which line span it writes, and which DraftComment
/// it rewrites (`None` while creating one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDock {
    pub side: Side,
    pub span: LineSpan,
    pub editing: Option<u64>,
}

/// Single in-memory slot: the Review for the Comparison on screen.
#[derive(Debug)]
pub struct OpenReview {
    review: Review,
    path: String,
    dock: Option<LineDock>,
    contexts: ReviewContexts,
    draft_context: Option<Arc<CommentContext>>,
    origin: ReviewOrigin,
    publication_store: Option<PublicationStore>,
    store: Option<ReviewStore>,
    storage_error: Option<String>,
    load_failed: bool,
}

impl OpenReview {
    pub fn new(comparison: Comparison, path: impl Into<String>) -> Self {
        Self {
            review: Review::new(comparison),
            path: path.into(),
            dock: None,
            contexts: ReviewContexts::new(),
            draft_context: None,
            origin: ReviewOrigin::Local,
            publication_store: None,
            store: None,
            storage_error: None,
            load_failed: false,
        }
    }

    pub fn with_publication_store(mut self, store: PublicationStore) -> Self {
        self.publication_store = Some(store);
        self
    }

    pub fn unavailable(
        comparison: Comparison,
        path: impl Into<String>,
        origin: ReviewOrigin,
        error: String,
    ) -> Self {
        let mut open = Self::new(comparison, path);
        open.origin = origin;
        open.storage_error = Some(error);
        open.load_failed = true;
        open
    }

    pub fn reopen(
        comparison: Comparison,
        path: impl Into<String>,
        origin: ReviewOrigin,
        store: ReviewStore,
    ) -> Self {
        let loaded = store.load(&comparison);
        let mut open = Self::new(comparison, path);
        open.origin = origin;
        open.store = Some(store);
        match loaded {
            Ok(Some(saved)) => {
                open.review = saved.review;
                open.contexts = saved.contexts;
            }
            Ok(None) => {}
            Err(e) => {
                open.storage_error = Some(format!("{e:#}"));
                open.load_failed = true;
            }
        }
        open
    }

    pub fn origin(&self) -> &ReviewOrigin {
        &self.origin
    }
    pub fn storage_error(&self) -> Option<&str> {
        self.storage_error.as_deref()
    }
    pub fn publication_eligibility(&self) -> Result<(), &str> {
        if self.storage_error.is_some() {
            return Err("Review storage failed · publication unavailable");
        }
        self.origin.eligibility(&self.review.comparison)
    }
    fn persist(&mut self) {
        if self.load_failed {
            return;
        }
        if let Some(store) = &self.store {
            self.storage_error = store
                .save(&self.review, &self.contexts)
                .err()
                .map(|e| format!("{e:#}"));
        }
    }

    pub fn show_origin(
        &mut self,
        comparison: Comparison,
        path: impl Into<String>,
        origin: ReviewOrigin,
    ) -> PathView {
        let path = path.into();
        if self.review.comparison != comparison {
            let publications = self.publication_store.clone();
            if let Some(store) = self.store.clone() {
                *self = Self::reopen(comparison, path, origin, store);
            } else {
                let error = self.storage_error.clone();
                *self = match error {
                    Some(error) => Self::unavailable(comparison, path, origin, error),
                    None => {
                        let mut open = Self::new(comparison, path);
                        open.origin = origin;
                        open
                    }
                };
            }
            self.publication_store = publications;
        } else {
            self.origin = origin;
            self.set_path(path);
        }
        self.snapshot(None)
    }

    pub fn review(&self) -> &Review {
        &self.review
    }

    pub fn contexts(&self) -> &ReviewContexts {
        &self.contexts
    }

    /// Freeze the text the author selected, not whatever is on disk at Copy time.
    pub fn capture_draft_context(&mut self, preimage: Arc<str>, postimage: Arc<str>) {
        if self.dock.is_none_or(|dock| dock.editing.is_some()) {
            return;
        }
        let context = CommentContext {
            preimage,
            postimage,
        };
        // Comments created against the same file text share the underlying snapshot.
        self.draft_context = Some(
            self.contexts
                .values()
                .find(|saved| saved.as_ref() == &context)
                .cloned()
                .unwrap_or_else(|| Arc::new(context)),
        );
    }

    pub fn dock(&self) -> Option<LineDock> {
        self.dock
    }

    /// Current path, without signaling an edit body.
    pub fn current(&self) -> PathView {
        self.snapshot(None)
    }

    /// Same Comparison keeps this Review. Same path keeps the dock.
    /// A different Comparison replaces the Review and clears the dock.
    #[cfg(test)]
    pub fn show(&mut self, comparison: Comparison, path: impl Into<String>) -> PathView {
        self.show_origin(comparison, path, ReviewOrigin::Local)
    }

    /// Tree click inside this Diff. Same path rule as [`Self::show`].
    pub fn select_path(&mut self, path: impl Into<String>) -> PathView {
        self.set_path(path.into());
        self.snapshot(None)
    }

    /// Open the dock for a new line comment. `side` is the caller's selection.
    pub fn begin_draft(&mut self, side: Side, start: u32, count: u32) -> PathView {
        self.draft_context = None;
        self.dock = Some(LineDock {
            side,
            span: LineSpan { start, count },
            editing: None,
        });
        self.snapshot(None)
    }

    /// Open the dock on a line Anchor and return its body once.
    /// Unknown id or a file Anchor leaves the dock as it was.
    pub fn begin_edit(&mut self, id: u64) -> PathView {
        let hit = self
            .review
            .comments
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| match &c.anchor {
                Anchor::Line { side, span, .. } => Some((c.body.clone(), *side, *span)),
                Anchor::File { .. } => None,
            });
        let Some((body, side, span)) = hit else {
            return self.snapshot(None);
        };
        self.draft_context = None;
        self.dock = Some(LineDock {
            side,
            span,
            editing: Some(id),
        });
        self.snapshot(Some(body))
    }

    /// Trim `body`. Empty cancels. Editing rewrites the body; creating adds a
    /// line DraftComment on the remembered path (Hunk stays `None`).
    pub fn commit(&mut self, body: &str) -> PathView {
        let Some(dock) = self.dock.take() else {
            return self.snapshot(None);
        };
        let context = self.draft_context.take();
        let body = body.trim();
        if body.is_empty() {
            return self.snapshot(None);
        }
        match dock.editing {
            Some(id) => {
                self.review.update_comment_body(id, body);
            }
            None => {
                let comment = self.review.add_line_span_comment(
                    self.path.clone(),
                    dock.side,
                    dock.span.start,
                    dock.span.count,
                    body,
                );
                if let Some(context) = context {
                    self.contexts.insert(comment.id, context);
                }
            }
        }
        self.persist();
        self.snapshot(None)
    }

    /// Clear the dock. DraftComments stay.
    pub fn cancel(&mut self) -> PathView {
        self.dock = None;
        self.draft_context = None;
        self.snapshot(None)
    }

    /// Remove that DraftComment. Ids are not reused. Editing that id clears the dock.
    pub fn delete(&mut self, id: u64) -> PathView {
        if let Some(store) = &self.publication_store {
            match store.protected_comments(&self.review.comparison) {
                Ok(protected) if protected.contains(&id) => return self.snapshot(None),
                Err(error) => {
                    self.storage_error = Some(format!("{error:#}"));
                    return self.snapshot(None);
                }
                _ => {}
            }
        }
        self.review.delete_comment(id);
        self.contexts.remove(&id);
        if self.dock.is_some_and(|d| d.editing == Some(id)) {
            self.dock = None;
            self.draft_context = None;
        }
        self.persist();
        self.snapshot(None)
    }

    fn set_path(&mut self, path: String) {
        if self.path != path {
            self.path = path;
            self.dock = None;
            self.draft_context = None;
        }
    }

    fn snapshot(&self, body: Option<String>) -> PathView {
        PathView {
            comments: self.review.comments_for_path(&self.path).cloned().collect(),
            dock: self.dock,
            body,
        }
    }
}

#[cfg(test)]
impl OpenReview {
    // ponytail: the dock never creates a file Anchor; tests plant one.
    // Id stays off Review's sequence so a later line comment is not this id.
    fn insert_file_comment(&mut self, path: &str, body: &str) -> u64 {
        let id = u64::MAX;
        self.review.comments.push(DraftComment {
            id,
            body: body.to_string(),
            anchor: Anchor::File {
                path: path.to_string(),
            },
        });
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Oid, Repository};
    use std::path::PathBuf;

    fn cmp(head: u8) -> Comparison {
        Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Some(Oid::from_bytes([1; 20])),
            head_oid: Oid::from_bytes([head; 20]),
            uncommitted: false,
        }
    }

    fn uncommitted(head: u8) -> Comparison {
        let oid = Oid::from_bytes([head; 20]);
        Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Some(oid),
            head_oid: oid,
            uncommitted: true,
        }
    }

    fn line(id: u64, path: &str, side: Side, start: u32, count: u32, body: &str) -> DraftComment {
        DraftComment {
            id,
            body: body.into(),
            anchor: Anchor::Line {
                path: path.into(),
                side,
                span: LineSpan { start, count },
                hunk: None,
            },
        }
    }

    fn mr(iid: u64) -> ReviewOrigin {
        ReviewOrigin::GitLab {
            base_url: "https://gitlab.example.com".into(),
            project: "team/repo".into(),
            iid,
            base_sha: Oid::from_bytes([1; 20]).to_string(),
            start_sha: Some(Oid::from_bytes([3; 20]).to_string()),
            head_sha: Oid::from_bytes([2; 20]).to_string(),
        }
    }

    #[test]
    fn same_comparison_shares_local_work_with_independent_originating_targets() {
        let directory = tempfile::tempdir().unwrap();
        let store = ReviewStore::new(directory.path().into());
        let mut open = OpenReview::reopen(cmp(2), "a.rs", mr(10), store);
        assert_eq!(open.publication_eligibility(), Ok(()));
        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context("before\n".into(), "after\n".into());
        open.commit("shared local work");
        let exported = crate::export::export_review(open.review(), open.contexts());
        assert_eq!(open.origin(), &mr(10));
        assert_eq!(
            open.show_origin(cmp(2), "a.rs", mr(11)).comments[0].body,
            "shared local work"
        );
        assert_eq!(open.origin(), &mr(11));
        assert_eq!(
            crate::export::export_review(open.review(), open.contexts()),
            exported
        );
        open.begin_edit(1);
        open.commit("edited locally");
        assert_eq!(
            open.show_origin(cmp(2), "a.rs", mr(10)).comments[0].body,
            "edited locally"
        );
        assert_eq!(open.origin(), &mr(10));
        assert_eq!(
            open.show_origin(cmp(2), "a.rs", ReviewOrigin::Local)
                .comments[0]
                .body,
            "edited locally"
        );
        assert!(open.publication_eligibility().is_err());
        let store = open.store.clone().unwrap();
        drop(open);
        let reopened = OpenReview::reopen(cmp(2), "a.rs", mr(12), store);
        assert_eq!(reopened.current().comments[0].body, "edited locally");
        assert_eq!(reopened.origin(), &mr(12));
    }

    #[test]
    fn only_complete_mr_versions_have_publication_eligibility() {
        let directory = tempfile::tempdir().unwrap();
        let store = ReviewStore::new(directory.path().into());
        let mut open = OpenReview::reopen(cmp(2), "a.rs", mr(10), store);
        assert_eq!(open.publication_eligibility(), Ok(()));
        open.show_origin(cmp(4), "a.rs", mr(10));
        assert!(
            open.publication_eligibility()
                .unwrap_err()
                .contains("subset")
        );
        open.show_origin(uncommitted(2), "a.rs", ReviewOrigin::Local);
        assert!(open.publication_eligibility().is_err());
        let mut incomplete = mr(10);
        if let ReviewOrigin::GitLab { start_sha, .. } = &mut incomplete {
            *start_sha = None;
        }
        open.show_origin(cmp(2), "a.rs", incomplete);
        assert!(
            open.publication_eligibility()
                .unwrap_err()
                .contains("incomplete")
        );
    }

    #[test]
    fn unreadable_and_future_review_storage_is_reported_and_preserved() {
        for replacement in [b"broken JSON".as_slice(), br#"{"version":42}"#.as_slice()] {
            let directory = tempfile::tempdir().unwrap();
            let store = ReviewStore::new(directory.path().into());
            let mut open = OpenReview::reopen(cmp(2), "a.rs", mr(10), store.clone());
            open.begin_draft(Side::Postimage, 1, 1);
            open.commit("existing work");
            let path = std::fs::read_dir(directory.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            std::fs::write(&path, replacement).unwrap();
            let mut reopened = OpenReview::reopen(cmp(2), "a.rs", mr(10), store);
            assert!(reopened.storage_error().unwrap().contains("preserved"));
            assert!(reopened.publication_eligibility().is_err());
            reopened.begin_draft(Side::Postimage, 1, 1);
            reopened.commit("new in-memory work");
            assert_eq!(std::fs::read(path).unwrap(), replacement);
        }
    }

    #[test]
    fn malformed_stable_comment_identity_is_reported_instead_of_reused() {
        let directory = tempfile::tempdir().unwrap();
        let store = ReviewStore::new(directory.path().into());
        let mut open = OpenReview::reopen(cmp(2), "a.rs", mr(10), store.clone());
        open.begin_draft(Side::Postimage, 1, 1);
        open.commit("existing work");
        let path = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        data["review"]["next_id"] = 1.into();
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let reopened = OpenReview::reopen(cmp(2), "a.rs", mr(10), store);
        assert!(reopened.storage_error().is_some());
    }

    #[test]
    fn saved_review_reopens_with_identity_anchor_and_creation_context() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::review_store::ReviewStore::new(directory.path().into());
        let mut open = OpenReview::reopen(
            cmp(2),
            "a.rs",
            crate::publication::ReviewOrigin::Local,
            store.clone(),
        );
        open.begin_draft(Side::Postimage, 3, 2);
        open.capture_draft_context("old\n".into(), "selected\n".into());
        open.commit("draft");
        open.begin_edit(1);
        open.commit("edited");
        let export = crate::export::export_review(open.review(), open.contexts());
        drop(open);
        let mut reopened = OpenReview::reopen(
            cmp(2),
            "a.rs",
            crate::publication::ReviewOrigin::Local,
            store.clone(),
        );
        assert_eq!(
            reopened.current().comments,
            vec![line(1, "a.rs", Side::Postimage, 3, 2, "edited")]
        );
        assert_eq!(
            crate::export::export_review(reopened.review(), reopened.contexts()),
            export
        );
        reopened.delete(1);
        drop(reopened);
        let mut reopened = OpenReview::reopen(
            cmp(2),
            "a.rs",
            crate::publication::ReviewOrigin::Local,
            store,
        );
        assert!(reopened.current().comments.is_empty());
        reopened.begin_draft(Side::Preimage, 1, 1);
        assert_eq!(reopened.commit("next").comments[0].id, 2);
    }

    #[test]
    fn submit_trims_and_blank_or_whitespace_cancels() {
        let mut open = OpenReview::new(cmp(2), "src/a.rs");
        open.begin_draft(Side::Postimage, 3, 2);
        let blank = open.commit("  \n");
        assert!(blank.comments.is_empty());
        assert!(blank.dock.is_none());
        assert!(blank.body.is_none());

        open.begin_draft(Side::Postimage, 3, 2);
        let whitespace = open.commit(" \t ");
        assert!(whitespace.comments.is_empty());
        assert!(whitespace.dock.is_none());

        let drafted = open.begin_draft(Side::Postimage, 3, 2);
        assert_eq!(
            drafted.dock,
            Some(LineDock {
                side: Side::Postimage,
                span: LineSpan { start: 3, count: 2 },
                editing: None,
            })
        );
        assert!(drafted.body.is_none());
        let saved = open.commit("  nits  ");
        assert!(saved.dock.is_none());
        assert_eq!(
            saved.comments,
            vec![line(1, "src/a.rs", Side::Postimage, 3, 2, "nits")]
        );
        assert_eq!(open.review().comments, saved.comments);

        open.begin_draft(Side::Preimage, 1, 1);
        let cancelled = open.cancel();
        assert!(cancelled.dock.is_none());
        assert_eq!(cancelled.comments, saved.comments);
    }

    #[test]
    fn edit_changes_body_only_and_returns_it_once() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        open.begin_draft(Side::Postimage, 4, 3);
        open.commit("first");
        let started = open.begin_edit(1);
        assert_eq!(started.body.as_deref(), Some("first"));
        assert_eq!(
            started.dock,
            Some(LineDock {
                side: Side::Postimage,
                span: LineSpan { start: 4, count: 3 },
                editing: Some(1),
            })
        );
        assert!(open.current().body.is_none());
        let updated = open.commit("  second  ");
        assert!(updated.dock.is_none());
        assert!(updated.body.is_none());
        assert_eq!(
            updated.comments,
            vec![line(1, "a.rs", Side::Postimage, 4, 3, "second")]
        );
    }

    #[test]
    fn delete_drops_one_comment_and_closes_the_dock_when_editing_it() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.commit("keep");
        open.begin_draft(Side::Postimage, 2, 1);
        open.commit("drop");
        open.begin_edit(2);
        let gone = open.delete(2);
        assert!(gone.dock.is_none());
        assert_eq!(
            gone.comments,
            vec![line(1, "a.rs", Side::Postimage, 1, 1, "keep")]
        );
        let same = open.delete(2);
        assert_eq!(same.comments, gone.comments);

        let draft = open.begin_draft(Side::Preimage, 9, 1);
        let kept_dock = open.delete(1);
        assert_eq!(kept_dock.dock, draft.dock);
        assert!(kept_dock.comments.is_empty());
        assert!(open.review().comments.is_empty());
    }

    #[test]
    fn unknown_id_and_file_anchor_leave_the_dock_unchanged() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        let missing = open.begin_edit(7);
        assert!(missing.dock.is_none());
        assert!(missing.body.is_none());
        assert!(missing.comments.is_empty());

        let file_id = open.insert_file_comment("a.rs", "whole file");
        let drafted = open.begin_draft(Side::Preimage, 5, 2);
        let unknown = open.begin_edit(7);
        assert!(unknown.body.is_none());
        assert_eq!(unknown.dock, drafted.dock);
        let still = open.begin_edit(file_id);
        assert!(still.body.is_none());
        assert_eq!(still.dock, drafted.dock);
        assert!(
            still
                .comments
                .iter()
                .any(|c| matches!(c.anchor, Anchor::File { .. }))
        );

        let saved = open.commit("line");
        assert!(saved.dock.is_none());
        assert_eq!(saved.comments.len(), 2);
        assert!(saved.comments.iter().any(|c| c.id == file_id));
        assert!(saved.comments.iter().any(|c| c.body == "line"));
    }

    #[test]
    fn new_comment_does_not_reuse_a_deleted_id() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        let first = open.commit("a");
        let id = first.comments[0].id;
        open.delete(id);
        open.begin_draft(Side::Postimage, 2, 1);
        let second = open.commit("b");
        assert_eq!(second.comments.len(), 1);
        assert_ne!(second.comments[0].id, id);
        assert!(second.comments[0].id > id);
    }

    #[test]
    fn several_comments_stay_on_other_paths() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.commit("one");
        open.begin_draft(Side::Postimage, 4, 2);
        let both = open.commit("two");
        assert_eq!(both.comments.len(), 2);
        assert_eq!(both.comments[0].id, 1);
        assert_eq!(both.comments[1].id, 2);

        open.begin_draft(Side::Postimage, 8, 1);
        let other = open.select_path("b.rs");
        assert!(other.dock.is_none());
        assert!(other.comments.is_empty());
        assert_eq!(open.review().comments.len(), 2);

        let back = open.select_path("a.rs");
        assert!(back.dock.is_none());
        assert_eq!(back.comments.len(), 2);
        assert!(back.comments.iter().all(|c| c.anchor.path() == "a.rs"));
    }

    #[test]
    fn same_path_keeps_the_dock_and_comparison_swaps_the_review() {
        let comparison = cmp(2);
        let mut open = OpenReview::new(comparison.clone(), "a.rs");
        open.begin_draft(Side::Postimage, 3, 1);
        open.commit("saved");
        let open_dock = open.begin_draft(Side::Preimage, 6, 2);

        let same = open.show(comparison.clone(), "a.rs");
        assert_eq!(same.dock, open_dock.dock);
        assert_eq!(same.comments.len(), 1);

        let same_path = open.select_path("a.rs");
        assert_eq!(same_path.dock, open_dock.dock);

        let other_path = open.show(comparison.clone(), "b.rs");
        assert!(other_path.dock.is_none());
        assert!(other_path.comments.is_empty());
        assert_eq!(open.review().comments.len(), 1);
        let ignored = open.commit("nope");
        assert!(ignored.comments.is_empty());
        assert_eq!(open.review().comments[0].anchor.path(), "a.rs");

        let mut next = comparison;
        next.head_oid = Oid::from_bytes([3; 20]);
        let replaced = open.show(next.clone(), "a.rs");
        assert!(replaced.comments.is_empty());
        assert!(replaced.dock.is_none());
        assert!(open.review().comments.is_empty());
        assert_eq!(open.review().comparison, next);
    }

    #[test]
    fn uncommitted_comparison_uses_partial_eq() {
        let mut open = OpenReview::new(uncommitted(4), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.commit("on checkout");
        let kept = open.show(uncommitted(4), "b.rs");
        assert!(kept.dock.is_none());
        assert!(kept.comments.is_empty());
        assert_eq!(open.review().comments.len(), 1);

        let moved = open.show(uncommitted(5), "b.rs");
        assert!(moved.comments.is_empty());
        assert!(moved.dock.is_none());
        assert!(open.review().comments.is_empty());

        let mut open = OpenReview::new(uncommitted(4), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.commit("again");
        let mut as_commit = uncommitted(4);
        as_commit.uncommitted = false;
        open.show(as_commit, "a.rs");
        assert!(open.review().comments.is_empty());
    }

    #[test]
    fn preimage_and_postimage_spans_are_kept() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        let pre = open.begin_draft(Side::Preimage, 2, 4);
        assert_eq!(pre.dock.unwrap().side, Side::Preimage);
        assert_eq!(pre.dock.unwrap().span, LineSpan { start: 2, count: 4 });
        open.commit("removed");
        let post = open.begin_draft(Side::Postimage, 8, 1);
        assert_eq!(post.dock.unwrap().side, Side::Postimage);
        assert_eq!(
            post.comments.len(),
            1,
            "the preimage comment stays while drafting"
        );
        let both = open.commit("added");
        assert!(both.dock.is_none());
        assert_eq!(
            both.comments,
            vec![
                line(1, "a.rs", Side::Preimage, 2, 4, "removed"),
                line(2, "a.rs", Side::Postimage, 8, 1, "added"),
            ]
        );
    }

    #[test]
    fn context_survives_navigation_and_body_edit_but_not_delete_or_comparison_change() {
        let mut open = OpenReview::new(uncommitted(4), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context("old\n".into(), "selected\n".into());
        open.commit("first");
        let saved = open.contexts().get(&1).unwrap().clone();
        open.select_path("b.rs");
        open.select_path("a.rs");
        open.begin_edit(1);
        open.capture_draft_context("old\n".into(), "changed since selection\n".into());
        open.commit("edited");
        assert!(Arc::ptr_eq(&saved, open.contexts().get(&1).unwrap()));
        let text = crate::export::export_review(open.review(), open.contexts());
        assert!(text.contains("+selected"));
        assert!(text.contains("Body:\nedited"));
        assert!(!text.contains("changed since selection"));

        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context("old\n".into(), "selected\n".into());
        open.commit("second");
        assert!(Arc::ptr_eq(
            open.contexts().get(&1).unwrap(),
            open.contexts().get(&2).unwrap()
        ));
        open.delete(1);
        assert!(!open.contexts().contains_key(&1));
        open.show(uncommitted(5), "a.rs");
        assert!(open.contexts().is_empty());
    }

    #[test]
    fn cancelled_draft_does_not_supply_context_to_a_later_comment() {
        let mut open = OpenReview::new(cmp(2), "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context("old\n".into(), "cancelled\n".into());
        open.cancel();
        open.begin_draft(Side::Postimage, 1, 1);
        open.commit("no context");
        assert!(open.contexts().is_empty());
    }

    #[test]
    fn uncommitted_export_uses_loaded_text_after_file_is_changed_and_removed() {
        use crate::domain::{PathStatus, ViewOptions};
        use crate::git::{self, FileDiff};
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "before\n").unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "a.rs"],
            vec!["commit", "--quiet", "-m", "base"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::write(&path, "selected\n").unwrap();
        let repository = Repository::new(dir.path().to_path_buf());
        let comparison = git::uncommitted_head(&repository).unwrap().comparison;
        let FileDiff::Text {
            preimage_text,
            postimage_text,
            ..
        } = git::file_diff(
            &comparison,
            "a.rs",
            PathStatus::Modify,
            &ViewOptions::default(),
        )
        else {
            panic!("expected text diff");
        };
        let mut open = OpenReview::new(comparison, "a.rs");
        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context(preimage_text, postimage_text);
        // A disk edit while the comment editor is open must not change its context.
        std::fs::write(&path, "later\n").unwrap();
        open.commit("review selected version");
        open.select_path("another.rs");
        std::fs::remove_file(&path).unwrap();
        let text = crate::export::export_review(open.review(), open.contexts());
        assert!(text.contains("-before\n+selected"));
        assert!(!text.contains("later"));
        assert!(!text.contains("Unavailable"));
    }
}

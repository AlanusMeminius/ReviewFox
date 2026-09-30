//! The open Diff's one Review for its Comparison, and the unsaved line dock.
//! The dock is ephemeral and not a domain term. No GPUI.

use crate::domain::{Anchor, Comparison, DraftComment, LineSpan, Review, Side};

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
}

impl OpenReview {
    pub fn new(comparison: Comparison, path: impl Into<String>) -> Self {
        Self {
            review: Review::new(comparison),
            path: path.into(),
            dock: None,
        }
    }

    pub fn review(&self) -> &Review {
        &self.review
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
    pub fn show(&mut self, comparison: Comparison, path: impl Into<String>) -> PathView {
        let path = path.into();
        if self.review.comparison != comparison {
            self.review = Review::new(comparison);
            self.path = path;
            self.dock = None;
        } else {
            self.set_path(path);
        }
        self.snapshot(None)
    }

    /// Tree click inside this Diff. Same path rule as [`Self::show`].
    pub fn select_path(&mut self, path: impl Into<String>) -> PathView {
        self.set_path(path.into());
        self.snapshot(None)
    }

    /// Open the dock for a new line comment. `side` is the caller's selection.
    pub fn begin_draft(&mut self, side: Side, start: u32, count: u32) -> PathView {
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
        let body = body.trim();
        if body.is_empty() {
            return self.snapshot(None);
        }
        match dock.editing {
            Some(id) => {
                self.review.update_comment_body(id, body);
            }
            None => {
                self.review.add_line_span_comment(
                    self.path.clone(),
                    dock.side,
                    dock.span.start,
                    dock.span.count,
                    body,
                );
            }
        }
        self.snapshot(None)
    }

    /// Clear the dock. DraftComments stay.
    pub fn cancel(&mut self) -> PathView {
        self.dock = None;
        self.snapshot(None)
    }

    /// Remove that DraftComment. Ids are not reused. Editing that id clears the dock.
    pub fn delete(&mut self, id: u64) -> PathView {
        self.review.delete_comment(id);
        if self.dock.is_some_and(|d| d.editing == Some(id)) {
            self.dock = None;
        }
        self.snapshot(None)
    }

    fn set_path(&mut self, path: String) {
        if self.path != path {
            self.path = path;
            self.dock = None;
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
}

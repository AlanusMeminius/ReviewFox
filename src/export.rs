//! Agent-oriented clipboard Export. Comment targets remain separate from diff context.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use crate::domain::{AlignmentOp, Anchor, DraftComment, Review, Side};
use crate::git;

const CONTEXT_RADIUS: usize = 2;

/// Original text visible when a comment was created, independent of ViewOptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentContext {
    pub preimage: Arc<str>,
    pub postimage: Arc<str>,
}

pub type ReviewContexts = HashMap<u64, Arc<CommentContext>>;

struct Group<'a> {
    path: &'a str,
    context: Option<&'a CommentContext>,
    comments: Vec<(usize, &'a DraftComment)>,
}

/// No filesystem reads: Uncommitted context must match the saved comment's snapshot.
pub fn export_review(review: &Review, contexts: &ReviewContexts) -> String {
    if review.comments.is_empty() {
        return String::new();
    }
    let mut groups: Vec<Group<'_>> = Vec::new();
    for (index, comment) in review.comments.iter().enumerate() {
        let context = contexts.get(&comment.id).map(Arc::as_ref);
        let path = comment.anchor.path();
        if let Some(group) = groups
            .iter_mut()
            .find(|g| g.path == path && g.context == context)
        {
            group.comments.push((index + 1, comment));
        } else {
            groups.push(Group {
                path,
                context,
                comments: vec![(index + 1, comment)],
            });
        }
    }
    let mut out = format!(
        "ReviewFox Export · {}\nRepository: {}\n",
        review.comparison.label(),
        review.comparison.repository.path().display()
    );
    for group in groups {
        out.push_str(&format!("\n---\nFile: {}\n", group.path));
        for (number, comment) in &group.comments {
            out.push_str(&format!("\nComment {number}\nTarget: "));
            match comment.anchor {
                Anchor::File { .. } => out.push_str("file\n"),
                Anchor::Line { side, span, .. } => {
                    out.push_str(&format!("{} L{}", side.label(), span.start));
                    if span.count > 1 {
                        out.push_str(&format!("-{}", span.start + span.count - 1));
                    }
                    out.push('\n');
                }
            }
            out.push_str("Body:\n");
            out.push_str(&comment.body);
            out.push('\n');
        }
        let line_comments: Vec<_> = group
            .comments
            .iter()
            .copied()
            .filter(|(_, c)| matches!(c.anchor, Anchor::Line { .. }))
            .collect();
        if line_comments.is_empty() {
            continue;
        }
        out.push_str("\nRelated change context (captured at comment creation):\n");
        let Some(context) = group.context else {
            out.push_str("Unavailable: no captured file text. Comment targets are preserved.\n");
            continue;
        };
        let (rows, blocks) = diff_rows(context);
        let mut excerpts: Vec<(Range<usize>, Vec<usize>)> = Vec::new();
        for (number, comment) in line_comments {
            if let Some(range) = excerpt_range(&rows, &blocks, &comment.anchor) {
                excerpts.push((range, vec![number]));
            } else {
                out.push_str(&format!(
                    "Unavailable for comment {number}: target is outside the captured file.\n"
                ));
            }
        }
        excerpts.sort_by_key(|(range, _)| range.start);
        let mut merged: Vec<(Range<usize>, Vec<usize>)> = Vec::new();
        for (range, numbers) in excerpts {
            if let Some((last, shared)) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
                shared.extend(numbers);
                continue;
            }
            merged.push((range, numbers));
        }
        for (range, mut numbers) in merged {
            numbers.sort_unstable();
            out.push_str(&format!(
                "\nFor comments: {}\n",
                numbers
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            write_excerpt(&mut out, &rows[range]);
        }
    }
    out
}

struct Row<'a> {
    mark: char,
    text: &'a str,
    old_before: usize,
    new_before: usize,
    no_newline: bool,
}

impl Row<'_> {
    fn line(&self, side: Side) -> Option<usize> {
        match side {
            Side::Preimage if self.mark != '+' => Some(self.old_before + 1),
            Side::Postimage if self.mark != '-' => Some(self.new_before + 1),
            _ => None,
        }
    }
}

fn diff_rows(context: &CommentContext) -> (Vec<Row<'_>>, Vec<Range<usize>>) {
    // Export reflects real text changes even when the viewer ignores whitespace.
    let alignment = git::compute_exact_alignment(&context.preimage, &context.postimage);
    let old: Vec<_> = context.preimage.split_inclusive('\n').collect();
    let new: Vec<_> = context.postimage.split_inclusive('\n').collect();
    let mut rows = Vec::new();
    let mut blocks = Vec::new();
    let (mut old_before, mut new_before) = (0, 0);
    for op in alignment.ops {
        let start = rows.len();
        let (old_count, new_count, equal) = match op {
            AlignmentOp::Equal { preimage, .. } => (preimage.count, preimage.count, true),
            AlignmentOp::Insert { postimages, .. } => (0, postimages.count, false),
            AlignmentOp::Delete { preimages, .. } => (preimages.count, 0, false),
            AlignmentOp::Replace {
                preimages,
                postimages,
            } => (preimages.count, postimages.count, false),
        };
        for _ in 0..old_count {
            rows.push(Row {
                mark: if equal { ' ' } else { '-' },
                text: old[old_before]
                    .strip_suffix('\n')
                    .unwrap_or(old[old_before]),
                old_before,
                new_before,
                no_newline: !old[old_before].ends_with('\n'),
            });
            old_before += 1;
            if equal {
                new_before += 1;
            }
        }
        if !equal {
            for _ in 0..new_count {
                rows.push(Row {
                    mark: '+',
                    text: new[new_before]
                        .strip_suffix('\n')
                        .unwrap_or(new[new_before]),
                    old_before,
                    new_before,
                    no_newline: !new[new_before].ends_with('\n'),
                });
                new_before += 1;
            }
            blocks.push(start..rows.len());
        }
    }
    (rows, blocks)
}

fn excerpt_range(
    rows: &[Row<'_>],
    blocks: &[Range<usize>],
    anchor: &Anchor,
) -> Option<Range<usize>> {
    let Anchor::Line { side, span, .. } = anchor else {
        return None;
    };
    let start_line = span.start as usize;
    let end_line = start_line + span.count.max(1) as usize - 1;
    // Never silently clamp an invalid target to unrelated code.
    let first = rows
        .iter()
        .position(|r| r.line(*side) == Some(start_line))?;
    let last = rows.iter().rposition(|r| r.line(*side) == Some(end_line))?;
    let mut range = first..last + 1;
    for block in blocks {
        if block.start < range.end && range.start < block.end {
            range.start = range.start.min(block.start);
            range.end = range.end.max(block.end);
        }
    }
    // Add unchanged context only; do not pull in unrelated nearby changes.
    for _ in 0..CONTEXT_RADIUS {
        if range.start > 0 && rows[range.start - 1].mark == ' ' {
            range.start -= 1;
        } else {
            break;
        }
    }
    for _ in 0..CONTEXT_RADIUS {
        if range.end < rows.len() && rows[range.end].mark == ' ' {
            range.end += 1;
        } else {
            break;
        }
    }
    Some(range)
}

fn write_excerpt(out: &mut String, rows: &[Row<'_>]) {
    let old_count = rows.iter().filter(|r| r.mark != '+').count();
    let new_count = rows.iter().filter(|r| r.mark != '-').count();
    let first = &rows[0];
    let old_start = first.old_before + usize::from(old_count > 0);
    let new_start = first.new_before + usize::from(new_count > 0);
    // Source text may itself contain Markdown fences.
    let longest = rows
        .iter()
        .flat_map(|r| r.text.split(|c| c != '`'))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(3.max(longest + 1));
    out.push_str(&format!(
        "{fence}diff\n@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
    ));
    for row in rows {
        out.push(row.mark);
        out.push_str(row.text);
        out.push('\n');
        if row.no_newline {
            out.push_str("\\ No newline at end of file\n");
        }
    }
    out.push_str(&format!("{fence}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Comparison, Oid, Repository};
    use std::path::PathBuf;

    fn review() -> Review {
        Review::new(Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Some(Oid::from_bytes([1; 20])),
            head_oid: Oid::from_bytes([2; 20]),
            uncommitted: false,
        })
    }
    fn contexts(review: &Review, old: &str, new: &str) -> ReviewContexts {
        let context = Arc::new(CommentContext {
            preimage: old.into(),
            postimage: new.into(),
        });
        review
            .comments
            .iter()
            .map(|c| (c.id, context.clone()))
            .collect()
    }
    #[test]
    fn empty_review_exports_empty() {
        assert!(export_review(&review(), &ReviewContexts::new()).is_empty());
    }
    #[test]
    fn opposite_sides_share_full_replace_but_keep_targets() {
        let mut review = review();
        review.add_line_span_comment("a.rs", Side::Preimage, 2, 1, "keep the check");
        review.add_line_span_comment("a.rs", Side::Postimage, 3, 1, "handle failure");
        let text = export_review(
            &review,
            &contexts(&review, "a\nold1\nold2\nz\n", "a\nnew1\nnew2\nnew3\nz\n"),
        );
        assert!(text.contains("Target: preimage L2"));
        assert!(text.contains("Target: postimage L3"));
        assert!(text.contains("For comments: 1, 2"));
        assert_eq!(text.matches("@@ -").count(), 1);
        assert!(text.contains("@@ -1,4 +1,5 @@"));
        assert!(text.contains("-old2\n+new1\n+new2\n+new3"));
        assert_eq!(text.matches("keep the check").count(), 1);
    }
    #[test]
    fn insert_and_delete_have_correct_empty_side_headers() {
        for (side, old, new, expected) in [
            (Side::Postimage, "", "added\n", "@@ -0,0 +1,1 @@\n+added"),
            (Side::Preimage, "removed\n", "", "@@ -1,1 +0,0 @@\n-removed"),
        ] {
            let mut review = review();
            review.add_line_span_comment("a", side, 1, 1, "body");
            let text = export_review(&review, &contexts(&review, old, new));
            assert!(text.contains(expected), "{text}");
        }
    }
    #[test]
    fn spanning_blocks_exports_both_and_comment_once() {
        let mut review = review();
        review.add_line_span_comment("a", Side::Postimage, 2, 4, "one comment");
        let text = export_review(
            &review,
            &contexts(&review, "a\nx\nb\nc\ny\nz\n", "a\nX\nb\nc\nY\nz\n"),
        );
        assert!(text.contains("-x\n+X"));
        assert!(text.contains("-y\n+Y"));
        assert_eq!(text.matches("one comment").count(), 1);
    }
    #[test]
    fn equal_only_comment_does_not_include_nearby_change() {
        let mut review = review();
        review.add_line_span_comment("a", Side::Postimage, 3, 1, "context only");
        let text = export_review(
            &review,
            &contexts(&review, "old\na\nb\nc\n", "new\na\nb\nc\n"),
        );
        assert!(text.contains("@@ -2,3 +2,3 @@\n a\n b\n c"));
        assert!(!text.contains("-old"));
        assert!(!text.contains("+new"));
    }
    #[test]
    fn whitespace_changes_are_exported() {
        let mut review = review();
        review.add_line_span_comment("a", Side::Postimage, 1, 1, "indent");
        let text = export_review(&review, &contexts(&review, " x\n", "    x\n"));
        assert!(text.contains("- x\n+    x"));
    }
    #[test]
    fn missing_or_invalid_context_preserves_comment() {
        let mut review = review();
        review.add_line_span_comment("a", Side::Postimage, 9, 1, "body");
        for contexts in [ReviewContexts::new(), contexts(&review, "a\n", "b\n")] {
            let text = export_review(&review, &contexts);
            assert!(text.contains("Unavailable"));
            assert!(text.contains("Target: postimage L9"));
            assert_eq!(text.matches("Body:\nbody").count(), 1);
            assert!(!text.contains("@@ -"));
        }
    }
    #[test]
    fn different_snapshots_of_one_file_are_not_merged() {
        let mut review = review();
        let first = review
            .add_line_span_comment("a", Side::Postimage, 1, 1, "first")
            .id;
        let second = review
            .add_line_span_comment("a", Side::Postimage, 1, 1, "second")
            .id;
        let mut contexts = ReviewContexts::new();
        contexts.insert(
            first,
            Arc::new(CommentContext {
                preimage: "old\n".into(),
                postimage: "first\n".into(),
            }),
        );
        contexts.insert(
            second,
            Arc::new(CommentContext {
                preimage: "old\n".into(),
                postimage: "second\n".into(),
            }),
        );
        let text = export_review(&review, &contexts);
        assert_eq!(text.matches("File: a\n").count(), 2);
        assert!(text.contains("+first"));
        assert!(text.contains("+second"));
        assert!(!text.contains("For comments: 1, 2"));
    }
    #[test]
    fn source_fences_cannot_close_diff_fence() {
        let mut review = review();
        review.add_line_span_comment("a.md", Side::Postimage, 1, 1, "body");
        let text = export_review(&review, &contexts(&review, "", "```rust\ncode\n```\n"));
        assert!(text.contains("````diff\n"));
    }

    #[test]
    fn newline_and_crlf_changes_are_not_hidden() {
        let mut review = review();
        review.add_line_span_comment("a", Side::Postimage, 1, 1, "body");
        let text = export_review(&review, &contexts(&review, "value", "value\n"));
        assert!(text.contains("-value\n\\ No newline at end of file\n+value\n"));
        let text = export_review(&review, &contexts(&review, "value\r\n", "value\n"));
        assert!(text.contains("-value\r\n+value\n"));
    }

    #[test]
    fn separate_blocks_have_correct_line_numbers_and_bounded_context() {
        let mut review = review();
        review.add_line_span_comment("a", Side::Postimage, 2, 1, "first");
        review.add_line_span_comment("a", Side::Postimage, 9, 1, "second");
        let text = export_review(
            &review,
            &contexts(
                &review,
                "a\nx\nb\nc\nd\ne\nf\ng\ny\nz\n",
                "a\nX\nb\nc\nd\ne\nf\ng\nY\nz\n",
            ),
        );
        assert!(text.contains("@@ -1,4 +1,4 @@\n a\n-x\n+X\n b\n c"));
        assert!(text.contains("@@ -7,4 +7,4 @@\n f\n g\n-y\n+Y\n z"));
        assert_eq!(text.matches("@@ -").count(), 2);
        assert!(!text.contains("\n d\n"));
    }

    #[test]
    fn file_comments_export_without_invented_line_context() {
        let mut review = review();
        review.comments.push(DraftComment {
            id: 1,
            body: "explain this file".into(),
            anchor: Anchor::File {
                path: "README.md".into(),
            },
        });
        let text = export_review(&review, &ReviewContexts::new());
        assert!(text.contains("File: README.md"));
        assert!(text.contains("Target: file\nBody:\nexplain this file"));
        assert!(!text.contains("Related change context"));
    }
}

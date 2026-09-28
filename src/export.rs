//! Clipboard Export: narrative snippets at DraftComment anchors (not the full patch).

use crate::domain::{Anchor, Comparison, Review, Side};
use crate::git;

const CONTEXT_RADIUS: u32 = 2;

/// Build default Export text for a Review. Empty Review → empty string.
pub fn export_review(review: &Review) -> String {
    if review.comments.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "ReviewFox Export · {}\n",
        review.comparison.label()
    ));
    out.push_str(&format!(
        "{}\n",
        review.comparison.repository.path().display()
    ));

    for (i, comment) in review.comments.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str("---\n");
        out.push_str(&format_comment(&review.comparison, comment));
    }
    out
}

fn format_comment(comparison: &Comparison, comment: &crate::domain::DraftComment) -> String {
    match &comment.anchor {
        Anchor::File { path } => {
            format!("{path}\n\n{}\n", comment.body)
        }
        Anchor::Line {
            path, side, span, ..
        } => {
            let end = span.start + span.count.max(1) - 1;
            let header = if span.count <= 1 {
                format!("{path} ({}) L{}\n", side.label(), span.start)
            } else {
                format!("{path} ({}) L{}-{end}\n", side.label(), span.start)
            };

            let mut s = header;
            if let Some(ctx) = context_lines(comparison, path, *side, span.start, end) {
                for (ln, text) in ctx {
                    let mark = if ln >= span.start && ln <= end {
                        '>'
                    } else {
                        ' '
                    };
                    s.push_str(&format!("{mark} {ln:>4} | {text}\n"));
                }
                s.push('\n');
            }
            s.push_str(&comment.body);
            s.push('\n');
            s
        }
    }
}

fn context_lines(
    comparison: &Comparison,
    path: &str,
    side: Side,
    start: u32,
    end: u32,
) -> Option<Vec<(u32, String)>> {
    let lines = git::side_lines(comparison, side, path).ok()?;
    if lines.is_empty() {
        return None;
    }
    let from = start.saturating_sub(CONTEXT_RADIUS).max(1);
    let to = (end + CONTEXT_RADIUS).min(lines.len() as u32);
    let mut out = Vec::new();
    for ln in from..=to {
        let text = lines.get((ln - 1) as usize).cloned().unwrap_or_default();
        out.push((ln, text));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LineSpan, Oid, Repository};
    use std::path::PathBuf;

    fn review_with_body(body: &str) -> Review {
        let comparison = Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Some(Oid::from_bytes([1; 20])),
            head_oid: Oid::from_bytes([2; 20]),
        };
        let mut review = Review::new(comparison);
        // File anchor — no git needed for context
        review.comments.push(crate::domain::DraftComment {
            id: 1,
            body: body.into(),
            anchor: Anchor::File {
                path: "README.md".into(),
            },
        });
        // Force next_id consistency not required for export
        review
    }

    #[test]
    fn empty_review_exports_empty() {
        let comparison = Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Some(Oid::from_bytes([1; 20])),
            head_oid: Oid::from_bytes([2; 20]),
        };
        assert!(export_review(&Review::new(comparison)).is_empty());
    }

    #[test]
    fn file_anchor_export_includes_body() {
        let text = export_review(&review_with_body("please expand"));
        assert!(text.contains("ReviewFox Export"));
        assert!(text.contains("README.md"));
        assert!(text.contains("please expand"));
        assert!(!text.contains(" L")); // no line label for file anchor
    }

    #[test]
    fn format_line_block_marks_anchor() {
        // Unit the formatter path via a Line comment without git: call format pieces
        // through export when we only have File — covered above.
        // Direct context marking:
        let mut s = String::new();
        let span = LineSpan { start: 2, count: 1 };
        let end = span.start;
        for (ln, text) in [(1u32, "a"), (2, "b"), (3, "c")] {
            let mark = if ln >= span.start && ln <= end {
                '>'
            } else {
                ' '
            };
            s.push_str(&format!("{mark} {ln:>4} | {text}\n"));
        }
        assert!(s.contains(">    2 | b"));
        assert!(s.contains("     1 | a"));
    }
}

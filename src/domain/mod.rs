//! Git-free review domain types. See CONTEXT.md.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Raw Git object id (SHA-1, 20 bytes).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Oid([u8; 20]);

impl Oid {
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// First 7 hex chars (display convenience, not identity).
    pub fn short(&self) -> String {
        let full = self.to_string();
        full[..7].to_string()
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({self})")
    }
}

impl FromStr for Oid {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.len() != 40 {
            return Err(format!("oid must be 40 hex chars, got {}", s.len()));
        }
        let mut bytes = [0u8; 20];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
            bytes[i] = u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
        }
        Ok(Self(bytes))
    }
}

/// Canonical absolute worktree path (symlinks resolved).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Repository {
    path: PathBuf,
}

impl Repository {
    pub fn new(canonical_path: PathBuf) -> Self {
        Self {
            path: canonical_path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// Reviewable surface: `(repository, base_oid, head_oid)` — commit OIDs only.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Comparison {
    pub repository: Repository,
    pub base_oid: Oid,
    pub head_oid: Oid,
}

impl Comparison {
    pub fn label(&self) -> String {
        format!("{}..{}", self.base_oid.short(), self.head_oid.short())
    }
}

/// Diff-computation knobs. Does not change Comparison identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewOptions {
    pub ignore_whitespace: bool,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStatus {
    Add,
    Delete,
    Modify,
}

impl PathStatus {
    pub fn letter(self) -> &'static str {
        match self {
            Self::Add => "A",
            Self::Delete => "D",
            Self::Modify => "M",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedPath {
    /// Repo-relative path using `/` separators.
    pub path: String,
    pub status: PathStatus,
    pub additions: u32,
    pub deletions: u32,
}

impl ChangedPath {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// 1-based line span within one side of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSpan {
    pub start: u32,
    pub count: u32,
}

/// How old and new lines correspond for one file under ViewOptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlignmentOp {
    Equal { old: LineSpan, new: LineSpan },
    Insert { after_old: u32, news: LineSpan },
    Delete { olds: LineSpan, at_new: u32 },
    /// Many-to-many replace; pairwise maps are optional refinement.
    Replace { olds: LineSpan, news: LineSpan },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Alignment {
    pub ops: Vec<AlignmentOp>,
}

/// Contiguous algorithm-produced change block (non-equal ops).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: LineSpan,
    pub new: LineSpan,
}

impl Alignment {
    pub fn hunks(&self) -> Vec<Hunk> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                AlignmentOp::Equal { .. } => None,
                AlignmentOp::Insert { after_old, news } => Some(Hunk {
                    old: LineSpan {
                        start: *after_old,
                        count: 0,
                    },
                    new: *news,
                }),
                AlignmentOp::Delete { olds, at_new } => Some(Hunk {
                    old: *olds,
                    new: LineSpan {
                        start: *at_new,
                        count: 0,
                    },
                }),
                AlignmentOp::Replace { olds, news } => Some(Hunk {
                    old: *olds,
                    new: *news,
                }),
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    Insert,
    Delete,
    Replace,
}

/// Dual-pane display row projected from Alignment + file text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRow {
    pub old_ln: Option<u32>,
    pub new_ln: Option<u32>,
    pub old_text: String,
    pub new_text: String,
    pub kind: RowKind,
}

/// Which side of a Comparison a line Anchor refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    New,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Self::Old => "old",
            Self::New => "new",
        }
    }
}

/// Attachment of a DraftComment to a place in a Comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // File anchors are in the domain model; UI starts with Line only.
pub enum Anchor {
    File {
        path: String,
    },
    Line {
        path: String,
        side: Side,
        span: LineSpan,
        /// Optional owning Hunk; omitted in v1 line clicks.
        hunk: Option<Hunk>,
    },
}

impl Anchor {
    pub fn path(&self) -> &str {
        match self {
            Self::File { path } | Self::Line { path, .. } => path,
        }
    }

    pub fn line_on(&self, side: Side, line: u32) -> bool {
        match self {
            Self::Line {
                side: a_side,
                span,
                ..
            } if *a_side == side => {
                line >= span.start && line < span.start + span.count.max(1)
            }
            _ => false,
        }
    }
}

/// Locally stored comment; not published remotely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftComment {
    pub id: u64,
    pub body: String,
    pub anchor: Anchor,
}

/// Ongoing work against one Comparison (in-memory for now).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub comparison: Comparison,
    pub comments: Vec<DraftComment>,
    next_id: u64,
}

impl Review {
    pub fn new(comparison: Comparison) -> Self {
        Self {
            comparison,
            comments: Vec::new(),
            next_id: 1,
        }
    }

    pub fn ensure_comparison(&mut self, comparison: Comparison) {
        if self.comparison != comparison {
            *self = Self::new(comparison);
        }
    }

    pub fn add_line_comment(
        &mut self,
        path: impl Into<String>,
        side: Side,
        line: u32,
        body: impl Into<String>,
    ) -> &DraftComment {
        let id = self.next_id;
        self.next_id += 1;
        self.comments.push(DraftComment {
            id,
            body: body.into(),
            anchor: Anchor::Line {
                path: path.into(),
                side,
                span: LineSpan {
                    start: line,
                    count: 1,
                },
                hunk: None,
            },
        });
        self.comments.last().unwrap()
    }

    pub fn comments_for_path<'a>(
        &'a self,
        path: &'a str,
    ) -> impl Iterator<Item = &'a DraftComment> + 'a {
        self.comments
            .iter()
            .filter(move |c| c.anchor.path() == path)
    }
}

pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.lines().collect()
    }
}

/// Project Alignment into dual-pane rows (replace is many-to-many padded).
pub fn display_rows(old_text: &str, new_text: &str, alignment: &Alignment) -> Vec<DisplayRow> {
    let old_lines = split_lines(old_text);
    let new_lines = split_lines(new_text);
    let mut rows = Vec::new();

    for op in &alignment.ops {
        match *op {
            AlignmentOp::Equal { old, new } => {
                for i in 0..old.count {
                    let oi = (old.start - 1 + i) as usize;
                    let ni = (new.start - 1 + i) as usize;
                    rows.push(DisplayRow {
                        old_ln: Some(old.start + i),
                        new_ln: Some(new.start + i),
                        old_text: old_lines.get(oi).unwrap_or(&"").to_string(),
                        new_text: new_lines.get(ni).unwrap_or(&"").to_string(),
                        kind: RowKind::Equal,
                    });
                }
            }
            AlignmentOp::Delete { olds, .. } => {
                for i in 0..olds.count {
                    let oi = (olds.start - 1 + i) as usize;
                    rows.push(DisplayRow {
                        old_ln: Some(olds.start + i),
                        new_ln: None,
                        old_text: old_lines.get(oi).unwrap_or(&"").to_string(),
                        new_text: String::new(),
                        kind: RowKind::Delete,
                    });
                }
            }
            AlignmentOp::Insert { news, .. } => {
                for i in 0..news.count {
                    let ni = (news.start - 1 + i) as usize;
                    rows.push(DisplayRow {
                        old_ln: None,
                        new_ln: Some(news.start + i),
                        old_text: String::new(),
                        new_text: new_lines.get(ni).unwrap_or(&"").to_string(),
                        kind: RowKind::Insert,
                    });
                }
            }
            AlignmentOp::Replace { olds, news } => {
                let n = olds.count.max(news.count);
                for i in 0..n {
                    let old_ln = (i < olds.count).then_some(olds.start + i);
                    let new_ln = (i < news.count).then_some(news.start + i);
                    let old_text = old_ln
                        .map(|ln| old_lines.get((ln - 1) as usize).unwrap_or(&"").to_string())
                        .unwrap_or_default();
                    let new_text = new_ln
                        .map(|ln| new_lines.get((ln - 1) as usize).unwrap_or(&"").to_string())
                        .unwrap_or_default();
                    rows.push(DisplayRow {
                        old_ln,
                        new_ln,
                        old_text,
                        new_text,
                        kind: RowKind::Replace,
                    });
                }
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_comparison() -> Comparison {
        Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Oid::from_bytes([1; 20]),
            head_oid: Oid::from_bytes([2; 20]),
        }
    }

    #[test]
    fn review_adds_line_comment() {
        let mut review = Review::new(fake_comparison());
        review.add_line_comment("src/a.rs", Side::New, 12, "nits");
        let c = review.comments_for_path("src/a.rs").next().unwrap();
        assert_eq!(c.body, "nits");
        assert!(matches!(
            &c.anchor,
            Anchor::Line {
                side: Side::New,
                span: LineSpan {
                    start: 12,
                    count: 1
                },
                ..
            }
        ));
    }

    #[test]
    fn review_resets_on_comparison_change() {
        let mut review = Review::new(fake_comparison());
        review.add_line_comment("a.rs", Side::Old, 1, "x");
        let mut other = fake_comparison();
        other.head_oid = Oid::from_bytes([3; 20]);
        review.ensure_comparison(other.clone());
        assert!(review.comments.is_empty());
        assert_eq!(review.comparison, other);
    }
}

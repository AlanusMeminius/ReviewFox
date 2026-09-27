//! Git-free review domain types. See CONTEXT.md.

use std::collections::HashSet;
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

/// First visual line of a Hunk for jump landing (§3.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkJumpTarget {
    /// Side that has the Hunk's first visual line.
    pub side: Side,
    pub ln: u32,
}

/// Land point plus scroll parameter `s` (row units) at that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkLand {
    pub target: HunkJumpTarget,
    pub s: u32,
}

/// Land point for hunk `index` from [`Alignment::hunks`]: Insert → first new
/// line; Delete → first old line; Replace → first old line if any, else first new.
pub fn hunk_jump_target(alignment: &Alignment, index: usize) -> Option<HunkJumpTarget> {
    let hunk = alignment.hunks().into_iter().nth(index)?;
    if hunk.old.count > 0 {
        Some(HunkJumpTarget {
            side: Side::Old,
            ln: hunk.old.start,
        })
    } else if hunk.new.count > 0 {
        Some(HunkJumpTarget {
            side: Side::New,
            ln: hunk.new.start,
        })
    } else {
        None
    }
}

/// Equal lines kept at each end of a contiguous run beside a Hunk (§3.3).
pub const EQUAL_CONTEXT: u32 = 3;

/// Per-file fold: which Equal op indices are expanded. Default = all collapsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldState {
    pub expanded: HashSet<usize>,
}

impl FoldState {
    pub fn collapsed() -> Self {
        Self::default()
    }

    pub fn expand(&mut self, omit_id: usize) {
        self.expanded.insert(omit_id);
    }
}

/// Which side(s) in-file search inspects (§3.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    Old,
    New,
    Both,
}

/// One hit from [`search_file`]: side + 1-based line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub side: Side,
    pub ln: u32,
}

/// Case-insensitive substring search over old and/or new text.
/// Empty / whitespace-only query yields no matches.
pub fn search_file(
    old_text: &str,
    new_text: &str,
    query: &str,
    scope: SearchScope,
) -> Vec<SearchMatch> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let take = |side: Side, text: &str, out: &mut Vec<SearchMatch>| {
        for (i, line) in split_lines(text).into_iter().enumerate() {
            if line.to_lowercase().contains(&q) {
                out.push(SearchMatch {
                    side,
                    ln: (i + 1) as u32,
                });
            }
        }
    };
    match scope {
        SearchScope::Old => take(Side::Old, old_text, &mut out),
        SearchScope::New => take(Side::New, new_text, &mut out),
        SearchScope::Both => {
            take(Side::Old, old_text, &mut out);
            take(Side::New, new_text, &mut out);
        }
    }
    out
}

/// Expand-then-land plan for jumping to a search match (§3.5).
/// If the line lies in a collapsed Equal span, `expand` is that op index;
/// after expanding both sides, land on `target` using post-expansion rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchJumpPlan {
    pub expand: Option<usize>,
    pub target: HunkJumpTarget,
}

/// Plan a search-match jump: expand the collapsed Equal that hides `ln` (if any),
/// then land on that line — same landing rule as a Hunk jump.
pub fn match_jump_plan(
    alignment: &Alignment,
    fold: &FoldState,
    side: Side,
    ln: u32,
) -> MatchJumpPlan {
    MatchJumpPlan {
        expand: collapsed_equal_containing(alignment, fold, side, ln),
        target: HunkJumpTarget { side, ln },
    }
}

/// Op index of a collapsed Equal whose omitted middle contains `ln` on `side`.
fn collapsed_equal_containing(
    alignment: &Alignment,
    fold: &FoldState,
    side: Side,
    ln: u32,
) -> Option<usize> {
    let has_hunk = alignment
        .ops
        .iter()
        .any(|op| !matches!(op, AlignmentOp::Equal { .. }));
    if !has_hunk {
        return None;
    }
    for (op_idx, op) in alignment.ops.iter().enumerate() {
        let AlignmentOp::Equal { old, new } = *op else {
            continue;
        };
        let n = old.count.min(new.count);
        if n <= EQUAL_CONTEXT * 2 || fold.expanded.contains(&op_idx) {
            continue;
        }
        let start = match side {
            Side::Old => old.start,
            Side::New => new.start,
        };
        let from = start + EQUAL_CONTEXT;
        let to = start + n - EQUAL_CONTEXT - 1;
        if ln >= from && ln <= to {
            return Some(op_idx);
        }
    }
    None
}

/// Session-level mono font size for dual-pane Diff (§3.5). One value drives
/// both panes and the ribbons/gutter metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffFontSize {
    px: u32,
}

impl DiffFontSize {
    pub const DEFAULT: u32 = 13;
    pub const MIN: u32 = 10;
    pub const MAX: u32 = 22;

    pub fn px(self) -> u32 {
        self.px
    }

    pub fn increase(&mut self) {
        self.px = (self.px + 1).min(Self::MAX);
    }

    pub fn decrease(&mut self) {
        self.px = self.px.saturating_sub(1).max(Self::MIN);
    }

    pub fn reset(&mut self) {
        self.px = Self::DEFAULT;
    }

    /// Row height matching the prototype: `round(fontSize * 22 / 13)`.
    pub fn row_height(self) -> f32 {
        (self.px as f32 * 22.0 / 13.0).round()
    }
}

impl Default for DiffFontSize {
    fn default() -> Self {
        Self {
            px: Self::DEFAULT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    Insert,
    Delete,
    Replace,
    /// Collapsed Equal span; `DisplayRow.ln` is `from`.
    Omit { id: usize, from: u32, to: u32 },
}

/// One visual line on one side. Not a shared old+new row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRow {
    pub ln: u32,
    pub text: String,
    pub kind: RowKind,
}

/// Center-gutter bridge for one Alignment op. Row offsets are content rows
/// (end exclusive). A seam has no rows on that side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bridge {
    Insert {
        after_old: u32,
        news: LineSpan,
        old_seam: u32,
        new_from: u32,
        new_to: u32,
    },
    Delete {
        olds: LineSpan,
        at_new: u32,
        old_from: u32,
        old_to: u32,
        new_seam: u32,
    },
    Replace {
        olds: LineSpan,
        news: LineSpan,
        old_from: u32,
        old_to: u32,
        new_from: u32,
        new_to: u32,
    },
}

impl Bridge {
    /// Short position copy for hover/status. Same facts as the op, not a second model.
    pub fn position_copy(&self) -> String {
        match *self {
            Self::Insert { after_old, news, .. } => {
                let after = if after_old == 0 {
                    "file start".to_string()
                } else {
                    format!("old {after_old}")
                };
                format!("Insert new {} after {after}", span_label(news))
            }
            Self::Delete { olds, at_new, .. } => {
                format!("Delete old {} at new {at_new}", span_label(olds))
            }
            Self::Replace { olds, news, .. } => {
                format!("Replace old {} ↔ new {}", span_label(olds), span_label(news))
            }
        }
    }
}

fn span_label(span: LineSpan) -> String {
    if span.count <= 1 {
        format!("{}", span.start)
    } else {
        format!(
            "{}\u{2013}{}",
            span.start,
            span.start + span.count.saturating_sub(1)
        )
    }
}

/// Shared scroll parameter knots, in row units. `s` advances once per visual
/// step; a side that does not gain a row stays put on that step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollKnot {
    pub s: u32,
    pub old_y: u32,
    pub new_y: u32,
}

/// Per-side rows plus the connector layout those rows imply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRows {
    pub old_rows: Vec<DisplayRow>,
    pub new_rows: Vec<DisplayRow>,
    pub bridges: Vec<Bridge>,
    pub knots: Vec<ScrollKnot>,
    /// One entry per Hunk, in Alignment order; `s` is the scroll parameter at
    /// the Hunk's first visual line.
    pub hunk_lands: Vec<HunkLand>,
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

/// One token (or whitespace / punctuation run) inside a Replace line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenPart {
    pub text: String,
    pub changed: bool,
}

/// Intra-line marks for a Replace block. Same-line (1↔1) uses LCS token
/// pairing; many-to-many uses set membership and does not invent row links.
pub fn replace_marks(olds: &[&str], news: &[&str]) -> (Vec<Vec<TokenPart>>, Vec<Vec<TokenPart>>) {
    if olds.len() == 1 && news.len() == 1 {
        let (o, n) = pair_marks(olds[0], news[0]);
        (vec![o], vec![n])
    } else {
        (block_marks(olds, news), block_marks(news, olds))
    }
}

fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();
    while start < bytes.len() {
        let rest = &s[start..];
        let end = if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len())
        } else if rest.starts_with(char::is_whitespace) {
            rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len())
        } else {
            rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1)
        };
        out.push(&s[start..start + end]);
        start += end;
    }
    if out.is_empty() {
        out.push(s);
    }
    out
}

fn pair_marks(old_text: &str, new_text: &str) -> (Vec<TokenPart>, Vec<TokenPart>) {
    let a = tokenize(old_text);
    let b = tokenize(new_text);
    let ca = lcs_changed(&a, &b);
    let cb = lcs_changed(&b, &a);
    let old = a
        .into_iter()
        .zip(ca)
        .map(|(t, chg)| TokenPart {
            text: t.to_string(),
            changed: chg && !t.trim().is_empty(),
        })
        .collect();
    let neu = b
        .into_iter()
        .zip(cb)
        .map(|(t, chg)| TokenPart {
            text: t.to_string(),
            changed: chg && !t.trim().is_empty(),
        })
        .collect();
    (old, neu)
}

fn block_marks(lines: &[&str], other_lines: &[&str]) -> Vec<Vec<TokenPart>> {
    let other: std::collections::HashSet<&str> = other_lines
        .iter()
        .flat_map(|line| tokenize(line))
        .filter(|t| !t.trim().is_empty())
        .collect();
    lines
        .iter()
        .map(|line| {
            tokenize(line)
                .into_iter()
                .map(|t| TokenPart {
                    text: t.to_string(),
                    changed: !t.trim().is_empty() && !other.contains(t),
                })
                .collect()
        })
        .collect()
}

/// Which tokens in `a` are not in an LCS with `b` (prototype `lcsChanged`).
fn lcs_changed(a: &[&str], b: &[&str]) -> Vec<bool> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut changed = vec![true; n];
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if a[i] == b[j] {
            changed[i] = false;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    changed
}

fn line_text(lines: &[&str], ln: u32) -> String {
    lines
        .get(ln.saturating_sub(1) as usize)
        .unwrap_or(&"")
        .to_string()
}

fn push_side(rows: &mut Vec<DisplayRow>, lines: &[&str], span: LineSpan, kind: RowKind) {
    for i in 0..span.count {
        let ln = span.start + i;
        rows.push(DisplayRow {
            ln,
            text: line_text(lines, ln),
            kind,
        });
    }
}

fn advance(knots: &mut Vec<ScrollKnot>, old_y: &mut u32, new_y: &mut u32, d_old: u32, d_new: u32) {
    if d_old == 0 && d_new == 0 {
        return;
    }
    *old_y += d_old;
    *new_y += d_new;
    let s = knots.last().map(|k| k.s).unwrap_or(0) + d_old.max(d_new);
    knots.push(ScrollKnot {
        s,
        old_y: *old_y,
        new_y: *new_y,
    });
}

/// Project Alignment into per-side rows and one bridge per change op.
///
/// Equal lines are one row on each side. Insert rows exist only on the new
/// side, delete rows only on the old side. A Replace is one block: the old
/// lines stacked, the new lines stacked, with no invented partner row on the
/// shorter side. Does not fold Equal runs (fully expanded).
pub fn display_rows(old_text: &str, new_text: &str, alignment: &Alignment) -> DisplayRows {
    project_rows(old_text, new_text, alignment, None)
}

/// Like [`display_rows`], but collapses Equal runs longer than
/// `2 * EQUAL_CONTEXT` beside a Hunk unless their op index is in `fold.expanded`.
pub fn display_rows_folded(
    old_text: &str,
    new_text: &str,
    alignment: &Alignment,
    fold: &FoldState,
) -> DisplayRows {
    project_rows(old_text, new_text, alignment, Some(fold))
}

fn project_rows(
    old_text: &str,
    new_text: &str,
    alignment: &Alignment,
    fold: Option<&FoldState>,
) -> DisplayRows {
    let old_lines = split_lines(old_text);
    let new_lines = split_lines(new_text);
    let mut old_rows = Vec::new();
    let mut new_rows = Vec::new();
    let mut bridges = Vec::new();
    let mut knots = vec![ScrollKnot {
        s: 0,
        old_y: 0,
        new_y: 0,
    }];
    let mut old_y = 0u32;
    let mut new_y = 0u32;
    let mut hunk_lands = Vec::new();
    let has_hunk = alignment.ops.iter().any(|op| !matches!(op, AlignmentOp::Equal { .. }));

    for (op_idx, op) in alignment.ops.iter().enumerate() {
        match *op {
            AlignmentOp::Equal { old, new } => {
                let n = old.count.min(new.count);
                let collapse = fold.is_some_and(|f| {
                    has_hunk && n > EQUAL_CONTEXT * 2 && !f.expanded.contains(&op_idx)
                });
                if collapse {
                    let head = EQUAL_CONTEXT;
                    let tail = EQUAL_CONTEXT;
                    push_side(
                        &mut old_rows,
                        &old_lines,
                        LineSpan {
                            start: old.start,
                            count: head,
                        },
                        RowKind::Equal,
                    );
                    push_side(
                        &mut new_rows,
                        &new_lines,
                        LineSpan {
                            start: new.start,
                            count: head,
                        },
                        RowKind::Equal,
                    );
                    advance(&mut knots, &mut old_y, &mut new_y, head, head);

                    let from_old = old.start + EQUAL_CONTEXT;
                    let to_old = old.start + n - EQUAL_CONTEXT - 1;
                    let from_new = new.start + EQUAL_CONTEXT;
                    let to_new = new.start + n - EQUAL_CONTEXT - 1;
                    old_rows.push(omit_row(op_idx, from_old, to_old));
                    new_rows.push(omit_row(op_idx, from_new, to_new));
                    advance(&mut knots, &mut old_y, &mut new_y, 1, 1);

                    push_side(
                        &mut old_rows,
                        &old_lines,
                        LineSpan {
                            start: old.start + n - tail,
                            count: tail,
                        },
                        RowKind::Equal,
                    );
                    push_side(
                        &mut new_rows,
                        &new_lines,
                        LineSpan {
                            start: new.start + n - tail,
                            count: tail,
                        },
                        RowKind::Equal,
                    );
                    advance(&mut knots, &mut old_y, &mut new_y, tail, tail);
                } else {
                    push_side(
                        &mut old_rows,
                        &old_lines,
                        LineSpan {
                            start: old.start,
                            count: n,
                        },
                        RowKind::Equal,
                    );
                    push_side(
                        &mut new_rows,
                        &new_lines,
                        LineSpan {
                            start: new.start,
                            count: n,
                        },
                        RowKind::Equal,
                    );
                    advance(&mut knots, &mut old_y, &mut new_y, n, n);
                }
            }
            AlignmentOp::Insert { after_old, news } => {
                let s = knots.last().map(|k| k.s).unwrap_or(0);
                if let Some(target) = land_from_op(op) {
                    hunk_lands.push(HunkLand { target, s });
                }
                let old_seam = old_y;
                let new_from = new_y;
                push_side(&mut new_rows, &new_lines, news, RowKind::Insert);
                advance(&mut knots, &mut old_y, &mut new_y, 0, news.count);
                bridges.push(Bridge::Insert {
                    after_old,
                    news,
                    old_seam,
                    new_from,
                    new_to: new_y,
                });
            }
            AlignmentOp::Delete { olds, at_new } => {
                let s = knots.last().map(|k| k.s).unwrap_or(0);
                if let Some(target) = land_from_op(op) {
                    hunk_lands.push(HunkLand { target, s });
                }
                let old_from = old_y;
                let new_seam = new_y;
                push_side(&mut old_rows, &old_lines, olds, RowKind::Delete);
                advance(&mut knots, &mut old_y, &mut new_y, olds.count, 0);
                bridges.push(Bridge::Delete {
                    olds,
                    at_new,
                    old_from,
                    old_to: old_y,
                    new_seam,
                });
            }
            AlignmentOp::Replace { olds, news } => {
                let s = knots.last().map(|k| k.s).unwrap_or(0);
                if let Some(target) = land_from_op(op) {
                    hunk_lands.push(HunkLand { target, s });
                }
                let old_from = old_y;
                let new_from = new_y;
                push_side(&mut old_rows, &old_lines, olds, RowKind::Replace);
                push_side(&mut new_rows, &new_lines, news, RowKind::Replace);
                let common = olds.count.min(news.count);
                advance(&mut knots, &mut old_y, &mut new_y, common, common);
                if olds.count > common {
                    advance(
                        &mut knots,
                        &mut old_y,
                        &mut new_y,
                        olds.count - common,
                        0,
                    );
                }
                if news.count > common {
                    advance(
                        &mut knots,
                        &mut old_y,
                        &mut new_y,
                        0,
                        news.count - common,
                    );
                }
                bridges.push(Bridge::Replace {
                    olds,
                    news,
                    old_from,
                    old_to: old_y,
                    new_from,
                    new_to: new_y,
                });
            }
        }
    }

    DisplayRows {
        old_rows,
        new_rows,
        bridges,
        knots,
        hunk_lands,
    }
}

fn land_from_op(op: &AlignmentOp) -> Option<HunkJumpTarget> {
    match *op {
        AlignmentOp::Equal { .. } => None,
        AlignmentOp::Insert { news, .. } if news.count > 0 => Some(HunkJumpTarget {
            side: Side::New,
            ln: news.start,
        }),
        AlignmentOp::Delete { olds, .. } if olds.count > 0 => Some(HunkJumpTarget {
            side: Side::Old,
            ln: olds.start,
        }),
        AlignmentOp::Replace { olds, news } => {
            if olds.count > 0 {
                Some(HunkJumpTarget {
                    side: Side::Old,
                    ln: olds.start,
                })
            } else if news.count > 0 {
                Some(HunkJumpTarget {
                    side: Side::New,
                    ln: news.start,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn omit_row(id: usize, from: u32, to: u32) -> DisplayRow {
    DisplayRow {
        ln: from,
        text: format!("⋯ {from}\u{2013}{to}"),
        kind: RowKind::Omit { id, from, to },
    }
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

    #[test]
    fn insert_at_file_start_puts_new_lines_only_on_the_new_side() {
        let display = display_rows(
            "alpha\nbeta\n",
            "HEAD\nNECK\nalpha\nbeta\n",
            &Alignment {
                ops: vec![
                    AlignmentOp::Insert {
                        after_old: 0,
                        news: LineSpan { start: 1, count: 2 },
                    },
                    AlignmentOp::Equal {
                        old: LineSpan { start: 1, count: 2 },
                        new: LineSpan { start: 3, count: 2 },
                    },
                ],
            },
        );
        assert_eq!(
            display.old_rows,
            vec![
                DisplayRow {
                    ln: 1,
                    text: "alpha".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 2,
                    text: "beta".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.new_rows,
            vec![
                DisplayRow {
                    ln: 1,
                    text: "HEAD".into(),
                    kind: RowKind::Insert,
                },
                DisplayRow {
                    ln: 2,
                    text: "NECK".into(),
                    kind: RowKind::Insert,
                },
                DisplayRow {
                    ln: 3,
                    text: "alpha".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 4,
                    text: "beta".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.bridges,
            vec![Bridge::Insert {
                after_old: 0,
                news: LineSpan { start: 1, count: 2 },
                old_seam: 0,
                new_from: 0,
                new_to: 2,
            }]
        );
        assert_eq!(
            display.bridges[0].position_copy(),
            "Insert new 1\u{2013}2 after file start"
        );
    }

    #[test]
    fn insert_at_file_end_anchors_after_the_last_old_line() {
        let old: String = (1..=15).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        let new = format!("{old}\nN16\nN17\nN18");
        let display = display_rows(
            &old,
            &new,
            &Alignment {
                ops: vec![
                    AlignmentOp::Equal {
                        old: LineSpan { start: 1, count: 15 },
                        new: LineSpan { start: 1, count: 15 },
                    },
                    AlignmentOp::Insert {
                        after_old: 15,
                        news: LineSpan { start: 16, count: 3 },
                    },
                ],
            },
        );
        assert_eq!(display.old_rows.len(), 15);
        assert!(display.old_rows.iter().all(|row| row.kind == RowKind::Equal));
        assert_eq!(display.old_rows[0].ln, 1);
        assert_eq!(display.old_rows[14].ln, 15);
        assert_eq!(
            &display.new_rows[15..],
            &[
                DisplayRow {
                    ln: 16,
                    text: "N16".into(),
                    kind: RowKind::Insert,
                },
                DisplayRow {
                    ln: 17,
                    text: "N17".into(),
                    kind: RowKind::Insert,
                },
                DisplayRow {
                    ln: 18,
                    text: "N18".into(),
                    kind: RowKind::Insert,
                },
            ]
        );
        assert_eq!(
            display.bridges,
            vec![Bridge::Insert {
                after_old: 15,
                news: LineSpan { start: 16, count: 3 },
                old_seam: 15,
                new_from: 15,
                new_to: 18,
            }]
        );
        assert_eq!(
            display.bridges[0].position_copy(),
            "Insert new 16\u{2013}18 after old 15"
        );
    }

    #[test]
    fn delete_rows_appear_only_on_the_old_side() {
        let display = display_rows(
            "keep\ngone-a\ngone-b\nkeep2\n",
            "keep\nkeep2\n",
            &Alignment {
                ops: vec![
                    AlignmentOp::Equal {
                        old: LineSpan { start: 1, count: 1 },
                        new: LineSpan { start: 1, count: 1 },
                    },
                    AlignmentOp::Delete {
                        olds: LineSpan { start: 2, count: 2 },
                        at_new: 2,
                    },
                    AlignmentOp::Equal {
                        old: LineSpan { start: 4, count: 1 },
                        new: LineSpan { start: 2, count: 1 },
                    },
                ],
            },
        );
        assert_eq!(
            display.old_rows,
            vec![
                DisplayRow {
                    ln: 1,
                    text: "keep".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 2,
                    text: "gone-a".into(),
                    kind: RowKind::Delete,
                },
                DisplayRow {
                    ln: 3,
                    text: "gone-b".into(),
                    kind: RowKind::Delete,
                },
                DisplayRow {
                    ln: 4,
                    text: "keep2".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.new_rows,
            vec![
                DisplayRow {
                    ln: 1,
                    text: "keep".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 2,
                    text: "keep2".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.bridges,
            vec![Bridge::Delete {
                olds: LineSpan { start: 2, count: 2 },
                at_new: 2,
                old_from: 1,
                old_to: 3,
                new_seam: 1,
            }]
        );
        assert_eq!(
            display.bridges[0].position_copy(),
            "Delete old 2\u{2013}3 at new 2"
        );
    }

    #[test]
    fn replace_three_old_lines_with_one_new_line_has_no_partner_rows() {
        let display = display_rows(
            "old-a\nold-b\nold-c\n",
            "new-a\n",
            &Alignment {
                ops: vec![AlignmentOp::Replace {
                    olds: LineSpan { start: 1, count: 3 },
                    news: LineSpan { start: 1, count: 1 },
                }],
            },
        );
        assert_eq!(
            display.old_rows,
            vec![
                DisplayRow {
                    ln: 1,
                    text: "old-a".into(),
                    kind: RowKind::Replace,
                },
                DisplayRow {
                    ln: 2,
                    text: "old-b".into(),
                    kind: RowKind::Replace,
                },
                DisplayRow {
                    ln: 3,
                    text: "old-c".into(),
                    kind: RowKind::Replace,
                },
            ]
        );
        assert_eq!(
            display.new_rows,
            vec![DisplayRow {
                ln: 1,
                text: "new-a".into(),
                kind: RowKind::Replace,
            }]
        );
        assert_eq!(
            display.bridges,
            vec![Bridge::Replace {
                olds: LineSpan { start: 1, count: 3 },
                news: LineSpan { start: 1, count: 1 },
                old_from: 0,
                old_to: 3,
                new_from: 0,
                new_to: 1,
            }]
        );
        assert_eq!(
            display.bridges[0].position_copy(),
            "Replace old 1\u{2013}3 ↔ new 1"
        );
    }

    #[test]
    fn equal_lines_are_one_row_on_each_side_with_paired_line_numbers() {
        let display = display_rows(
            "w\nx\ny\nsame\nstill\n",
            "a\nb\nc\nd\ne\nf\ng\nh\nsame\nstill\n",
            &Alignment {
                ops: vec![AlignmentOp::Equal {
                    old: LineSpan { start: 4, count: 2 },
                    new: LineSpan { start: 9, count: 2 },
                }],
            },
        );
        assert_eq!(
            display.old_rows,
            vec![
                DisplayRow {
                    ln: 4,
                    text: "same".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 5,
                    text: "still".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.new_rows,
            vec![
                DisplayRow {
                    ln: 9,
                    text: "same".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 10,
                    text: "still".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert!(display.bridges.is_empty());
    }

    /// Long Equal run beside a Hunk: keep 3 lines at each end, one omit separator per side.
    #[test]
    fn long_equal_run_beside_hunk_collapses_to_one_separator_per_side() {
        // 10 Equal lines, then a one-line Insert. Context is 3, so middle 4–7 collapse.
        let old: String = (1..=10).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        let new = format!("{old}\nINS");
        let alignment = Alignment {
            ops: vec![
                AlignmentOp::Equal {
                    old: LineSpan { start: 1, count: 10 },
                    new: LineSpan { start: 1, count: 10 },
                },
                AlignmentOp::Insert {
                    after_old: 10,
                    news: LineSpan { start: 11, count: 1 },
                },
            ],
        };
        let display = display_rows_folded(&old, &new, &alignment, &FoldState::collapsed());

        assert_eq!(
            display.old_rows,
            vec![
                DisplayRow {
                    ln: 1,
                    text: "L1".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 2,
                    text: "L2".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 3,
                    text: "L3".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 4,
                    text: "⋯ 4\u{2013}7".into(),
                    kind: RowKind::Omit {
                        id: 0,
                        from: 4,
                        to: 7,
                    },
                },
                DisplayRow {
                    ln: 8,
                    text: "L8".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 9,
                    text: "L9".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 10,
                    text: "L10".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.new_rows[..7],
            [
                DisplayRow {
                    ln: 1,
                    text: "L1".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 2,
                    text: "L2".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 3,
                    text: "L3".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 4,
                    text: "⋯ 4\u{2013}7".into(),
                    kind: RowKind::Omit {
                        id: 0,
                        from: 4,
                        to: 7,
                    },
                },
                DisplayRow {
                    ln: 8,
                    text: "L8".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 9,
                    text: "L9".into(),
                    kind: RowKind::Equal,
                },
                DisplayRow {
                    ln: 10,
                    text: "L10".into(),
                    kind: RowKind::Equal,
                },
            ]
        );
        assert_eq!(
            display.new_rows[7],
            DisplayRow {
                ln: 11,
                text: "INS".into(),
                kind: RowKind::Insert,
            }
        );
        // One separator each side, same omit id.
        let old_omits: Vec<_> = display
            .old_rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Omit { .. }))
            .collect();
        let new_omits: Vec<_> = display
            .new_rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Omit { .. }))
            .collect();
        assert_eq!(old_omits.len(), 1);
        assert_eq!(new_omits.len(), 1);
        assert_eq!(old_omits[0].kind, new_omits[0].kind);
    }

    #[test]
    fn equal_run_that_fits_context_stays_fully_visible() {
        // 6 Equal lines = exactly 2×CONTEXT; must not collapse.
        let old: String = (1..=6).map(|i| format!("E{i}")).collect::<Vec<_>>().join("\n");
        let new = format!("{old}\nX");
        let alignment = Alignment {
            ops: vec![
                AlignmentOp::Equal {
                    old: LineSpan { start: 1, count: 6 },
                    new: LineSpan { start: 1, count: 6 },
                },
                AlignmentOp::Insert {
                    after_old: 6,
                    news: LineSpan { start: 7, count: 1 },
                },
            ],
        };
        let display = display_rows_folded(&old, &new, &alignment, &FoldState::collapsed());
        assert!(
            display
                .old_rows
                .iter()
                .all(|r| !matches!(r.kind, RowKind::Omit { .. }))
        );
        assert!(
            display
                .new_rows
                .iter()
                .all(|r| !matches!(r.kind, RowKind::Omit { .. }))
        );
        assert_eq!(display.old_rows.len(), 6);
        assert_eq!(display.new_rows.len(), 7);
        assert_eq!(display.old_rows[0].ln, 1);
        assert_eq!(display.old_rows[5].ln, 6);
        assert_eq!(display.old_rows[5].text, "E6");
    }

    #[test]
    fn no_hunk_file_shows_every_line_and_no_separator() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj";
        let new = old;
        let alignment = Alignment {
            ops: vec![AlignmentOp::Equal {
                old: LineSpan { start: 1, count: 10 },
                new: LineSpan { start: 1, count: 10 },
            }],
        };
        let display = display_rows_folded(old, new, &alignment, &FoldState::collapsed());
        assert_eq!(display.old_rows.len(), 10);
        assert_eq!(display.new_rows.len(), 10);
        assert!(
            display
                .old_rows
                .iter()
                .chain(display.new_rows.iter())
                .all(|r| !matches!(r.kind, RowKind::Omit { .. }))
        );
        assert_eq!(display.old_rows[0].text, "a");
        assert_eq!(display.old_rows[9].text, "j");
        assert!(display.bridges.is_empty());
    }

    #[test]
    fn expanding_one_separator_by_id_expands_that_span_on_both_sides() {
        let old: String = (1..=10).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        let new = format!("{old}\nINS");
        let alignment = Alignment {
            ops: vec![
                AlignmentOp::Equal {
                    old: LineSpan { start: 1, count: 10 },
                    new: LineSpan { start: 1, count: 10 },
                },
                AlignmentOp::Insert {
                    after_old: 10,
                    news: LineSpan { start: 11, count: 1 },
                },
            ],
        };
        let mut fold = FoldState::collapsed();
        let collapsed = display_rows_folded(&old, &new, &alignment, &fold);
        let omit_id = match collapsed.old_rows[3].kind {
            RowKind::Omit { id, .. } => id,
            other => panic!("expected omit at index 3, got {other:?}"),
        };
        assert!(matches!(collapsed.new_rows[3].kind, RowKind::Omit { id, .. } if id == omit_id));

        fold.expand(omit_id);
        let expanded = display_rows_folded(&old, &new, &alignment, &fold);

        assert!(
            expanded
                .old_rows
                .iter()
                .chain(expanded.new_rows.iter())
                .all(|r| !matches!(r.kind, RowKind::Omit { .. }))
        );
        assert_eq!(expanded.old_rows.len(), 10);
        assert_eq!(expanded.new_rows.len(), 11);
        assert_eq!(
            expanded.old_rows.iter().map(|r| r.ln).collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(
            expanded.new_rows[..10]
                .iter()
                .map(|r| r.ln)
                .collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(expanded.new_rows[10].ln, 11);
        assert_eq!(expanded.new_rows[10].kind, RowKind::Insert);
        // Middle lines that were omitted are restored on both sides.
        assert_eq!(expanded.old_rows[3].text, "L4");
        assert_eq!(expanded.new_rows[3].text, "L4");
        assert_eq!(expanded.old_rows[6].text, "L7");
        assert_eq!(expanded.new_rows[6].text, "L7");
    }

    #[test]
    fn hunk_jump_lands_on_first_visual_line_of_insert() {
        let alignment = Alignment {
            ops: vec![
                AlignmentOp::Equal {
                    old: LineSpan { start: 1, count: 2 },
                    new: LineSpan { start: 1, count: 2 },
                },
                AlignmentOp::Insert {
                    after_old: 2,
                    news: LineSpan { start: 3, count: 2 },
                },
            ],
        };
        let land = hunk_jump_target(&alignment, 0).expect("insert hunk");
        assert_eq!(
            land,
            HunkJumpTarget {
                side: Side::New,
                ln: 3,
            }
        );
    }

    #[test]
    fn hunk_jump_lands_on_first_visual_line_of_delete() {
        let alignment = Alignment {
            ops: vec![
                AlignmentOp::Equal {
                    old: LineSpan { start: 1, count: 1 },
                    new: LineSpan { start: 1, count: 1 },
                },
                AlignmentOp::Delete {
                    olds: LineSpan { start: 2, count: 2 },
                    at_new: 2,
                },
            ],
        };
        let land = hunk_jump_target(&alignment, 0).expect("delete hunk");
        assert_eq!(
            land,
            HunkJumpTarget {
                side: Side::Old,
                ln: 2,
            }
        );
    }

    #[test]
    fn same_line_replace_marks_differing_tokens() {
        let (old_marks, new_marks) =
            replace_marks(&["int timeoutMs = 10;"], &["int timeoutMs = 40;"]);
        assert_eq!(old_marks.len(), 1);
        assert_eq!(new_marks.len(), 1);
        let old_changed: Vec<&str> = old_marks[0]
            .iter()
            .filter(|p| p.changed)
            .map(|p| p.text.as_str())
            .collect();
        let new_changed: Vec<&str> = new_marks[0]
            .iter()
            .filter(|p| p.changed)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(old_changed, vec!["10"]);
        assert_eq!(new_changed, vec!["40"]);
        // Unchanged tokens stay unmarked.
        assert!(
            old_marks[0]
                .iter()
                .any(|p| p.text == "timeoutMs" && !p.changed)
        );
        assert!(
            new_marks[0]
                .iter()
                .any(|p| p.text == "timeoutMs" && !p.changed)
        );
    }

    #[test]
    fn many_to_many_replace_marks_keep_block_first_rows() {
        let olds = ["old-a", "old-b", "old-c"];
        let news = ["new-a"];
        let display = display_rows(
            "old-a\nold-b\nold-c\n",
            "new-a\n",
            &Alignment {
                ops: vec![AlignmentOp::Replace {
                    olds: LineSpan { start: 1, count: 3 },
                    news: LineSpan { start: 1, count: 1 },
                }],
            },
        );
        assert_eq!(display.old_rows.len(), 3);
        assert_eq!(display.new_rows.len(), 1);
        assert_eq!(display.bridges.len(), 1);
        assert!(matches!(display.bridges[0], Bridge::Replace { .. }));

        let (old_marks, new_marks) = replace_marks(&olds, &news);
        assert_eq!(old_marks.len(), 3, "one mark row per old line — no padding");
        assert_eq!(new_marks.len(), 1, "one mark row per new line — no partner invented");
        // Block marks flag tokens absent from the other side; shared "-" / "a" stay unmarked.
        let old_changed: Vec<&str> = old_marks
            .iter()
            .flat_map(|line| line.iter())
            .filter(|p| p.changed)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(old_changed, vec!["old", "old", "b", "old", "c"]);
        assert_eq!(
            new_marks[0]
                .iter()
                .filter(|p| p.changed)
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            vec!["new"]
        );
    }

    #[test]
    fn search_file_lists_matches_with_side_and_line_by_scope() {
        let old = "alpha\nneedle here\nomega\n";
        let new = "alpha\nbeta\nother NEEDLE\n";

        assert_eq!(
            search_file(old, new, "needle", SearchScope::Old),
            vec![SearchMatch {
                side: Side::Old,
                ln: 2,
            }]
        );
        assert_eq!(
            search_file(old, new, "needle", SearchScope::New),
            vec![SearchMatch {
                side: Side::New,
                ln: 3,
            }]
        );
        assert_eq!(
            search_file(old, new, "needle", SearchScope::Both),
            vec![
                SearchMatch {
                    side: Side::Old,
                    ln: 2,
                },
                SearchMatch {
                    side: Side::New,
                    ln: 3,
                },
            ]
        );
        assert!(search_file(old, new, "  ", SearchScope::Both).is_empty());
    }

    #[test]
    fn match_jump_in_collapsed_equal_expands_then_lands_on_line() {
        // 10 Equal lines + Insert. Lines 4–7 are omitted when collapsed.
        let old: String = (1..=10).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        let new = format!("{old}\nINS");
        let alignment = Alignment {
            ops: vec![
                AlignmentOp::Equal {
                    old: LineSpan { start: 1, count: 10 },
                    new: LineSpan { start: 1, count: 10 },
                },
                AlignmentOp::Insert {
                    after_old: 10,
                    news: LineSpan { start: 11, count: 1 },
                },
            ],
        };
        let mut fold = FoldState::collapsed();
        let collapsed = display_rows_folded(&old, &new, &alignment, &fold);
        assert!(
            collapsed
                .old_rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::Omit { .. })),
            "precondition: Equal run is collapsed"
        );
        assert!(
            !collapsed
                .old_rows
                .iter()
                .any(|r| r.ln == 5 && !matches!(r.kind, RowKind::Omit { .. })),
            "line 5 is not visible while collapsed"
        );

        let plan = match_jump_plan(&alignment, &fold, Side::Old, 5);
        assert_eq!(
            plan.target,
            HunkJumpTarget {
                side: Side::Old,
                ln: 5
            }
        );
        assert_eq!(plan.expand, Some(0), "must expand the Equal op at index 0");

        if let Some(id) = plan.expand {
            fold.expand(id);
        }
        let expanded = display_rows_folded(&old, &new, &alignment, &fold);
        let land_row = expanded
            .old_rows
            .iter()
            .position(|r| r.ln == 5 && !matches!(r.kind, RowKind::Omit { .. }))
            .expect("line 5 visible after expand");
        // Post-expansion: line 5 is the 5th Equal row (index 4), scroll gap at that row.
        assert_eq!(land_row, 4);
    }

    #[test]
    fn diff_font_size_steps_and_resets_within_shared_bounds() {
        let mut size = DiffFontSize::default();
        assert_eq!(size.px(), DiffFontSize::DEFAULT);

        size.increase();
        assert_eq!(size.px(), DiffFontSize::DEFAULT + 1);

        for _ in 0..40 {
            size.increase();
        }
        assert_eq!(size.px(), DiffFontSize::MAX);

        for _ in 0..40 {
            size.decrease();
        }
        assert_eq!(size.px(), DiffFontSize::MIN);

        size.reset();
        assert_eq!(size.px(), DiffFontSize::DEFAULT);
    }
}

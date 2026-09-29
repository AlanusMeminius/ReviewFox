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
    /// Base commit; `None` = the empty tree (base of a root commit).
    pub base_oid: Option<Oid>,
    pub head_oid: Oid,
}

impl Comparison {
    /// Label shown for an empty-tree base.
    pub const EMPTY_BASE_LABEL: &'static str = "root";

    /// `e7a2ab7..287bfa5`, or `root..ae12de0` for an empty-tree base.
    pub fn label(&self) -> String {
        let base = self
            .base_oid
            .map(|o| o.short())
            .unwrap_or_else(|| Self::EMPTY_BASE_LABEL.to_string());
        format!("{base}..{}", self.head_oid.short())
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

/// Which side(s) Diff find inspects (§3.5). Orthogonal to [`SearchFiles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSide {
    Old,
    New,
    Both,
}

/// Whether Diff find covers the current file only or every changed path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchFiles {
    File,
    All,
}

/// One occurrence hit: side, 1-based line, byte range on that line.
/// `path` is set when the hit comes from [`search_files`] (All-files).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub side: Side,
    pub ln: u32,
    pub bytes: std::ops::Range<usize>,
    pub path: Option<String>,
}

/// Byte offset of the first case-insensitive match of `query` in `line`, if any.
pub fn first_match_byte(line: &str, query: &str) -> Option<usize> {
    match_byte_ranges(line, query).first().map(|r| r.start)
}

/// Every non-overlapping case-insensitive match of `query` in `line` as byte ranges
/// on the original UTF-8 (char-boundary aligned).
pub fn match_byte_ranges(line: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        if let Some(r) = ci_match_range(line, i, q) {
            out.push(r.clone());
            i = r.end;
        } else {
            i = next_char_index(line, i);
        }
    }
    out
}

fn next_char_index(line: &str, byte: usize) -> usize {
    if byte >= line.len() {
        return line.len();
    }
    let char_start = if line.is_char_boundary(byte) {
        byte
    } else {
        line[..byte]
            .char_indices()
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(byte)
    };
    line[char_start..]
        .chars()
        .next()
        .map(|c| char_start + c.len_utf8())
        .unwrap_or(line.len())
}

fn char_eq_ci(a: char, b: char) -> bool {
    a.to_lowercase().eq(b.to_lowercase())
}

fn ci_match_range(line: &str, start_byte: usize, needle: &str) -> Option<std::ops::Range<usize>> {
    let tail = line.get(start_byte..)?;
    let mut hay = tail.chars();
    let mut end = start_byte;
    for n in needle.chars() {
        let c = hay.next()?;
        if !char_eq_ci(c, n) {
            return None;
        }
        end += c.len_utf8();
    }
    Some(start_byte..end)
}

/// Next wrap index into a match list of `len`. No current selection → first (0).
pub fn next_match_index(len: usize, current: Option<usize>) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(current.map(|i| (i + 1) % len).unwrap_or(0))
    }
}

/// Previous wrap index into a match list of `len`. No current selection → last.
pub fn prev_match_index(len: usize, current: Option<usize>) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(
            current
                .map(|i| if i == 0 { len - 1 } else { i - 1 })
                .unwrap_or(len - 1),
        )
    }
}

/// One path's old/new text for [`search_files`] (tree order).
#[derive(Clone, Copy, Debug)]
pub struct SearchFileText<'a> {
    pub path: &'a str,
    pub old_text: &'a str,
    pub new_text: &'a str,
}

/// Case-insensitive occurrence search over one file's old and/or new text.
/// Empty / whitespace-only query yields no matches. Order: Side (Old before
/// New when Both) → line number → byte start.
pub fn search_file(
    old_text: &str,
    new_text: &str,
    query: &str,
    side: SearchSide,
) -> Vec<SearchMatch> {
    search_file_inner(old_text, new_text, query, side, None)
}

/// Occurrence search over many files in the given order. Each hit carries
/// `path`. Within a file, same order as [`search_file`].
pub fn search_files(
    files: &[SearchFileText<'_>],
    query: &str,
    side: SearchSide,
) -> Vec<SearchMatch> {
    let mut out = Vec::new();
    for f in files {
        out.extend(search_file_inner(
            f.old_text,
            f.new_text,
            query,
            side,
            Some(f.path.to_string()),
        ));
    }
    out
}

fn search_file_inner(
    old_text: &str,
    new_text: &str,
    query: &str,
    side: SearchSide,
    path: Option<String>,
) -> Vec<SearchMatch> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let take = |s: Side, text: &str, out: &mut Vec<SearchMatch>| {
        for (i, line) in split_lines(text).into_iter().enumerate() {
            for bytes in match_byte_ranges(line, query) {
                out.push(SearchMatch {
                    side: s,
                    ln: (i + 1) as u32,
                    bytes,
                    path: path.clone(),
                });
            }
        }
    };
    match side {
        SearchSide::Old => take(Side::Old, old_text, &mut out),
        SearchSide::New => take(Side::New, new_text, &mut out),
        SearchSide::Both => {
            take(Side::Old, old_text, &mut out);
            take(Side::New, new_text, &mut out);
        }
    }
    out
}

/// Next Side in Tab cycle: Old → New → Both → Old.
pub fn next_search_side(side: SearchSide) -> SearchSide {
    match side {
        SearchSide::Old => SearchSide::New,
        SearchSide::New => SearchSide::Both,
        SearchSide::Both => SearchSide::Old,
    }
}

/// Toggle Files factor: File ↔ All.
pub fn toggle_search_files(files: SearchFiles) -> SearchFiles {
    match files {
        SearchFiles::File => SearchFiles::All,
        SearchFiles::All => SearchFiles::File,
    }
}

/// Expand-then-land plan for jumping to a search match (§3.5).
/// If the line lies in a collapsed Equal span, `expand` is that op index;
/// after expanding both sides, land on `target`. The Diff viewport chooses
/// scroll fraction (search → center; hunk → §3.1 one-third anchor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchJumpPlan {
    pub expand: Option<usize>,
    pub target: HunkJumpTarget,
}

/// Plan a search-match jump: expand the collapsed Equal that hides `ln` (if any),
/// then land on that line. Scroll fraction is chosen by the Diff viewport.
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

/// Mono font size for dual-pane Diff (§3.5). One value drives both panes and
/// the ribbons/gutter metrics. `base` is the Code Font size setting; A−/A+ move
/// `px` for the session only and `reset` (A) returns to `base`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffFontSize {
    px: u32,
    base: u32,
}

impl DiffFontSize {
    pub const DEFAULT: u32 = 13;
    pub const MIN: u32 = 10;
    pub const MAX: u32 = 22;

    /// Starts at `base`, clamped to `MIN..=MAX`.
    pub fn new(base: u32) -> Self {
        let base = base.clamp(Self::MIN, Self::MAX);
        Self { px: base, base }
    }

    pub fn px(self) -> u32 {
        self.px
    }

    pub fn base(self) -> u32 {
        self.base
    }

    pub fn increase(&mut self) {
        self.px = (self.px + 1).min(Self::MAX);
    }

    pub fn decrease(&mut self) {
        self.px = self.px.saturating_sub(1).max(Self::MIN);
    }

    pub fn reset(&mut self) {
        self.px = self.base;
    }

    /// Row height matching the prototype: `round(fontSize * 22 / 13)`.
    pub fn row_height(self) -> f32 {
        (self.px as f32 * 22.0 / 13.0).round()
    }
}

impl Default for DiffFontSize {
    fn default() -> Self {
        Self::new(Self::DEFAULT)
    }
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

    /// Side and 1-based lines (end exclusive) a line Anchor covers; a zero
    /// count still covers its start line. `None` for a file Anchor.
    pub fn lines(&self) -> Option<(Side, std::ops::Range<u32>)> {
        match self {
            Self::Line { side, span, .. } => {
                Some((*side, span.start..span.start + span.count.max(1)))
            }
            Self::File { .. } => None,
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

/// Byte ranges of the highlighted runs in one line's marks. Adjacent changed
/// tokens merge, and so do changed tokens separated only by whitespace; an
/// unchanged non-whitespace token breaks the run. Runs never start or end on
/// whitespace. `parts` concatenate to the line text.
pub fn changed_runs(parts: &[TokenPart]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    // Whether the last run may still grow (only whitespace since its end).
    let mut open = false;
    let mut at = 0usize;
    for part in parts {
        let end = at + part.text.len();
        if part.text.is_empty() {
            continue;
        }
        if part.changed {
            match runs.last_mut() {
                Some(last) if open => last.1 = end,
                _ => runs.push((at, end)),
            }
            open = true;
        } else if !part.text.trim().is_empty() {
            open = false;
        }
        at = end;
    }
    runs
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_comparison() -> Comparison {
        Comparison {
            repository: Repository::new(PathBuf::from("/tmp/repo")),
            base_oid: Some(Oid::from_bytes([1; 20])),
            head_oid: Oid::from_bytes([2; 20]),
        }
    }

    #[test]
    fn comparison_label_renders_empty_base_as_root() {
        let mut c = fake_comparison();
        assert_eq!(c.label(), "0101010..0202020");
        c.base_oid = None;
        assert_eq!(c.label(), "root..0202020");
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

    fn parts(spec: &[(&str, bool)]) -> Vec<TokenPart> {
        spec.iter()
            .map(|&(t, c)| TokenPart {
                text: t.to_string(),
                changed: c,
            })
            .collect()
    }

    #[test]
    fn changed_runs_bridge_whitespace_between_changed_words() {
        // "old content 10" vs "completely different 10".
        let (old, new) = replace_marks(&["old content 10"], &["completely different 10"]);
        assert_eq!(changed_runs(&old[0]), vec![(0, 11)]);
        assert_eq!(changed_runs(&new[0]), vec![(0, 20)]);
    }

    #[test]
    fn changed_runs_break_on_unchanged_token_and_skip_edge_whitespace() {
        let p = parts(&[
            ("  ", false),
            ("a", true),
            (" ", false),
            ("b", true),
            (" ", false),
            ("keep", false),
            (" ", false),
            ("c", true),
            ("d", true),
            ("  ", false),
        ]);
        assert_eq!(changed_runs(&p), vec![(2, 5), (11, 13)]);
    }

    #[test]
    fn changed_runs_cover_an_all_changed_line_once() {
        let p = parts(&[("x", true), (" ", false), ("=", true), (" ", false), ("1", true)]);
        assert_eq!(changed_runs(&p), vec![(0, 5)]);
        assert!(changed_runs(&parts(&[("same", false)])).is_empty());
    }

    #[test]
    fn changed_runs_use_byte_offsets_for_cjk() {
        // Many-to-many block: "行" appears on the other side and breaks the run.
        let olds = ["第21行：代码评审 レビュー length"];
        let news = ["新插入的中文行一", "新插入的中文行二"];
        let (old, _) = replace_marks(&olds, &news);
        let line = olds[0];
        let runs = changed_runs(&old[0]);
        let texts: Vec<&str> = runs.iter().map(|&(a, b)| &line[a..b]).collect();
        assert_eq!(texts, vec!["第21", "：代码评审 レビュー length"]);
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
    fn match_byte_ranges_cjk_and_multi_hit() {
        let line = "中文中文x";
        let hits = match_byte_ranges(line, "中文");
        assert_eq!(hits.len(), 2);
        assert_eq!(&line[hits[0].clone()], "中文");
        assert_eq!(&line[hits[1].clone()], "中文");
        assert_eq!(first_match_byte("中x", "中"), Some(0));
    }

    #[test]
    fn match_byte_ranges_turkish_i_and_mixed() {
        // ẞ (3 UTF-8 bytes) case-folds to ß (2 bytes); indices must stay on the original line.
        let line = "ẞxα";
        assert_eq!(first_match_byte(line, "ß"), Some(0));
        assert_eq!(&line[match_byte_ranges(line, "ß")[0].clone()], "ẞ");
        let mixed = "a中文needle中文";
        let m = match_byte_ranges(mixed, "needle");
        assert_eq!(m.len(), 1);
        assert_eq!(&mixed[m[0].clone()], "needle");
    }

    #[test]
    fn search_file_lists_occurrence_matches_by_side() {
        let old = "alpha\nneedle here needle\nomega\n";
        let new = "alpha\nbeta\nother NEEDLE\n";

        assert_eq!(
            search_file(old, new, "needle", SearchSide::Old),
            vec![
                SearchMatch {
                    side: Side::Old,
                    ln: 2,
                    bytes: 0..6,
                    path: None,
                },
                SearchMatch {
                    side: Side::Old,
                    ln: 2,
                    bytes: 12..18,
                    path: None,
                },
            ]
        );
        assert_eq!(
            search_file(old, new, "needle", SearchSide::New),
            vec![SearchMatch {
                side: Side::New,
                ln: 3,
                bytes: 6..12,
                path: None,
            }]
        );
        assert_eq!(
            search_file(old, new, "needle", SearchSide::Both),
            vec![
                SearchMatch {
                    side: Side::Old,
                    ln: 2,
                    bytes: 0..6,
                    path: None,
                },
                SearchMatch {
                    side: Side::Old,
                    ln: 2,
                    bytes: 12..18,
                    path: None,
                },
                SearchMatch {
                    side: Side::New,
                    ln: 3,
                    bytes: 6..12,
                    path: None,
                },
            ]
        );
        assert!(search_file(old, new, "  ", SearchSide::Both).is_empty());
    }

    #[test]
    fn search_file_both_orders_old_before_new_then_line_then_byte() {
        // New has an earlier line hit; Both still lists all Old before all New.
        let old = "zzz\nx needle\n";
        let new = "needle top\nother\n";
        let hits = search_file(old, new, "needle", SearchSide::Both);
        assert_eq!(
            hits
                .iter()
                .map(|m| (m.side, m.ln, m.bytes.start))
                .collect::<Vec<_>>(),
            vec![
                (Side::Old, 2, 2),
                (Side::New, 1, 0),
            ]
        );
    }

    #[test]
    fn search_files_walks_paths_in_given_order() {
        let files = [
            SearchFileText {
                path: "b.rs",
                old_text: "nope\n",
                new_text: "needle in b\n",
            },
            SearchFileText {
                path: "a.rs",
                old_text: "needle in a\n",
                new_text: "\n",
            },
        ];
        let hits = search_files(&files, "needle", SearchSide::Both);
        assert_eq!(
            hits
                .iter()
                .map(|m| (m.path.as_deref(), m.side, m.ln))
                .collect::<Vec<_>>(),
            vec![
                (Some("b.rs"), Side::New, 1),
                (Some("a.rs"), Side::Old, 1),
            ]
        );
    }

    #[test]
    fn search_side_and_files_cycle_helpers() {
        assert_eq!(next_search_side(SearchSide::Old), SearchSide::New);
        assert_eq!(next_search_side(SearchSide::New), SearchSide::Both);
        assert_eq!(next_search_side(SearchSide::Both), SearchSide::Old);
        assert_eq!(toggle_search_files(SearchFiles::File), SearchFiles::All);
        assert_eq!(toggle_search_files(SearchFiles::All), SearchFiles::File);
    }

    #[test]
    fn match_index_nav_wraps_and_handles_empty() {
        assert_eq!(next_match_index(0, None), None);
        assert_eq!(prev_match_index(0, Some(0)), None);

        assert_eq!(next_match_index(3, None), Some(0));
        assert_eq!(next_match_index(3, Some(0)), Some(1));
        assert_eq!(next_match_index(3, Some(2)), Some(0));

        assert_eq!(prev_match_index(3, None), Some(2));
        assert_eq!(prev_match_index(3, Some(0)), Some(2));
        assert_eq!(prev_match_index(3, Some(1)), Some(0));
    }

    #[test]
    fn match_jump_in_collapsed_equal_expands_that_span() {
        // 10 Equal lines + Insert. Lines 4–7 are omitted when collapsed.
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

        let plan = match_jump_plan(&alignment, &fold, Side::Old, 5);
        assert_eq!(
            plan.target,
            HunkJumpTarget {
                side: Side::Old,
                ln: 5
            }
        );
        assert_eq!(plan.expand, Some(0), "must expand the Equal op at index 0");

        // Once expanded, nothing hides the line. Landing row: ui::diff::layout tests.
        fold.expand(0);
        assert_eq!(match_jump_plan(&alignment, &fold, Side::Old, 5).expand, None);
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

    #[test]
    fn diff_font_size_resets_to_its_base_and_clamps_it() {
        let mut size = DiffFontSize::new(16);
        assert_eq!(size.px(), 16);
        size.increase();
        size.increase();
        assert_eq!(size.px(), 18);
        size.reset();
        assert_eq!(size.px(), 16);

        assert_eq!(DiffFontSize::new(40).px(), DiffFontSize::MAX);
        assert_eq!(DiffFontSize::new(2).px(), DiffFontSize::MIN);
    }
}

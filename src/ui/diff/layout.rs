//! Per-file dual-pane Layout: the view projection of an Alignment under a
//! FoldState. Pure (no GPUI). Rebuilt on Alignment / fold / ViewOptions /
//! soft-wrap width or font change; never on the per-frame path. See
//! docs/diffview-architecture.md §3.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use super::tabs::{TabExpansion, display_columns};
use super::visual_wrap::WrapCtx;
pub use super::visual_wrap::{AppliedWrap, WrapPlan};

use crate::domain::{
    Alignment, AlignmentOp, Anchor, EQUAL_CONTEXT, FoldState, HunkJumpTarget, LineSpan, Side,
    TokenPart, changed_runs, replace_marks,
};

/// What a line row shows. Omission separators are not a kind; see [`OmitRow`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Equal,
    Insert,
    Delete,
    Replace,
}

/// Which slice of a logical line a visual row shows (§3.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinePart {
    #[default]
    First,
    Continuation,
    /// Blank row beside a longer Equal partner; no line number, no hatch.
    EqualPad,
}

/// One visual line on one side. Not a shared preimage+postimage row. Text is a byte
/// range into that side's shared text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRow {
    pub ln: u32,
    pub kind: LineKind,
    /// Visual row index on this side (omission separators count as rows).
    pub row: u32,
    pub part: LinePart,
    bytes: Range<usize>,
    /// Index into [`Layout::bridges`] of the Replace block holding this line.
    block: Option<u32>,
}

impl LineRow {
    /// Byte range of this line in the side's shared text (no trailing `\n`/`\r`).
    pub fn bytes(&self) -> Range<usize> {
        self.bytes.clone()
    }

    /// Line number column: first visual row of a logical line only (§3.1.1).
    pub fn shows_line_number(&self) -> bool {
        self.part == LinePart::First
    }

    /// Equal padding beside a longer wrapped partner: blank, no hatch (§3.1.1).
    pub fn is_equal_padding(&self) -> bool {
        self.part == LinePart::EqualPad
    }

    /// Which wrapped segment of the logical line this row shows (0 = first).
    pub fn segment_index(&self, side: &SideLayout) -> usize {
        side.lines
            .iter()
            .filter(|l| l.ln == self.ln && l.row <= self.row && l.part != LinePart::EqualPad)
            .count()
            .saturating_sub(1)
    }
}

/// One side's view of one collapsed Equal span (§3.3). Takes a visual row but
/// is not a line; the preimage and postimage separators of a span share `id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OmitRow {
    /// Equal op index in the Alignment; expand with [`FoldState::expand`].
    pub id: usize,
    pub from: u32,
    pub to: u32,
    pub row: u32,
}

/// What occupies one visual row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row<'a> {
    Line(&'a LineRow),
    Omit(&'a OmitRow),
}

/// Center-gutter bridge for one Alignment op. Row offsets are visual rows
/// (end exclusive). A seam has no rows on that side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bridge {
    Insert {
        after_preimage: u32,
        postimages: LineSpan,
        preimage_seam: u32,
        postimage_from: u32,
        postimage_to: u32,
    },
    Delete {
        preimages: LineSpan,
        at_postimage: u32,
        preimage_from: u32,
        preimage_to: u32,
        postimage_seam: u32,
    },
    Replace {
        preimages: LineSpan,
        postimages: LineSpan,
        preimage_from: u32,
        preimage_to: u32,
        postimage_from: u32,
        postimage_to: u32,
    },
}

impl Bridge {
    /// Short position copy for hover/status. Same facts as the op, not a second model.
    pub fn position_copy(&self) -> String {
        match *self {
            Self::Insert {
                after_preimage, postimages, ..
            } => {
                let after = if after_preimage == 0 {
                    "file start".to_string()
                } else {
                    format!("preimage {after_preimage}")
                };
                format!("Insert postimage {} after {after}", span_label(postimages))
            }
            Self::Delete { preimages, at_postimage, .. } => {
                format!(
                    "Delete preimage {} at postimage {at_postimage}",
                    span_label(preimages)
                )
            }
            Self::Replace { preimages, postimages, .. } => {
                format!(
                    "Replace preimage {} ↔ postimage {}",
                    span_label(preimages),
                    span_label(postimages)
                )
            }
        }
    }

    pub fn kind(&self) -> LineKind {
        match self {
            Self::Insert { .. } => LineKind::Insert,
            Self::Delete { .. } => LineKind::Delete,
            Self::Replace { .. } => LineKind::Replace,
        }
    }

    /// Visual rows the bridge touches on `side`: `(from, to)`, end exclusive.
    /// A seam is `(seam, seam)`.
    pub fn rows(&self, side: Side) -> (u32, u32) {
        match (*self, side) {
            (Self::Insert { preimage_seam, .. }, Side::Preimage) => (preimage_seam, preimage_seam),
            (
                Self::Insert {
                    postimage_from, postimage_to, ..
                },
                Side::Postimage,
            ) => (postimage_from, postimage_to),
            (
                Self::Delete {
                    preimage_from, preimage_to, ..
                },
                Side::Preimage,
            ) => (preimage_from, preimage_to),
            (Self::Delete { postimage_seam, .. }, Side::Postimage) => (postimage_seam, postimage_seam),
            (
                Self::Replace {
                    preimage_from, preimage_to, ..
                },
                Side::Preimage,
            ) => (preimage_from, preimage_to),
            (
                Self::Replace {
                    postimage_from, postimage_to, ..
                },
                Side::Postimage,
            ) => (postimage_from, postimage_to),
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
    pub preimage_y: u32,
    pub postimage_y: u32,
}

impl ScrollKnot {
    pub fn y(&self, side: Side) -> u32 {
        match side {
            Side::Preimage => self.preimage_y,
            Side::Postimage => self.postimage_y,
        }
    }
}

/// Land point plus scroll parameter `s` (row units) at that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkLand {
    pub target: HunkJumpTarget,
    pub s: u32,
}

/// One side's visual rows: line rows and omission separators kept apart,
/// plus the seam and comment indices the renderer looks up per row.
#[derive(Debug)]
pub struct SideLayout {
    text: Arc<str>,
    lines: Vec<LineRow>,
    /// Sorted by `row`.
    omits: Vec<OmitRow>,
    /// Rows whose top edge is a pinch (Insert on preimage, Delete on postimage); sorted.
    seams: Vec<u32>,
    /// Line numbers with a DraftComment on this side.
    commented: HashSet<u32>,
    /// Longest shown line in display columns (tabs expanded), computed on first use (bounds `x_offset`).
    max_chars: OnceCell<usize>,
}

impl SideLayout {
    fn new(text: Arc<str>) -> Self {
        Self {
            text,
            lines: Vec::new(),
            omits: Vec::new(),
            seams: Vec::new(),
            commented: HashSet::new(),
            max_chars: OnceCell::new(),
        }
    }

    /// Visual row count (lines + separators).
    pub fn rows(&self) -> usize {
        self.lines.len() + self.omits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows() == 0
    }

    #[cfg(test)]
    pub fn lines(&self) -> &[LineRow] {
        &self.lines
    }

    pub fn omits(&self) -> &[OmitRow] {
        &self.omits
    }

    pub fn text(&self, line: &LineRow) -> &str {
        &self.text[line.bytes.clone()]
    }

    /// Full logical line text (first visual row's bytes).
    pub fn line_text(&self, ln: u32) -> Option<&str> {
        let line = self
            .lines
            .iter()
            .find(|l| l.ln == ln && l.part == LinePart::First)?;
        Some(self.text(line))
    }

    /// Display columns of the longest shown line (folded-away lines
    /// excluded; tabs expanded to their stops, see `tabs`).
    /// One pass over the side's text the first time it is asked; with a mono
    /// advance it estimates the side's widest line without shaping it.
    pub fn max_chars(&self) -> usize {
        *self.max_chars.get_or_init(|| {
            self.lines
                .iter()
                .map(|l| display_columns(self.text(l)))
                .max()
                .unwrap_or(0)
        })
    }

    /// What occupies visual row `i`.
    pub fn row(&self, i: usize) -> Option<Row<'_>> {
        if i >= self.rows() {
            return None;
        }
        let before = self.omits.partition_point(|o| (o.row as usize) < i);
        match self.omits.get(before) {
            Some(o) if o.row as usize == i => Some(Row::Omit(o)),
            _ => self.lines.get(i - before).map(Row::Line),
        }
    }

    /// Rows in order, O(1) per step.
    #[cfg(test)]
    pub fn iter_rows(&self) -> impl Iterator<Item = Row<'_>> + '_ {
        let mut lines = self.lines.iter().peekable();
        let mut omits = self.omits.iter().peekable();
        std::iter::from_fn(move || match (lines.peek(), omits.peek()) {
            (Some(l), Some(o)) if o.row < l.row => omits.next().map(Row::Omit),
            (Some(_), _) => lines.next().map(Row::Line),
            (None, Some(_)) => omits.next().map(Row::Omit),
            (None, None) => None,
        })
    }

    /// Visual row of the first row of logical line `ln` (§3.5 hunk / match land).
    pub fn row_of_line(&self, ln: u32) -> Option<u32> {
        self.lines
            .iter()
            .find(|l| l.ln == ln && l.part == LinePart::First)
            .map(|l| l.row)
    }

    pub(crate) fn push_visual_line(
        &mut self,
        ln: u32,
        kind: LineKind,
        part: LinePart,
        bytes: Range<usize>,
        block: Option<u32>,
    ) {
        let row = self.rows() as u32;
        self.lines.push(LineRow {
            ln,
            kind,
            row,
            part,
            bytes,
            block,
        });
    }

    /// Last line row above visual row `row`.
    pub fn line_above(&self, row: usize) -> Option<&LineRow> {
        let i = self.lines.partition_point(|l| (l.row as usize) < row);
        i.checked_sub(1).map(|i| &self.lines[i])
    }

    pub fn seams(&self) -> &[u32] {
        &self.seams
    }

    #[cfg(test)]
    pub fn is_seam(&self, row: usize) -> bool {
        self.seams.binary_search(&(row as u32)).is_ok()
    }

    pub fn has_comment(&self, ln: u32) -> bool {
        self.commented.contains(&ln)
    }

    fn push_lines(
        &mut self,
        ranges: &[Range<usize>],
        span: LineSpan,
        kind: LineKind,
        block: Option<u32>,
    ) {
        for i in 0..span.count {
            let ln = span.start + i;
            let row = self.rows() as u32;
            let bytes = ranges
                .get(ln.saturating_sub(1) as usize)
                .cloned()
                .unwrap_or(0..0);
            self.lines.push(LineRow {
                ln,
                kind,
                row,
                part: LinePart::First,
                bytes,
                block,
            });
        }
    }

    fn push_omit(&mut self, id: usize, from: u32, to: u32) {
        let row = self.rows() as u32;
        self.omits.push(OmitRow { id, from, to, row });
    }

    fn finish_seams(&mut self) {
        self.seams.sort_unstable();
        self.seams.dedup();
    }
}

/// Intra-line marks for one Replace block, one entry per line on each side.
#[derive(Debug)]
struct BlockMarks {
    preimage: Vec<Vec<TokenPart>>,
    postimage: Vec<Vec<TokenPart>>,
}

/// Per-side rows plus the connector and scroll geometry those rows imply.
#[derive(Debug)]
pub struct Layout {
    pub preimage: SideLayout,
    pub postimage: SideLayout,
    pub bridges: Vec<Bridge>,
    pub knots: Vec<ScrollKnot>,
    /// One entry per Hunk, in Alignment order; `s` is the scroll parameter at
    /// the Hunk's first visual line.
    pub hunk_lands: Vec<HunkLand>,
    pub hunk_count: usize,
    max_ln: u32,
    /// Lazy word marks, one cell per bridge (only Replace cells are filled).
    marks: Vec<OnceCell<BlockMarks>>,
    /// `None` when wrap is off (N = 1 everywhere).
    pub wrap: Option<AppliedWrap>,
}

impl Layout {
    /// Project Alignment into per-side rows and one bridge per change op.
    ///
    /// Equal lines are one row on each side. Insert rows exist only on the postimage
    /// side, delete rows only on the preimage side. A Replace is one block: the preimage
    /// lines stacked, the postimage lines stacked, with no invented partner row on the
    /// shorter side. With `fold`, Equal runs longer than `2 * EQUAL_CONTEXT`
    /// beside a Hunk collapse unless their op index is in `fold.expanded`;
    /// `None` shows every line.
    pub fn build(
        preimage_text: Arc<str>,
        postimage_text: Arc<str>,
        alignment: &Alignment,
        fold: Option<&FoldState>,
        wrap: Option<(&WrapPlan, &mut dyn FnMut(char) -> f32)>,
    ) -> Self {
        let mut ctx = wrap.map(|(plan, cw)| WrapCtx {
            plan: *plan,
            preimage_w: plan.preimage.width_px,
            postimage_w: plan.postimage.width_px,
            cw,
            preimage_breaks: HashMap::new(),
            postimage_breaks: HashMap::new(),
        });
        Self::project(preimage_text, postimage_text, alignment, fold, ctx.as_mut())
    }

    /// Visual row of a match byte in `ln` (§3.5); without wrap, the line's first row.
    pub fn row_of_match_byte(&self, side: Side, ln: u32, byte: usize) -> Option<u32> {
        let side_layout = self.side(side);
        let first = side_layout.row_of_line(ln)?;
        let Some(applied) = self.wrap.as_ref() else {
            return Some(first);
        };
        let line_row = side_layout
            .lines
            .iter()
            .find(|l| l.ln == ln && l.part == LinePart::First)?;
        let text = side_layout.text(line_row);
        let display_byte = TabExpansion::new(text).display_offset(byte);
        let mut row = first;
        if let Some(breaks) = applied.breaks(side, ln) {
            for &b in &breaks.breaks {
                if display_byte >= b {
                    row += 1;
                } else {
                    break;
                }
            }
        }
        Some(row)
    }

    fn project(
        preimage_text: Arc<str>,
        postimage_text: Arc<str>,
        alignment: &Alignment,
        fold: Option<&FoldState>,
        mut wrap: Option<&mut WrapCtx<'_>>,
    ) -> Self {
        let preimage_keep = Arc::clone(&preimage_text);
        let postimage_keep = Arc::clone(&postimage_text);
        let preimage_str = preimage_keep.as_ref();
        let postimage_str = postimage_keep.as_ref();
        let preimage_ranges = line_ranges(preimage_str);
        let postimage_ranges = line_ranges(postimage_str);
        let mut preimage = SideLayout::new(preimage_text);
        let mut postimage = SideLayout::new(postimage_text);
        let mut bridges = Vec::new();
        let mut knots = vec![ScrollKnot {
            s: 0,
            preimage_y: 0,
            postimage_y: 0,
        }];
        let mut hunk_lands = Vec::new();
        let mut max_ln = 0u32;
        let has_hunk = alignment
            .ops
            .iter()
            .any(|op| !matches!(op, AlignmentOp::Equal { .. }));

        for (op_idx, op) in alignment.ops.iter().enumerate() {
            let s = knots.last().map(|k| k.s).unwrap_or(0);
            if let Some(target) = land_from_op(op) {
                hunk_lands.push(HunkLand { target, s });
            }
            match *op {
                AlignmentOp::Equal { preimage: o, postimage: n } => {
                    let count = o.count.min(n.count);
                    max_ln = max_ln.max(span_end(o, count)).max(span_end(n, count));
                    let collapse = fold.is_some_and(|f| {
                        has_hunk && count > EQUAL_CONTEXT * 2 && !f.expanded.contains(&op_idx)
                    });
                    if collapse {
                        let head = EQUAL_CONTEXT;
                        let tail = EQUAL_CONTEXT;
                        push_equal_span(
                            &mut preimage,
                            &mut postimage,
                            preimage_str,
                            postimage_str,
                            &preimage_ranges,
                            &postimage_ranges,
                            o.start,
                            n.start,
                            head,
                            &mut knots,
                            wrap.as_deref_mut(),
                        );

                        preimage.push_omit(op_idx, o.start + head, o.start + count - tail - 1);
                        postimage.push_omit(op_idx, n.start + head, n.start + count - tail - 1);
                        advance(&mut knots, 1, 1);

                        push_equal_span(
                            &mut preimage,
                            &mut postimage,
                            preimage_str,
                            postimage_str,
                            &preimage_ranges,
                            &postimage_ranges,
                            o.start + count - tail,
                            n.start + count - tail,
                            tail,
                            &mut knots,
                            wrap.as_deref_mut(),
                        );
                    } else {
                        push_equal_span(
                            &mut preimage,
                            &mut postimage,
                            preimage_str,
                            postimage_str,
                            &preimage_ranges,
                            &postimage_ranges,
                            o.start,
                            n.start,
                            count,
                            &mut knots,
                            wrap.as_deref_mut(),
                        );
                    }
                }
                AlignmentOp::Insert { after_preimage, postimages } => {
                    max_ln = max_ln.max(span_end(postimages, postimages.count));
                    let preimage_seam = preimage.rows() as u32;
                    let postimage_from = postimage.rows() as u32;
                    let d_postimage = push_side_span(
                        &mut postimage,
                        postimage_str,
                        &postimage_ranges,
                        postimages,
                        LineKind::Insert,
                        None,
                        wrap.as_deref_mut(),
                        Side::Postimage,
                    );
                    advance(&mut knots, 0, d_postimage);
                    preimage.seams.push(preimage_seam);
                    bridges.push(Bridge::Insert {
                        after_preimage,
                        postimages,
                        preimage_seam,
                        postimage_from,
                        postimage_to: postimage.rows() as u32,
                    });
                }
                AlignmentOp::Delete { preimages, at_postimage } => {
                    max_ln = max_ln.max(span_end(preimages, preimages.count));
                    let preimage_from = preimage.rows() as u32;
                    let postimage_seam = postimage.rows() as u32;
                    let d_preimage = push_side_span(
                        &mut preimage,
                        preimage_str,
                        &preimage_ranges,
                        preimages,
                        LineKind::Delete,
                        None,
                        wrap.as_deref_mut(),
                        Side::Preimage,
                    );
                    advance(&mut knots, d_preimage, 0);
                    postimage.seams.push(postimage_seam);
                    bridges.push(Bridge::Delete {
                        preimages,
                        at_postimage,
                        preimage_from,
                        preimage_to: preimage.rows() as u32,
                        postimage_seam,
                    });
                }
                AlignmentOp::Replace { preimages, postimages } => {
                    max_ln = max_ln
                        .max(span_end(preimages, preimages.count))
                        .max(span_end(postimages, postimages.count));
                    let block = Some(bridges.len() as u32);
                    let preimage_from = preimage.rows() as u32;
                    let postimage_from = postimage.rows() as u32;
                    let (d_preimage, d_postimage) = push_replace_block(
                        &mut preimage,
                        &mut postimage,
                        preimage_str,
                        postimage_str,
                        &preimage_ranges,
                        &postimage_ranges,
                        preimages,
                        postimages,
                        block,
                        wrap.as_deref_mut(),
                    );
                    let common = d_preimage.min(d_postimage);
                    advance(&mut knots, common, common);
                    advance(&mut knots, d_preimage - common, 0);
                    advance(&mut knots, 0, d_postimage - common);
                    bridges.push(Bridge::Replace {
                        preimages,
                        postimages,
                        preimage_from,
                        preimage_to: preimage.rows() as u32,
                        postimage_from,
                        postimage_to: postimage.rows() as u32,
                    });
                }
            }
        }
        preimage.finish_seams();
        postimage.finish_seams();
        let marks = bridges.iter().map(|_| OnceCell::new()).collect();
        let wrap = wrap.map(|ctx| AppliedWrap {
            plan: ctx.plan,
            preimage_breaks: std::mem::take(&mut ctx.preimage_breaks),
            postimage_breaks: std::mem::take(&mut ctx.postimage_breaks),
        });

        Self {
            preimage,
            postimage,
            bridges,
            knots,
            hunk_lands,
            hunk_count: alignment.hunks().len(),
            max_ln,
            marks,
            wrap,
        }
    }

    pub fn side(&self, side: Side) -> &SideLayout {
        match side {
            Side::Preimage => &self.preimage,
            Side::Postimage => &self.postimage,
        }
    }

    /// Largest line number any row or separator shows (sizes the gutter).
    pub fn max_line_number(&self) -> u32 {
        self.max_ln
    }

    /// Scroll parameter at the file end, in row units.
    pub fn end_s(&self) -> u32 {
        self.knots.last().map(|k| k.s).unwrap_or(0)
    }

    /// Word marks for a Replace line; `None` for other lines. Computed for the
    /// whole block the first time any of its lines asks, then memoized.
    pub fn marks(&self, side: Side, line: &LineRow) -> Option<&[TokenPart]> {
        let block = line.block? as usize;
        let Bridge::Replace {
            preimages,
            postimages,
            preimage_from,
            preimage_to,
            postimage_from,
            postimage_to,
            ..
        } = self.bridges[block]
        else {
            return None;
        };
        let marks = self.marks[block].get_or_init(|| {
            let preimage_texts = self.block_texts(Side::Preimage, preimage_from, preimage_to);
            let postimage_texts = self.block_texts(Side::Postimage, postimage_from, postimage_to);
            let (preimage, postimage) = replace_marks(&preimage_texts, &postimage_texts);
            BlockMarks { preimage, postimage }
        });
        let (parts, line_span) = match side {
            Side::Preimage => (&marks.preimage, preimages),
            Side::Postimage => (&marks.postimage, postimages),
        };
        parts
            .get(line.ln.checked_sub(line_span.start)? as usize)
            .map(Vec::as_slice)
    }

    /// Highlight runs of a Replace line as byte ranges into its text: one
    /// continuous span per run of change (see `domain::changed_runs`).
    pub fn mark_runs(&self, side: Side, line: &LineRow) -> Option<Vec<(usize, usize)>> {
        self.marks(side, line).map(changed_runs)
    }

    fn block_texts(&self, side: Side, from: u32, to: u32) -> Vec<&str> {
        let s = self.side(side);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for r in from..to {
            let Some(Row::Line(l)) = s.row(r as usize) else {
                continue;
            };
            if l.part == LinePart::First && seen.insert(l.ln) {
                out.push(s.text(l));
            }
        }
        out
    }

    /// Rebuild the comment row index. Call after a rebuild and when comments change.
    pub fn set_comments<'a>(&mut self, anchors: impl IntoIterator<Item = &'a Anchor>) {
        self.preimage.commented.clear();
        self.postimage.commented.clear();
        for anchor in anchors {
            let Some((side, lines)) = anchor.lines() else {
                continue;
            };
            let set = match side {
                Side::Preimage => &mut self.preimage.commented,
                Side::Postimage => &mut self.postimage.commented,
            };
            set.extend(lines);
        }
    }
}

/// Byte ranges of each line, same splitting as [`crate::domain::split_lines`].
fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let base = text.as_ptr() as usize;
    text.lines()
        .map(|line| {
            let start = line.as_ptr() as usize - base;
            start..start + line.len()
        })
        .collect()
}

fn span(start: u32, count: u32) -> LineSpan {
    LineSpan { start, count }
}

fn span_end(span: LineSpan, count: u32) -> u32 {
    if count == 0 {
        0
    } else {
        span.start + count - 1
    }
}

fn line_bytes(ranges: &[Range<usize>], ln: u32) -> Range<usize> {
    ranges
        .get(ln.saturating_sub(1) as usize)
        .cloned()
        .unwrap_or(0..0)
}

fn push_equal_span(
    preimage: &mut SideLayout,
    postimage: &mut SideLayout,
    preimage_str: &str,
    postimage_str: &str,
    preimage_ranges: &[Range<usize>],
    postimage_ranges: &[Range<usize>],
    o_start: u32,
    n_start: u32,
    count: u32,
    knots: &mut Vec<ScrollKnot>,
    wrap: Option<&mut WrapCtx<'_>>,
) {
    if let Some(w) = wrap {
        for i in 0..count {
            let o_ln = o_start + i;
            let n_ln = n_start + i;
            let d = w.push_equal_pair(
                preimage,
                postimage,
                preimage_str,
                postimage_str,
                line_bytes(preimage_ranges, o_ln),
                line_bytes(postimage_ranges, n_ln),
                o_ln,
                n_ln,
                LineKind::Equal,
            );
            advance(knots, d, d);
        }
    } else {
        preimage.push_lines(preimage_ranges, span(o_start, count), LineKind::Equal, None);
        postimage.push_lines(postimage_ranges, span(n_start, count), LineKind::Equal, None);
        advance(knots, count, count);
    }
}

fn push_side_span(
    side: &mut SideLayout,
    side_str: &str,
    ranges: &[Range<usize>],
    span_lines: LineSpan,
    kind: LineKind,
    block: Option<u32>,
    wrap: Option<&mut WrapCtx<'_>>,
    which: Side,
) -> u32 {
    if let Some(w) = wrap {
        let mut total = 0u32;
        for i in 0..span_lines.count {
            let ln = span_lines.start + i;
            total += w.push_line_rows(
                side,
                side_str,
                line_bytes(ranges, ln),
                ln,
                kind,
                block,
                which,
            );
        }
        total
    } else {
        side.push_lines(ranges, span_lines, kind, block);
        span_lines.count
    }
}

fn push_replace_block(
    preimage: &mut SideLayout,
    postimage: &mut SideLayout,
    preimage_str: &str,
    postimage_str: &str,
    preimage_ranges: &[Range<usize>],
    postimage_ranges: &[Range<usize>],
    preimages: LineSpan,
    postimages: LineSpan,
    block: Option<u32>,
    wrap: Option<&mut WrapCtx<'_>>,
) -> (u32, u32) {
    let mut d_preimage = 0u32;
    let mut d_postimage = 0u32;
    if let Some(w) = wrap {
        for i in 0..preimages.count {
            let ln = preimages.start + i;
            d_preimage += w.push_line_rows(
                preimage,
                preimage_str,
                line_bytes(preimage_ranges, ln),
                ln,
                LineKind::Replace,
                block,
                Side::Preimage,
            );
        }
        for i in 0..postimages.count {
            let ln = postimages.start + i;
            d_postimage += w.push_line_rows(
                postimage,
                postimage_str,
                line_bytes(postimage_ranges, ln),
                ln,
                LineKind::Replace,
                block,
                Side::Postimage,
            );
        }
    } else {
        preimage.push_lines(preimage_ranges, preimages, LineKind::Replace, block);
        postimage.push_lines(postimage_ranges, postimages, LineKind::Replace, block);
        d_preimage = preimages.count;
        d_postimage = postimages.count;
    }
    (d_preimage, d_postimage)
}

fn advance(knots: &mut Vec<ScrollKnot>, d_preimage: u32, d_postimage: u32) {
    if d_preimage == 0 && d_postimage == 0 {
        return;
    }
    let last = *knots.last().expect("knots start with the origin");
    knots.push(ScrollKnot {
        s: last.s + d_preimage.max(d_postimage),
        preimage_y: last.preimage_y + d_preimage,
        postimage_y: last.postimage_y + d_postimage,
    });
}

fn land_from_op(op: &AlignmentOp) -> Option<HunkJumpTarget> {
    let preimage_land = |ln| HunkJumpTarget {
        side: Side::Preimage,
        ln,
    };
    let postimage_land = |ln| HunkJumpTarget {
        side: Side::Postimage,
        ln,
    };
    match *op {
        AlignmentOp::Equal { .. } => None,
        AlignmentOp::Insert { postimages, .. } => (postimages.count > 0).then(|| postimage_land(postimages.start)),
        AlignmentOp::Delete { preimages, .. } => (preimages.count > 0).then(|| preimage_land(preimages.start)),
        AlignmentOp::Replace { preimages, postimages } => {
            if preimages.count > 0 {
                Some(preimage_land(preimages.start))
            } else if postimages.count > 0 {
                Some(postimage_land(postimages.start))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::visual_wrap::{WrapPlan, WrapSide};
    use super::*;

    pub(crate) fn build(
        preimage: &str,
        postimage: &str,
        ops: Vec<AlignmentOp>,
        fold: Option<&FoldState>,
    ) -> Layout {
        Layout::build(preimage.into(), postimage.into(), &Alignment { ops }, fold, None)
    }

    pub(crate) fn eq(preimage: u32, postimage: u32, count: u32) -> AlignmentOp {
        AlignmentOp::Equal {
            preimage: span(preimage, count),
            postimage: span(postimage, count),
        }
    }

    pub(crate) fn lines(from: u32, to: u32, prefix: &str) -> String {
        (from..=to)
            .map(|i| format!("{prefix}{i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `str::lines` (and thus Layout) drops a trailing `\r`, same as word-mark
    /// text — so a highlight span that ate `\r`/`\n` clips away at the line end.
    #[test]
    fn line_ranges_exclude_crlf_terminators() {
        let text = "// c\r\nfn x\n";
        let ranges = line_ranges(text);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&text[ranges[0].clone()], "// c");
        assert_eq!(&text[ranges[1].clone()], "fn x");
        let layout = build(
            text,
            text,
            vec![AlignmentOp::Equal {
                preimage: span(1, 2),
                postimage: span(1, 2),
            }],
            None,
        );
        assert_eq!(layout.preimage.text(&layout.preimage.lines()[0]), "// c");
        assert_eq!(layout.preimage.lines()[0].bytes(), ranges[0]);
    }

    /// Rows as `ln text Kind` / `~id from-to`, for table-style asserts.
    fn dump(side: &SideLayout) -> Vec<String> {
        side.iter_rows()
            .map(|row| match row {
                Row::Line(l) => format!("{} {} {:?}", l.ln, side.text(l), l.kind),
                Row::Omit(o) => format!("~{} {}-{}", o.id, o.from, o.to),
            })
            .collect()
    }

    #[test]
    fn max_chars_is_the_longest_shown_line_per_side() {
        let layout = build(
            "ab
longer line
c
",
            "ab
xyzw
c
",
            vec![
                eq(1, 1, 1),
                AlignmentOp::Replace {
                    preimages: span(2, 1),
                    postimages: span(2, 1),
                },
                eq(3, 3, 1),
            ],
            None,
        );
        assert_eq!(layout.preimage.max_chars(), 11);
        assert_eq!(layout.postimage.max_chars(), 4);
        let empty = build(
            "",
            "é€x
",
            vec![AlignmentOp::Insert {
                after_preimage: 0,
                postimages: span(1, 1),
            }],
            None,
        );
        assert_eq!(empty.preimage.max_chars(), 0);
        assert_eq!(empty.postimage.max_chars(), 3);
        let tabs = build(
            "",
            "		x
ab	c
",
            vec![AlignmentOp::Insert {
                after_preimage: 0,
                postimages: span(1, 2),
            }],
            None,
        );
        assert_eq!(tabs.postimage.max_chars(), 9);
    }

    #[test]
    fn insert_at_file_start_puts_new_lines_only_on_the_new_side() {
        let layout = build(
            "alpha\nbeta\n",
            "HEAD\nNECK\nalpha\nbeta\n",
            vec![
                AlignmentOp::Insert {
                    after_preimage: 0,
                    postimages: span(1, 2),
                },
                AlignmentOp::Equal {
                    preimage: span(1, 2),
                    postimage: span(3, 2),
                },
            ],
            None,
        );
        assert_eq!(dump(&layout.preimage), ["1 alpha Equal", "2 beta Equal"]);
        assert_eq!(
            dump(&layout.postimage),
            [
                "1 HEAD Insert",
                "2 NECK Insert",
                "3 alpha Equal",
                "4 beta Equal"
            ]
        );
        assert_eq!(
            layout.bridges,
            vec![Bridge::Insert {
                after_preimage: 0,
                postimages: span(1, 2),
                preimage_seam: 0,
                postimage_from: 0,
                postimage_to: 2,
            }]
        );
        assert_eq!(
            layout.bridges[0].position_copy(),
            "Insert postimage 1\u{2013}2 after file start"
        );
        assert_eq!(layout.preimage.seams(), [0]);
        assert!(layout.preimage.is_seam(0) && !layout.preimage.is_seam(1));
        assert!(layout.postimage.seams().is_empty());
    }

    #[test]
    fn insert_at_file_end_anchors_after_the_last_old_line() {
        let preimage = lines(1, 15, "L");
        let postimage = format!("{preimage}\nN16\nN17\nN18");
        let layout = build(
            &preimage,
            &postimage,
            vec![
                eq(1, 1, 15),
                AlignmentOp::Insert {
                    after_preimage: 15,
                    postimages: span(16, 3),
                },
            ],
            None,
        );
        assert_eq!(layout.preimage.rows(), 15);
        assert!(layout.preimage.lines().iter().all(|l| l.kind == LineKind::Equal));
        assert_eq!(layout.preimage.lines()[0].ln, 1);
        assert_eq!(layout.preimage.lines()[14].ln, 15);
        assert_eq!(
            dump(&layout.postimage)[15..],
            ["16 N16 Insert", "17 N17 Insert", "18 N18 Insert"]
        );
        assert_eq!(
            layout.bridges,
            vec![Bridge::Insert {
                after_preimage: 15,
                postimages: span(16, 3),
                preimage_seam: 15,
                postimage_from: 15,
                postimage_to: 18,
            }]
        );
        assert_eq!(
            layout.bridges[0].position_copy(),
            "Insert postimage 16\u{2013}18 after preimage 15"
        );
        // The seam sits one past the last preimage row.
        assert!(layout.preimage.is_seam(15));
    }

    #[test]
    fn delete_rows_appear_only_on_the_old_side() {
        let layout = build(
            "keep\ngone-a\ngone-b\nkeep2\n",
            "keep\nkeep2\n",
            vec![
                eq(1, 1, 1),
                AlignmentOp::Delete {
                    preimages: span(2, 2),
                    at_postimage: 2,
                },
                AlignmentOp::Equal {
                    preimage: span(4, 1),
                    postimage: span(2, 1),
                },
            ],
            None,
        );
        assert_eq!(
            dump(&layout.preimage),
            [
                "1 keep Equal",
                "2 gone-a Delete",
                "3 gone-b Delete",
                "4 keep2 Equal"
            ]
        );
        assert_eq!(dump(&layout.postimage), ["1 keep Equal", "2 keep2 Equal"]);
        assert_eq!(
            layout.bridges,
            vec![Bridge::Delete {
                preimages: span(2, 2),
                at_postimage: 2,
                preimage_from: 1,
                preimage_to: 3,
                postimage_seam: 1,
            }]
        );
        assert_eq!(
            layout.bridges[0].position_copy(),
            "Delete preimage 2\u{2013}3 at postimage 2"
        );
        assert_eq!(layout.postimage.seams(), [1]);
    }

    #[test]
    fn replace_three_old_lines_with_one_new_line_has_no_partner_rows() {
        let layout = build(
            "old-a\nold-b\nold-c\n",
            "new-a\n",
            vec![AlignmentOp::Replace {
                preimages: span(1, 3),
                postimages: span(1, 1),
            }],
            None,
        );
        assert_eq!(
            dump(&layout.preimage),
            ["1 old-a Replace", "2 old-b Replace", "3 old-c Replace"]
        );
        assert_eq!(dump(&layout.postimage), ["1 new-a Replace"]);
        assert_eq!(
            layout.bridges,
            vec![Bridge::Replace {
                preimages: span(1, 3),
                postimages: span(1, 1),
                preimage_from: 0,
                preimage_to: 3,
                postimage_from: 0,
                postimage_to: 1,
            }]
        );
        assert_eq!(
            layout.bridges[0].position_copy(),
            "Replace preimage 1\u{2013}3 ↔ postimage 1"
        );
        // Knots: the common row moves both sides, then only preimage moves.
        assert_eq!(
            layout.knots,
            vec![
                ScrollKnot {
                    s: 0,
                    preimage_y: 0,
                    postimage_y: 0
                },
                ScrollKnot {
                    s: 1,
                    preimage_y: 1,
                    postimage_y: 1
                },
                ScrollKnot {
                    s: 3,
                    preimage_y: 3,
                    postimage_y: 1
                },
            ]
        );
    }

    #[test]
    fn one_side_empty_has_no_rows_and_a_single_seam() {
        let layout = build(
            "",
            "a\nb\nc\n",
            vec![AlignmentOp::Insert {
                after_preimage: 0,
                postimages: span(1, 3),
            }],
            Some(&FoldState::collapsed()),
        );
        assert!(layout.preimage.is_empty());
        assert_eq!(
            dump(&layout.postimage),
            ["1 a Insert", "2 b Insert", "3 c Insert"]
        );
        assert_eq!(layout.preimage.seams(), [0]);
        assert_eq!(layout.hunk_count, 1);
        assert_eq!(layout.hunk_lands.len(), 1);
        assert_eq!(layout.end_s(), 3);
    }

    #[test]
    fn equal_lines_are_one_row_on_each_side_with_paired_line_numbers() {
        let layout = build(
            "w\nx\ny\nsame\nstill\n",
            "a\nb\nc\nd\ne\nf\ng\nh\nsame\nstill\n",
            vec![AlignmentOp::Equal {
                preimage: span(4, 2),
                postimage: span(9, 2),
            }],
            None,
        );
        assert_eq!(dump(&layout.preimage), ["4 same Equal", "5 still Equal"]);
        assert_eq!(dump(&layout.postimage), ["9 same Equal", "10 still Equal"]);
        assert!(layout.bridges.is_empty());
        assert_eq!(layout.preimage.row_of_line(5), Some(1));
        assert_eq!(layout.postimage.row_of_line(9), Some(0));
        assert_eq!(layout.postimage.row_of_line(3), None);
    }

    pub(crate) fn ten_then_insert() -> (String, String, Vec<AlignmentOp>) {
        let preimage = lines(1, 10, "L");
        let postimage = format!("{preimage}\nINS");
        let ops = vec![
            eq(1, 1, 10),
            AlignmentOp::Insert {
                after_preimage: 10,
                postimages: span(11, 1),
            },
        ];
        (preimage, postimage, ops)
    }

    /// Long Equal run beside a Hunk: keep 3 lines at each end, one omit separator per side.
    #[test]
    fn long_equal_run_beside_hunk_collapses_to_one_separator_per_side() {
        let (preimage, postimage, ops) = ten_then_insert();
        let layout = build(&preimage, &postimage, ops, Some(&FoldState::collapsed()));
        let expect = [
            "1 L1 Equal",
            "2 L2 Equal",
            "3 L3 Equal",
            "~0 4-7",
            "8 L8 Equal",
            "9 L9 Equal",
            "10 L10 Equal",
        ];
        assert_eq!(dump(&layout.preimage), expect);
        assert_eq!(dump(&layout.postimage)[..7], expect);
        assert_eq!(dump(&layout.postimage)[7], "11 INS Insert");
        // One separator each side, same span; not in the line list.
        assert_eq!(layout.preimage.omits().len(), 1);
        assert_eq!(layout.postimage.omits().len(), 1);
        assert_eq!(layout.preimage.lines().len(), 6);
        let (o, n) = (layout.preimage.omits()[0], layout.postimage.omits()[0]);
        assert_eq!((o.id, o.from, o.to, o.row), (n.id, n.from, n.to, n.row));
        assert!(matches!(layout.preimage.row(3), Some(Row::Omit(_))));
        assert_eq!(layout.preimage.line_above(3).map(|l| l.ln), Some(3));
        assert_eq!(layout.preimage.row_of_line(8), Some(4));
        assert_eq!(layout.preimage.row_of_line(5), None);
        assert_eq!(layout.max_line_number(), 11);
    }

    #[test]
    fn equal_run_that_fits_context_stays_fully_visible() {
        // 6 Equal lines = exactly 2×CONTEXT; must not collapse.
        let preimage = lines(1, 6, "E");
        let postimage = format!("{preimage}\nX");
        let layout = build(
            &preimage,
            &postimage,
            vec![
                eq(1, 1, 6),
                AlignmentOp::Insert {
                    after_preimage: 6,
                    postimages: span(7, 1),
                },
            ],
            Some(&FoldState::collapsed()),
        );
        assert!(layout.preimage.omits().is_empty() && layout.postimage.omits().is_empty());
        assert_eq!(layout.preimage.rows(), 6);
        assert_eq!(layout.postimage.rows(), 7);
        assert_eq!(dump(&layout.preimage)[5], "6 E6 Equal");
    }

    #[test]
    fn no_hunk_file_shows_every_line_and_no_separator() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj";
        let layout = build(
            text,
            text,
            vec![eq(1, 1, 10)],
            Some(&FoldState::collapsed()),
        );
        assert_eq!(layout.preimage.rows(), 10);
        assert_eq!(layout.postimage.rows(), 10);
        assert!(layout.preimage.omits().is_empty() && layout.postimage.omits().is_empty());
        assert_eq!(dump(&layout.preimage)[0], "1 a Equal");
        assert_eq!(dump(&layout.preimage)[9], "10 j Equal");
        assert!(layout.bridges.is_empty());
        assert_eq!(layout.hunk_count, 0);
    }

    #[test]
    fn expanding_one_separator_by_id_expands_that_span_on_both_sides() {
        let (preimage, postimage, ops) = ten_then_insert();
        let mut fold = FoldState::collapsed();
        let collapsed = build(&preimage, &postimage, ops.clone(), Some(&fold));
        let Some(Row::Omit(o)) = collapsed.preimage.row(3) else {
            panic!("expected omit at row 3");
        };
        assert!(matches!(collapsed.postimage.row(3), Some(Row::Omit(n)) if n.id == o.id));

        fold.expand(o.id);
        let expanded = build(&preimage, &postimage, ops, Some(&fold));
        assert!(expanded.preimage.omits().is_empty() && expanded.postimage.omits().is_empty());
        assert_eq!(
            expanded
                .preimage
                .lines()
                .iter()
                .map(|l| l.ln)
                .collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(
            expanded
                .postimage
                .lines()
                .iter()
                .map(|l| l.ln)
                .collect::<Vec<_>>(),
            (1..=11).collect::<Vec<_>>()
        );
        assert_eq!(expanded.postimage.lines()[10].kind, LineKind::Insert);
        // Middle lines that were omitted are restored on both sides.
        assert_eq!(dump(&expanded.preimage)[3], "4 L4 Equal");
        assert_eq!(dump(&expanded.postimage)[6], "7 L7 Equal");
    }

    #[test]
    fn match_line_in_collapsed_equal_is_visible_after_expand() {
        let (preimage, postimage, ops) = ten_then_insert();
        let alignment = Alignment { ops };
        let mut fold = FoldState::collapsed();
        let collapsed = Layout::build(
            preimage.as_str().into(),
            postimage.as_str().into(),
            &alignment,
            Some(&fold),
            None,
        );
        assert_eq!(
            collapsed.preimage.row_of_line(5),
            None,
            "line 5 hidden while collapsed"
        );

        let plan = crate::domain::match_jump_plan(&alignment, &fold, Side::Preimage, 5);
        assert_eq!(plan.expand, Some(0));
        fold.expand(0);
        let expanded = Layout::build(
            preimage.as_str().into(),
            postimage.as_str().into(),
            &alignment,
            Some(&fold),
            None,
        );
        // Post-expansion: line 5 is the 5th Equal row (index 4).
        assert_eq!(expanded.preimage.row_of_line(5), Some(4));
    }

    #[test]
    fn hunk_lands_record_s_at_first_visual_line() {
        let layout = build(
            "a\nb\nc\nd",
            "a\nX\nY\nb\nc",
            vec![
                eq(1, 1, 1),
                AlignmentOp::Insert {
                    after_preimage: 1,
                    postimages: span(2, 2),
                },
                AlignmentOp::Equal {
                    preimage: span(2, 2),
                    postimage: span(4, 2),
                },
                AlignmentOp::Delete {
                    preimages: span(4, 1),
                    at_postimage: 6,
                },
            ],
            None,
        );
        assert_eq!(
            layout.hunk_lands,
            vec![
                HunkLand {
                    target: HunkJumpTarget {
                        side: Side::Postimage,
                        ln: 2
                    },
                    s: 1,
                },
                HunkLand {
                    target: HunkJumpTarget {
                        side: Side::Preimage,
                        ln: 4
                    },
                    s: 5,
                },
            ]
        );
    }

    #[test]
    fn replace_marks_are_lazy_and_per_line() {
        let layout = build(
            "int timeoutMs = 10;\nkeep",
            "int timeoutMs = 40;\nkeep",
            vec![
                AlignmentOp::Replace {
                    preimages: span(1, 1),
                    postimages: span(1, 1),
                },
                eq(2, 2, 1),
            ],
            None,
        );
        assert!(layout.marks[0].get().is_none(), "no LCS before a line asks");
        let changed = |side: Side| -> Vec<String> {
            let line = &layout.side(side).lines()[0];
            layout
                .marks(side, line)
                .unwrap()
                .iter()
                .filter(|p| p.changed)
                .map(|p| p.text.clone())
                .collect()
        };
        assert_eq!(changed(Side::Preimage), ["10"]);
        assert_eq!(changed(Side::Postimage), ["40"]);
        assert!(layout.marks[0].get().is_some());
        let old_line = &layout.preimage.lines()[0];
        assert_eq!(layout.mark_runs(Side::Preimage, old_line), Some(vec![(16, 18)]));
        // Equal lines have no marks.
        assert!(layout.marks(Side::Preimage, &layout.preimage.lines()[1]).is_none());
    }

    #[test]
    fn many_to_many_replace_marks_keep_block_first_rows() {
        let layout = build(
            "old-a\nold-b\nold-c\n",
            "new-a\n",
            vec![AlignmentOp::Replace {
                preimages: span(1, 3),
                postimages: span(1, 1),
            }],
            None,
        );
        let preimage_changed: Vec<&str> = layout
            .preimage
            .lines()
            .iter()
            .flat_map(|l| layout.marks(Side::Preimage, l).unwrap())
            .filter(|p| p.changed)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(preimage_changed, vec!["old", "old", "b", "old", "c"]);
        let new_line = &layout.postimage.lines()[0];
        assert_eq!(
            layout
                .marks(Side::Postimage, new_line)
                .unwrap()
                .iter()
                .filter(|p| p.changed)
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            vec!["new"]
        );
    }

    #[test]
    fn comment_index_covers_anchor_spans_per_side() {
        let (preimage, postimage, ops) = ten_then_insert();
        let mut layout = build(&preimage, &postimage, ops, None);
        let anchors = [
            Anchor::Line {
                path: "a".into(),
                side: Side::Postimage,
                span: span(3, 2),
                hunk: None,
            },
            Anchor::File { path: "a".into() },
        ];
        layout.set_comments(&anchors);
        assert!(layout.postimage.has_comment(3) && layout.postimage.has_comment(4));
        assert!(!layout.postimage.has_comment(5));
        assert!(!layout.preimage.has_comment(3));
        layout.set_comments(std::iter::empty());
        assert!(!layout.postimage.has_comment(3));
    }

    #[test]
    fn crlf_lines_are_byte_ranges_without_the_terminator() {
        let layout = build("a\r\nbb\r\n", "a\r\nbb\r\n", vec![eq(1, 1, 2)], None);
        assert_eq!(dump(&layout.preimage), ["1 a Equal", "2 bb Equal"]);
    }

    fn wrap_layout(
        preimage: &str,
        postimage: &str,
        ops: Vec<AlignmentOp>,
        width: f32,
        fold: Option<&FoldState>,
    ) -> Layout {
        let alignment = Alignment { ops };
        let plan = WrapPlan {
            preimage: WrapSide { width_px: width },
            postimage: WrapSide { width_px: width },
        };
        let mut cw = |_: char| 10.0f32;
        Layout::build(
            preimage.into(),
            postimage.into(),
            &alignment,
            fold,
            Some((&plan, &mut cw)),
        )
    }

    fn wrap_layout_unfolded(preimage: &str, postimage: &str, ops: Vec<AlignmentOp>, width: f32) -> Layout {
        wrap_layout(preimage, postimage, ops, width, None)
    }

    #[test]
    fn equal_wrap_two_vs_three_rows_stays_collinear_and_keeps_gap() {
        let preimage = "a".repeat(25);
        let postimage = "b".repeat(15);
        let wrapped = wrap_layout_unfolded(&preimage, &postimage, vec![eq(1, 1, 1)], 100.);
        assert_eq!(wrapped.preimage.rows(), 3);
        assert_eq!(wrapped.postimage.rows(), 3);
        assert!(wrapped.postimage.lines()[2].is_equal_padding());
        assert!(wrapped.preimage.lines()[0].shows_line_number());
        let end = wrapped.knots.last().unwrap();
        assert_eq!(end.preimage_y, end.postimage_y, "Equal pair stays collinear");
    }

    #[test]
    fn replace_wrap_knots_match_unwrapped_three_vs_two_visual_rows() {
        let preimage = "o".repeat(30);
        let postimage = format!("{}\n{}", "n".repeat(10), "m".repeat(10));
        let wrapped = wrap_layout_unfolded(
            &preimage,
            &postimage,
            vec![AlignmentOp::Replace {
                preimages: span(1, 1),
                postimages: span(1, 2),
            }],
            100.,
        );
        assert_eq!(wrapped.preimage.rows(), 3);
        assert_eq!(wrapped.postimage.rows(), 2);
        let reference = build(
            "a\nb\nc\n",
            "x\ny\n",
            vec![AlignmentOp::Replace {
                preimages: span(1, 3),
                postimages: span(1, 2),
            }],
            None,
        );
        assert_eq!(wrapped.knots, reference.knots);
    }

    #[test]
    fn row_of_match_byte_picks_visual_row() {
        let text = "a".repeat(25);
        let layout = wrap_layout_unfolded(&text, &text, vec![eq(1, 1, 1)], 100.);
        assert_eq!(layout.row_of_match_byte(Side::Preimage, 1, 0), Some(0));
        let b = layout
            .wrap
            .as_ref()
            .unwrap()
            .preimage_breaks
            .get(&1)
            .unwrap()
            .breaks[0];
        assert_eq!(layout.row_of_match_byte(Side::Preimage, 1, b), Some(1));
    }

    #[test]
    fn row_of_match_byte_without_wrap_is_first_row() {
        let layout = build("hello\n", "hello\n", vec![eq(1, 1, 1)], None);
        assert_eq!(layout.row_of_match_byte(Side::Preimage, 1, 3), Some(0));
        assert!(layout.wrap.is_none());
    }

    #[test]
    fn row_of_match_byte_on_tabbed_continuation_row() {
        let line = format!("\t{}", "a".repeat(30));
        let layout = wrap_layout_unfolded(&line, &line, vec![eq(1, 1, 1)], 50.);
        let tabs = TabExpansion::new(&line);
        let match_at = crate::domain::first_match_byte(&line, "aaaa").unwrap();
        let display = tabs.display_offset(match_at);
        let breaks = layout.wrap.as_ref().unwrap().preimage_breaks.get(&1).unwrap();
        assert!(
            breaks.breaks.iter().any(|&b| display >= b),
            "match should fall on a continuation row"
        );
        let row = layout.row_of_match_byte(Side::Preimage, 1, match_at).unwrap();
        assert!(row > layout.preimage.row_of_line(1).unwrap());
    }

    #[test]
    fn folded_file_with_wrap_keeps_omits_and_seams() {
        let pad = "x".repeat(30);
        let (preimage, postimage, ops) = ten_then_insert();
        let preimage: String = preimage
            .lines()
            .map(|l| format!("{l}{pad}"))
            .collect::<Vec<_>>()
            .join("\n");
        let postimage: String = postimage
            .lines()
            .map(|l| format!("{l}{pad}"))
            .collect::<Vec<_>>()
            .join("\n");
        let fold = FoldState::collapsed();
        let reference = build(&preimage, &postimage, ops.clone(), Some(&fold));
        let folded = wrap_layout(&preimage, &postimage, ops, 40., Some(&fold));
        assert!(
            folded.preimage.rows() > reference.preimage.rows(),
            "wrap should add visual rows beside fold"
        );
        let omit_key = |o: &OmitRow| (o.id, o.from, o.to);
        assert_eq!(
            folded.preimage.omits().iter().map(omit_key).collect::<Vec<_>>(),
            reference
                .preimage
                .omits()
                .iter()
                .map(omit_key)
                .collect::<Vec<_>>()
        );
        assert_eq!(folded.preimage.seams().len(), reference.preimage.seams().len());
        let omit_rows: Vec<_> = dump(&folded.preimage)
            .into_iter()
            .filter(|s| s.starts_with('~'))
            .collect();
        assert_eq!(omit_rows, ["~0 4-7"]);
        assert_eq!(folded.preimage.omits()[0].row, 24);
        assert_eq!(folded.preimage.seams(), [50]);
        assert!(dump(&folded.preimage).iter().any(|s| s.starts_with("8 L8")));
        assert!(dump(&folded.preimage).iter().any(|s| s.starts_with("10 L10")));
    }
}

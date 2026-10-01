//! Caret-free TextSelection on one side of the current ChangedPath.
//! Pure: no GPUI. The pane stores the result; the element maps pixels and paints.
//! See ADR-0017 and `.scratch/diff-text-selection/spec.md`.

use super::tabs::{TAB_SIZE, display_columns};
use crate::domain::Side;

/// Original-byte span on one side. Line numbers are 1-based. `end_byte` is
/// exclusive on `end_line`. Middle lines are whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    pub side: Side,
    pub start_line: u32,
    pub start_byte: usize,
    pub end_line: u32,
    pub end_byte: usize,
}

impl TextSelection {
    /// Bytes of logical line `ln` that this span covers. `line_len` is that
    /// line's original length. `None` when the line is outside the span or
    /// contributes no characters.
    pub fn bytes_on(&self, ln: u32, line_len: usize) -> Option<(usize, usize)> {
        if ln < self.start_line || ln > self.end_line {
            return None;
        }
        let start = if ln == self.start_line {
            self.start_byte.min(line_len)
        } else {
            0
        };
        let end = if ln == self.end_line {
            self.end_byte.min(line_len)
        } else {
            line_len
        };
        (end > start).then_some((start, end))
    }
}

/// Another whole-Identifier match of a settled TextSelection. The selected
/// bytes themselves are not in this set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccurrenceHighlight {
    pub side: Side,
    pub line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// What the pointer is on, already classified from Layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowClass {
    Text {
        ln: u32,
    },
    /// Layout's [`super::layout::OmitRow`] id. The pane expands that separator.
    Omission {
        id: usize,
    },
    Padding {
        ln: u32,
    },
    Hatch,
    /// Not a row under the pointer. Autoscroll uses [`Place`], not this variant,
    /// so a far hit cannot jump the span to the file's first or last line.
    Outside,
}

/// One autoscroll step. The pane applies it with the existing shared vertical
/// scroll and per-side horizontal scroll. All zero is a no-op.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollStep {
    /// Shared vertical scroll, in visual rows. Negative moves toward the top.
    pub rows: i32,
    /// Horizontal scroll, in display columns, on the pressed side. Negative
    /// moves toward column 0. Zero when soft wrap is on.
    pub columns: i32,
    /// `columns` moves both sides because horizontal sync is on.
    pub both_sides: bool,
}

/// Pointer versus the pressed side's code column. Default: inside the column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// Set when the pointer is above or below that column.
    pub vertical: Option<VerticalEdge>,
    /// -1 left of the column, +1 right. Unset over the gutter.
    pub horizontal: Option<i8>,
    /// Over the gutter: no autoscroll, and the span stays on the press side.
    pub over_gutter: bool,
    pub soft_wrap: bool,
    pub sync_horizontal: bool,
}

/// One visual row of shared vertical scroll toward the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerticalEdge {
    /// -1 above, +1 below.
    pub dir: i8,
    /// The row that enters. `None` when this side has no further row.
    pub entering: Option<RowClass>,
    /// Display column into that line. The model clamps it into the line.
    pub column: usize,
    /// Shared scroll still has room in `dir`. When false the step is a no-op.
    pub room: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Press,
    Move,
    Release,
}

/// One sample. `side` is where the pointer is; the span stays on the press.
/// `column` is a display column into the logical line (tabs already expanded).
/// `clicks` is the platform click count. The platform owns the double-click interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub phase: Phase,
    pub side: Side,
    pub class: RowClass,
    pub column: usize,
    pub clicks: usize,
    pub place: Place,
}

/// Text gesture plus the gutter line span it must not write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    selection: Option<TextSelection>,
    gutter: Option<(Side, u32, u32)>,
    press: Option<Press>,
    scroll: ScrollStep,
    /// Omission separator this gesture expands. The pane calls the existing
    /// expand path with this id.
    expand: Option<usize>,
    /// The press after an omission click is count 1, so the line that just
    /// appeared is not word-selected when the platform reports a double-click.
    single: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Press {
    side: Side,
    origin: Cell,
    head: Option<Anchor>,
    prior: Option<TextSelection>,
    text_row: bool,
    clicks: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    At { ln: u32, byte: usize },
    After { ln: u32 },
    Pad { ln: u32 },
    Hatch,
    Omit,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    At { ln: u32, byte: usize },
    After { ln: u32 },
}

impl Model {
    pub fn selection(&self) -> Option<TextSelection> {
        self.selection
    }

    pub fn gutter(&self) -> Option<(Side, u32, u32)> {
        self.gutter
    }

    /// Line-number drag. Does not write [`TextSelection`].
    pub fn drag_gutter(&mut self, side: Side, start: u32, end: u32) {
        self.gutter = Some((side, start, end));
    }

    pub fn clear_gutter(&mut self) {
        self.gutter = None;
    }

    /// Drop the TextSelection. The gutter line span stays.
    /// The pane calls this when the ChangedPath changes, the Comparison
    /// changes, or ignore-whitespace is toggled. Scroll, soft wrap, Code
    /// Font size, fold, and expand do not.
    pub fn clear_text(&mut self) {
        self.selection = None;
        self.cancel_press();
        self.scroll = ScrollStep::default();
    }

    pub fn press_side(&self) -> Option<Side> {
        self.press.as_ref().map(|p| p.side)
    }

    /// Scroll reported by the last sample. Zero unless that sample was a drag
    /// outside the pressed side's code column.
    pub fn scroll_step(&self) -> ScrollStep {
        self.scroll
    }

    /// Separator id from a press on an omission row, still set when release
    /// stays on that same separator. The pane expands it; this model does not.
    pub fn expand_separator(&self) -> Option<usize> {
        self.expand
    }

    /// Drop an in-progress gesture without changing the span.
    pub fn cancel_press(&mut self) {
        self.press = None;
        self.expand = None;
        self.single = false;
    }

    pub fn pointer<'a>(&mut self, sample: Sample, line: impl Fn(Side, u32) -> &'a str) {
        self.scroll = ScrollStep::default();
        match sample.phase {
            Phase::Press => {
                let mut clicks = sample.clicks;
                if self.single {
                    clicks = 1;
                    self.single = false;
                }
                let omit = match sample.class {
                    RowClass::Omission { id } => Some(id),
                    _ => None,
                };
                if omit.is_some() {
                    self.selection = None;
                    self.single = true;
                }
                self.expand = omit;
                let cell = cell_at(sample.class, sample.column, sample.side, &line);
                self.press = Some(Press {
                    side: sample.side,
                    origin: cell,
                    head: None,
                    prior: self.selection,
                    text_row: matches!(cell, Cell::At { .. } | Cell::After { .. }),
                    clicks,
                });
            }
            Phase::Move | Phase::Release => {
                let side = self.press.as_ref().map(|p| p.side).unwrap_or(sample.side);
                let mut cell = cell_at(sample.class, sample.column, side, &line);
                let text_row = self.press.as_ref().is_some_and(|p| p.text_row);
                if sample.phase == Phase::Move
                    && text_row
                    && let Some(next) = self.outside_cell(sample.place, sample.class, side, &line)
                {
                    cell = next;
                }
                if sample.phase == Phase::Release
                    && let Some(id) = self.expand
                {
                    self.expand = match sample.class {
                        RowClass::Omission { id: hit } if hit == id => Some(id),
                        _ => None,
                    };
                }
                self.track(cell, &line, sample.phase == Phase::Release);
            }
        }
    }

    pub fn clipboard<'a>(&self, line: impl Fn(Side, u32) -> &'a str) -> Option<String> {
        let sel = self.selection?;
        let mut parts = Vec::new();
        for ln in sel.start_line..=sel.end_line {
            let text = line(sel.side, ln);
            let start = if ln == sel.start_line {
                sel.start_byte.min(text.len())
            } else {
                0
            };
            let end = if ln == sel.end_line {
                sel.end_byte.min(text.len())
            } else {
                text.len()
            };
            parts.push(text.get(start..end).unwrap_or(""));
        }
        let joined = parts.join("\n");
        (!joined.is_empty()).then_some(joined)
    }

    /// Lighter matches of a settled one-Identifier span. During a gesture,
    /// keep painting the previous settled span until the new one is released;
    /// this avoids a blank frame when switching selections.
    /// `pre` and `post` are 1-based logical lines. `folded` lines are outside
    /// the scan.
    /// ponytail: linear scan each call; index if a profile says so.
    pub fn occurrences(
        &self,
        pre: &[&str],
        post: &[&str],
        folded: impl Fn(Side, u32) -> bool,
    ) -> Vec<OccurrenceHighlight> {
        let sel = match self.press.as_ref() {
            Some(press) => press.prior,
            None => self.selection,
        };
        let Some(sel) = sel else {
            return Vec::new();
        };
        let selected = side_lines(sel.side, pre, post)
            .get(sel.start_line.wrapping_sub(1) as usize)
            .copied()
            .unwrap_or("");
        let Some(needle) = exact_identifier(sel, selected) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for side in [Side::Preimage, Side::Postimage] {
            for (i, text) in side_lines(side, pre, post).iter().copied().enumerate() {
                let ln = (i as u32) + 1;
                if folded(side, ln) {
                    continue;
                }
                collect_matches(&mut out, side, ln, text, needle, sel);
            }
        }
        out
    }

    /// One move outside the code column. `None` means the sample's own row stands.
    fn outside_cell<'a>(
        &mut self,
        place: Place,
        hit: RowClass,
        side: Side,
        line: &impl Fn(Side, u32) -> &'a str,
    ) -> Option<Cell> {
        if place.over_gutter {
            return None;
        }
        let mut cell = None;
        if let Some(v) = place.vertical {
            if v.room {
                self.scroll.rows = i32::from(v.dir);
                cell = Some(match v.entering {
                    Some(class) => {
                        // A horizontal edge has no column on the line. Keep the
                        // anchor's column, then the horizontal step moves four.
                        let column = if place.horizontal.is_some() {
                            self.anchor_column(side, line).unwrap_or(v.column)
                        } else {
                            v.column
                        };
                        cell_at(class, column, side, line)
                    }
                    None => Cell::Outside,
                });
            } else if place.horizontal.is_none() || place.soft_wrap {
                return Some(Cell::Outside);
            }
        } else if place.horizontal.is_some() {
            let column = self.anchor_column(side, line).unwrap_or(0);
            cell = Some(cell_at(hit, column, side, line));
        }
        let Some(dir) = place.horizontal else {
            return cell;
        };
        if place.soft_wrap {
            return cell.or(Some(Cell::Outside));
        }
        self.scroll.columns = i32::from(dir) * 4;
        self.scroll.both_sides = place.sync_horizontal;
        let base = match cell {
            Some(Cell::At { .. } | Cell::After { .. } | Cell::Pad { .. }) => cell,
            _ => self.anchor_cell(),
        };
        Some(shift_columns(
            side,
            base.unwrap_or(Cell::Outside),
            self.scroll.columns,
            line,
        ))
    }

    fn anchor_column<'a>(&self, side: Side, line: &impl Fn(Side, u32) -> &'a str) -> Option<usize> {
        match self.anchor_cell()? {
            Cell::At { ln, byte } => {
                Some(display_columns(line(side, ln).get(..byte).unwrap_or("")))
            }
            Cell::After { ln } => Some(display_columns(line(side, ln))),
            _ => None,
        }
    }

    fn anchor_cell(&self) -> Option<Cell> {
        let press = self.press.as_ref()?;
        match press.head.or_else(|| anchor_of(press.origin))? {
            Anchor::At { ln, byte } => Some(Cell::At { ln, byte }),
            Anchor::After { ln } => Some(Cell::After { ln }),
        }
    }

    fn track<'a>(&mut self, cell: Cell, line: &impl Fn(Side, u32) -> &'a str, release: bool) {
        let Some(press) = self.press else {
            return;
        };
        let mut head = press.head;
        if press.text_row {
            if cell == press.origin {
                head = None;
                self.selection = press.prior;
            } else if let Some(anchor) = text_anchor(cell)
                && let Some(origin) = anchor_of(press.origin)
                && let Some(span) = span_between(press.side, origin, anchor, line)
            {
                head = Some(anchor);
                self.selection = Some(span);
            }
        }
        if let Some(press) = self.press.as_mut() {
            press.head = head;
        }
        if release {
            let press = self.press.take().expect("press");
            if press.head.is_none() {
                let next = if press.clicks >= 3 {
                    triple_click(press.side, press.origin, press.prior, line)
                } else if press.clicks == 2 {
                    double_click(press.side, press.origin, press.prior, line)
                } else {
                    click_result(press.side, press.origin, press.prior, line)
                };
                self.selection = if press.clicks >= 2 && same_identifier(press.prior, next, line) {
                    press.prior
                } else {
                    next
                };
            }
        }
    }
}

fn shift_columns<'a>(
    side: Side,
    cell: Cell,
    delta: i32,
    line: &impl Fn(Side, u32) -> &'a str,
) -> Cell {
    let (ln, col) = match cell {
        Cell::At { ln, byte } => (
            ln,
            display_columns(line(side, ln).get(..byte).unwrap_or("")),
        ),
        Cell::After { ln } => (ln, display_columns(line(side, ln))),
        other => return other,
    };
    let next = if delta >= 0 {
        col.saturating_add(delta as usize)
    } else {
        col.saturating_sub(delta.unsigned_abs() as usize)
    };
    cell_at(RowClass::Text { ln }, next, side, line)
}

fn cell_at<'a>(
    class: RowClass,
    column: usize,
    side: Side,
    line: &impl Fn(Side, u32) -> &'a str,
) -> Cell {
    match class {
        RowClass::Text { ln } => match byte_at_column(line(side, ln), column) {
            Some(byte) => Cell::At { ln, byte },
            None => Cell::After { ln },
        },
        RowClass::Padding { ln } => Cell::Pad { ln },
        RowClass::Omission { .. } => Cell::Omit,
        RowClass::Hatch => Cell::Hatch,
        RowClass::Outside => Cell::Outside,
    }
}

/// `Some(byte)` is the original character under `column`. `None` is past EOL.
fn byte_at_column(text: &str, column: usize) -> Option<usize> {
    let mut col = 0usize;
    for (i, c) in text.char_indices() {
        let w = if c == '\t' {
            TAB_SIZE - col % TAB_SIZE
        } else {
            1
        };
        if column < col + w {
            return Some(i);
        }
        col += w;
    }
    None
}

fn anchor_of(cell: Cell) -> Option<Anchor> {
    match cell {
        Cell::At { ln, byte } => Some(Anchor::At { ln, byte }),
        Cell::After { ln } => Some(Anchor::After { ln }),
        _ => None,
    }
}

fn text_anchor(cell: Cell) -> Option<Anchor> {
    match cell {
        Cell::At { ln, byte } => Some(Anchor::At { ln, byte }),
        Cell::After { ln } => Some(Anchor::After { ln }),
        // A wrap padding row has no characters. Leaving the head where it was
        // keeps the span from swallowing the rest of the logical line.
        _ => None,
    }
}

fn rank(anchor: Anchor) -> (u32, usize) {
    match anchor {
        Anchor::At { ln, byte } => (ln, byte),
        Anchor::After { ln } => (ln, usize::MAX),
    }
}

fn span_between<'a>(
    side: Side,
    a: Anchor,
    b: Anchor,
    line: &impl Fn(Side, u32) -> &'a str,
) -> Option<TextSelection> {
    let (lo, hi) = if rank(a) <= rank(b) { (a, b) } else { (b, a) };
    let (start_line, start_byte) = match lo {
        Anchor::At { ln, byte } => (ln, byte),
        Anchor::After { ln } => (ln.saturating_add(1), 0),
    };
    let (end_line, end_byte) = match hi {
        Anchor::At { ln, byte } => {
            let text = line(side, ln);
            let n = text
                .get(byte..)
                .and_then(|rest| rest.chars().next())
                .map(char::len_utf8)
                .unwrap_or(0);
            (ln, byte + n)
        }
        Anchor::After { ln } => (ln, line(side, ln).len()),
    };
    (start_line < end_line || (start_line == end_line && start_byte < end_byte)).then_some(
        TextSelection {
            side,
            start_line,
            start_byte,
            end_line,
            end_byte,
        },
    )
}

fn triple_click<'a>(
    side: Side,
    origin: Cell,
    prior: Option<TextSelection>,
    line: &impl Fn(Side, u32) -> &'a str,
) -> Option<TextSelection> {
    let ln = match origin {
        Cell::At { ln, .. } | Cell::After { ln } => ln,
        _ => return click_result(side, origin, prior, line),
    };
    let text = line(side, ln);
    Some(TextSelection {
        side,
        start_line: ln,
        start_byte: 0,
        end_line: ln,
        end_byte: text.len(),
    })
}

fn double_click<'a>(
    side: Side,
    origin: Cell,
    prior: Option<TextSelection>,
    line: &impl Fn(Side, u32) -> &'a str,
) -> Option<TextSelection> {
    let Cell::At { ln, byte } = origin else {
        return click_result(side, origin, prior, line);
    };
    let text = line(side, ln);
    let ch = text.get(byte..).and_then(|rest| rest.chars().next())?;
    if ch.is_whitespace() {
        return None;
    }
    let (start, end) = if is_ident_char(ch) {
        ident_run(text, byte)
    } else {
        (byte, byte + ch.len_utf8())
    };
    Some(TextSelection {
        side,
        start_line: ln,
        start_byte: start,
        end_line: ln,
        end_byte: end,
    })
}

fn side_lines<'a>(side: Side, pre: &'a [&'a str], post: &'a [&'a str]) -> &'a [&'a str] {
    match side {
        Side::Preimage => pre,
        Side::Postimage => post,
    }
}

fn exact_identifier(sel: TextSelection, text: &str) -> Option<&str> {
    if sel.start_line != sel.end_line {
        return None;
    }
    let slice = text.get(sel.start_byte..sel.end_byte)?;
    if slice.is_empty() || !slice.chars().all(is_ident_char) {
        return None;
    }
    let before_ok = text[..sel.start_byte]
        .chars()
        .next_back()
        .is_none_or(|c| !is_ident_char(c));
    let after_ok = text[sel.end_byte..]
        .chars()
        .next()
        .is_none_or(|c| !is_ident_char(c));
    (before_ok && after_ok).then_some(slice)
}

fn collect_matches(
    out: &mut Vec<OccurrenceHighlight>,
    side: Side,
    ln: u32,
    text: &str,
    needle: &str,
    sel: TextSelection,
) {
    let mut byte = 0usize;
    let mut start = None;
    for c in text.chars() {
        if is_ident_char(c) {
            if start.is_none() {
                start = Some(byte);
            }
        } else if let Some(s) = start.take() {
            push_ident(out, side, ln, text, s, byte, needle, sel);
        }
        byte += c.len_utf8();
    }
    if let Some(s) = start {
        push_ident(out, side, ln, text, s, byte, needle, sel);
    }
}

fn push_ident(
    out: &mut Vec<OccurrenceHighlight>,
    side: Side,
    ln: u32,
    text: &str,
    start: usize,
    end: usize,
    needle: &str,
    sel: TextSelection,
) {
    if text.get(start..end) != Some(needle) {
        return;
    }
    if side == sel.side && ln == sel.start_line && start == sel.start_byte && end == sel.end_byte {
        return;
    }
    out.push(OccurrenceHighlight {
        side,
        line: ln,
        start_byte: start,
        end_byte: end,
    });
}

fn is_ident_char(c: char) -> bool {
    (c == '_' || c.is_alphanumeric()) && !excluded_script(c)
}

/// Han, Hiragana, Katakana, Hangul.
/// ponytail: Unicode blocks, not Script. Enclosed CJK numbers outside these
/// blocks stay letters until a script table is needed.
fn excluded_script(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'
            | '\u{3005}'..='\u{3007}'
            | '\u{3040}'..='\u{30FF}'
            | '\u{3130}'..='\u{318F}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{D7B0}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF66}'..='\u{FF9D}'
            | '\u{FFA0}'..='\u{FFDC}'
    ) || matches!(
        c as u32,
        0x1AFF0..=0x1B16F
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x2EBF0..=0x2EE5F
            | 0x2F800..=0x2FA1F
            | 0x30000..=0x323AF
    )
}

fn ident_run(text: &str, at: usize) -> (usize, usize) {
    let mut start = at;
    while start > 0 {
        let Some(c) = text[..start].chars().next_back() else {
            break;
        };
        if !is_ident_char(c) {
            break;
        }
        start -= c.len_utf8();
    }
    let mut end = at;
    while let Some(c) = text[end..].chars().next() {
        if !is_ident_char(c) {
            break;
        }
        end += c.len_utf8();
    }
    (start, end)
}

fn same_identifier<'a>(
    prior: Option<TextSelection>,
    next: Option<TextSelection>,
    line: &impl Fn(Side, u32) -> &'a str,
) -> bool {
    let (Some(prior), Some(next)) = (prior, next) else {
        return false;
    };
    let text = |selection: TextSelection| {
        (selection.start_line == selection.end_line).then(|| {
            line(selection.side, selection.start_line).get(selection.start_byte..selection.end_byte)
        })
    };
    let (Some(prior_text), Some(next_text)) = (text(prior).flatten(), text(next).flatten()) else {
        return false;
    };
    exact_identifier(prior, prior_text) == Some(prior_text)
        && exact_identifier(next, next_text) == Some(next_text)
        && prior_text == next_text
}

fn click_result<'a>(
    side: Side,
    origin: Cell,
    prior: Option<TextSelection>,
    line: &impl Fn(Side, u32) -> &'a str,
) -> Option<TextSelection> {
    match origin {
        Cell::At { ln, byte } => {
            let ws = line(side, ln)
                .get(byte..)
                .and_then(|rest| rest.chars().next())
                .is_some_and(char::is_whitespace);
            if ws { None } else { prior }
        }
        Cell::After { .. } | Cell::Pad { .. } | Cell::Hatch => None,
        Cell::Omit | Cell::Outside => prior,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line<'a>(pre: &'a [&'a str], post: &'a [&'a str]) -> impl Fn(Side, u32) -> &'a str {
        move |side, ln| {
            let src = match side {
                Side::Preimage => pre,
                Side::Postimage => post,
            };
            src.get((ln as usize).wrapping_sub(1))
                .copied()
                .unwrap_or("")
        }
    }

    fn feed(model: &mut Model, pre: &[&str], post: &[&str], samples: &[Sample]) {
        for sample in samples {
            let lookup = line(pre, post);
            model.pointer(*sample, lookup);
        }
    }

    fn sample(phase: Phase, side: Side, class: RowClass, column: usize) -> Sample {
        Sample {
            phase,
            side,
            class,
            column,
            clicks: 1,
            place: Place::default(),
        }
    }

    fn multi(phase: Phase, side: Side, class: RowClass, column: usize, clicks: usize) -> Sample {
        Sample {
            clicks,
            ..sample(phase, side, class, column)
        }
    }

    fn no_fold(_: Side, _: u32) -> bool {
        false
    }

    fn highlights(
        model: &Model,
        pre: &[&str],
        post: &[&str],
        folded: impl Fn(Side, u32) -> bool,
    ) -> Vec<OccurrenceHighlight> {
        model.occurrences(pre, post, folded)
    }

    #[test]
    fn drag_on_one_line_selects_those_characters() {
        let pre = ["abcdef"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 4),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 4),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 1,
                end_line: 1,
                end_byte: 5,
            })
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("bcde"));
    }

    #[test]
    fn upward_drag_is_the_same_span() {
        let pre = ["abcdef"];
        let down = [
            sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1),
            sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 4),
        ];
        let up = [
            sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 4),
            sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 1),
        ];
        let mut a = Model::default();
        let mut b = Model::default();
        feed(&mut a, &pre, &[], &down);
        feed(&mut b, &pre, &[], &up);
        assert_eq!(a.selection(), b.selection());
        assert_eq!(a.clipboard(line(&pre, &[])).as_deref(), Some("bcde"));
    }

    #[test]
    fn drag_stays_on_the_pressed_side() {
        let pre = ["abcdef"];
        let post = ["XXXXXX"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Postimage, RowClass::Text { ln: 1 }, 3),
                sample(Phase::Release, Side::Postimage, RowClass::Text { ln: 1 }, 3),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 4,
            })
        );
        assert_eq!(model.clipboard(line(&pre, &post)).as_deref(), Some("abcd"));
    }

    #[test]
    fn multi_line_drag_is_partial_ends_and_whole_middle_without_trailing_newline() {
        let pre = ["abcdef", "XXXX", "ghijkl"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 3 }, 3),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 2,
                end_line: 3,
                end_byte: 4,
            })
        );
        let copied = model.clipboard(line(&pre, &[])).unwrap();
        assert_eq!(copied, "cdef\nXXXX\nghij");
        assert!(!copied.ends_with('\n'));
    }

    #[test]
    fn press_and_release_on_the_same_character_is_not_a_selection() {
        let pre = ["abcdef"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 4),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 1),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 1),
            ],
        );
        assert_eq!(model.selection(), None);
        assert_eq!(model.clipboard(line(&pre, &[])), None);
    }

    #[test]
    fn click_on_a_word_keeps_the_selection_and_blank_targets_clear_it() {
        let pre = ["a b", "", "x"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        let kept = model.selection();
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("a b"));

        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 3 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 3 }, 0),
            ],
        );
        assert_eq!(model.selection(), kept);

        for (class, column) in [
            (RowClass::Text { ln: 1 }, 1usize),
            (RowClass::Text { ln: 1 }, 9),
            (RowClass::Text { ln: 2 }, 0),
            (RowClass::Padding { ln: 1 }, 0),
            (RowClass::Hatch, 0),
        ] {
            feed(
                &mut model,
                &pre,
                &[],
                &[
                    sample(Phase::Press, Side::Preimage, class, column),
                    sample(Phase::Release, Side::Preimage, class, column),
                ],
            );
            assert_eq!(model.selection(), None, "{class:?} col {column}");
            model.pointer(
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                line(&pre, &[]),
            );
            model.pointer(
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                line(&pre, &[]),
            );
        }
    }

    #[test]
    fn drag_past_eol_includes_the_last_character_and_padding_adds_none() {
        let pre = ["abcdef", "ghijkl"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 40),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("cdef"));

        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                sample(Phase::Move, Side::Preimage, RowClass::Padding { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 2 }, 1),
            ],
        );
        assert_eq!(
            model.clipboard(line(&pre, &[])).as_deref(),
            Some("cdef\ngh")
        );
    }

    #[test]
    fn drag_onto_a_padding_row_does_not_take_the_rest_of_the_line() {
        let pre = ["abcdef"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                sample(Phase::Move, Side::Preimage, RowClass::Padding { ln: 1 }, 0),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("abc"));

        feed(
            &mut model,
            &pre,
            &[],
            &[sample(
                Phase::Release,
                Side::Preimage,
                RowClass::Padding { ln: 1 },
                0,
            )],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("abc"));
    }

    #[test]
    fn a_tab_is_one_character_and_the_copy_keeps_it() {
        let pre = ["a\tb"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 3),
            ],
        );
        assert_eq!(model.selection(), None);

        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 4),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 3,
            })
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("a\tb"));

        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        assert_eq!(model.selection(), None, "a click on the tab clears it");
    }

    #[test]
    fn no_text_rows_do_not_grow_a_selection() {
        let mut model = Model::default();
        feed(
            &mut model,
            &[],
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Hatch, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Hatch, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Hatch, 0),
            ],
        );
        assert_eq!(model.selection(), None);
        assert_eq!(model.clipboard(line(&[], &[])), None);
    }

    #[test]
    fn text_gestures_leave_the_gutter_span_and_gutter_drags_leave_text() {
        let pre = ["abcdef"];
        let gutter = (Side::Postimage, 3, 5);
        let mut model = Model::default();
        model.drag_gutter(gutter.0, gutter.1, gutter.2);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        let text = model.selection();
        assert!(text.is_some());
        assert_eq!(model.gutter(), Some(gutter));
        model.drag_gutter(Side::Preimage, 1, 2);
        assert_eq!(model.selection(), text);
        assert_eq!(model.gutter(), Some((Side::Preimage, 1, 2)));
        model.clear_gutter();
        assert_eq!(model.selection(), text);
        assert_eq!(model.gutter(), None);
    }

    #[test]
    fn original_bytes_count_a_multibyte_character_as_one_column() {
        let pre = ["éx"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 1),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: "éx".len(),
            })
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("éx"));
    }

    #[test]
    fn double_click_selects_the_whole_identifier() {
        let pre = ["foo_bar"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 4, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 4),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 7,
            })
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("foo_bar"));
    }

    #[test]
    fn double_click_on_foo_dot_bar_selects_only_one_side() {
        let pre = ["foo.bar"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    1,
                    2,
                ),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("foo"));

        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 5, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    5,
                    2,
                ),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 4,
                end_line: 1,
                end_byte: 7,
            })
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("bar"));
    }

    #[test]
    fn double_click_on_i_a_or_underscore_selects_that_identifier() {
        for text in ["i", "a", "_"] {
            let pre = [text];
            let mut model = Model::default();
            feed(
                &mut model,
                &pre,
                &[],
                &[
                    multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                    multi(
                        Phase::Release,
                        Side::Preimage,
                        RowClass::Text { ln: 1 },
                        0,
                        2,
                    ),
                ],
            );
            assert_eq!(
                model.selection(),
                Some(TextSelection {
                    side: Side::Preimage,
                    start_line: 1,
                    start_byte: 0,
                    end_line: 1,
                    end_byte: 1,
                }),
                "{text}"
            );
            assert_eq!(
                model.clipboard(line(&pre, &[])).as_deref(),
                Some(text),
                "{text}"
            );
        }
    }

    #[test]
    fn double_click_selects_a_unicode_letter_run() {
        let pre = ["café2"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 3, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    3,
                    2,
                ),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("café2"));
        assert_eq!(
            model.selection().map(|s| (s.start_byte, s.end_byte)),
            Some((0, "café2".len()))
        );
    }

    #[test]
    fn double_click_on_han_kana_or_hangul_selects_that_one_character() {
        // Unspaced runs are not one Identifier.
        for text in ["中文", "あい", "アイ", "한글"] {
            let pre = [text];
            let mut model = Model::default();
            feed(
                &mut model,
                &pre,
                &[],
                &[
                    multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                    multi(
                        Phase::Release,
                        Side::Preimage,
                        RowClass::Text { ln: 1 },
                        0,
                        2,
                    ),
                ],
            );
            let one = text.chars().next().unwrap();
            assert_eq!(
                model.selection(),
                Some(TextSelection {
                    side: Side::Preimage,
                    start_line: 1,
                    start_byte: 0,
                    end_line: 1,
                    end_byte: one.len_utf8(),
                }),
                "{text}"
            );
            assert_eq!(
                model.clipboard(line(&pre, &[])).as_deref(),
                Some(text.get(..one.len_utf8()).unwrap()),
                "{text}"
            );
        }
    }

    #[test]
    fn double_click_on_whitespace_clears_and_punctuation_selects_one_character() {
        let pre = ["a b"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        assert!(model.selection().is_some());
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    1,
                    2,
                ),
            ],
        );
        assert_eq!(model.selection(), None);

        let eq = ["=="];
        let mut model = Model::default();
        feed(
            &mut model,
            &eq,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    0,
                    2,
                ),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 1,
            })
        );
        assert_eq!(model.clipboard(line(&eq, &[])).as_deref(), Some("="));

        feed(
            &mut model,
            &eq,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    1,
                    2,
                ),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 1,
                end_line: 1,
                end_byte: 2,
            })
        );
    }

    #[test]
    fn double_click_selects_an_identifier_split_across_soft_wrap_rows() {
        // Column is into the original line. A continuation row is the same line.
        let pre = ["café_bar"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 5, 2),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    5,
                    2,
                ),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: "café_bar".len(),
            })
        );
        assert_eq!(
            model.clipboard(line(&pre, &[])).as_deref(),
            Some("café_bar")
        );
    }

    #[test]
    fn triple_click_selects_the_logical_line_without_a_trailing_newline() {
        let pre = ["let foo = 1", "bar baz"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 8, 3),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    8,
                    3,
                ),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 11,
            })
        );
        let copied = model.clipboard(line(&pre, &[])).unwrap();
        assert_eq!(copied, "let foo = 1");
        assert!(!copied.ends_with('\n'));

        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 2 }, 4, 3),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 2 },
                    4,
                    3,
                ),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("bar baz"));
    }

    #[test]
    fn triple_click_on_an_empty_line_selects_that_line() {
        let pre = ["abc", ""];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 2 }, 0, 3),
                multi(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Text { ln: 2 },
                    0,
                    3,
                ),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 2,
                start_byte: 0,
                end_line: 2,
                end_byte: 0,
            })
        );
    }

    #[test]
    fn triple_click_on_a_line_that_is_one_identifier_selects_that_identifier() {
        let pre = ["foo_bar"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2, 3),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 7,
            })
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("foo_bar"));
    }

    #[test]
    fn triple_click_on_whitespace_selects_the_whole_line() {
        let pre = ["a b"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1, 3),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 1),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("a b"));
    }

    #[test]
    fn double_click_keeps_thai_and_arabic_as_identifiers() {
        for text in ["คำไทย", "مرحبا"] {
            let pre = [text];
            let mut model = Model::default();
            feed(
                &mut model,
                &pre,
                &[],
                &[
                    multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                    sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                ],
            );
            assert_eq!(
                model.clipboard(line(&pre, &[])).as_deref(),
                Some(text),
                "{text}"
            );
        }
    }

    #[test]
    fn drag_does_not_light_occurrences_until_release_and_then_both_sides() {
        let pre = ["foo foo"];
        let post = ["x foo y"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 3,
            })
        );
        assert!(highlights(&model, &pre, &post, no_fold).is_empty());

        feed(
            &mut model,
            &pre,
            &post,
            &[sample(
                Phase::Release,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                2,
            )],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 4,
                    end_byte: 7,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 2,
                    end_byte: 5,
                },
            ]
        );
    }

    #[test]
    fn double_click_lights_the_same_identifier_and_a_new_press_clears_it() {
        let pre = ["foo foo"];
        let post = ["foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[multi(
                Phase::Press,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                0,
                2,
            )],
        );
        assert!(highlights(&model, &pre, &post, no_fold).is_empty());
        feed(
            &mut model,
            &pre,
            &post,
            &[sample(
                Phase::Release,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                0,
            )],
        );
        assert_eq!(
            model.selection().map(|s| (s.start_byte, s.end_byte)),
            Some((0, 3))
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 4,
                    end_byte: 7,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 0,
                    end_byte: 3,
                },
            ]
        );
        feed(
            &mut model,
            &pre,
            &post,
            &[sample(
                Phase::Press,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                4,
            )],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 4,
                    end_byte: 7,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 0,
                    end_byte: 3,
                },
            ]
        );
    }

    #[test]
    fn switching_double_click_keeps_old_occurrences_until_release() {
        let pre = ["foo bar foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        let old = highlights(&model, &pre, &[], no_fold);
        assert_eq!(old.len(), 1);

        feed(
            &mut model,
            &pre,
            &[],
            &[multi(
                Phase::Press,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                4,
                2,
            )],
        );
        assert_eq!(highlights(&model, &pre, &[], no_fold), old);

        feed(
            &mut model,
            &pre,
            &[],
            &[sample(
                Phase::Release,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                4,
            )],
        );
        assert_eq!(
            model
                .selection()
                .map(|selection| (selection.start_byte, selection.end_byte)),
            Some((4, 7))
        );
        assert!(highlights(&model, &pre, &[], no_fold).is_empty());
    }

    #[test]
    fn occurrence_match_is_case_sensitive_and_a_whole_identifier() {
        let pre = ["Foo foo userId user_id foo foo_bar"];
        let post = ["Foo", "userId", "foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![OccurrenceHighlight {
                side: Side::Postimage,
                line: 1,
                start_byte: 0,
                end_byte: 3,
            }]
        );

        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 8, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 8),
            ],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![OccurrenceHighlight {
                side: Side::Postimage,
                line: 2,
                start_byte: 0,
                end_byte: 6,
            }]
        );

        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(
                    Phase::Press,
                    Side::Preimage,
                    RowClass::Text { ln: 1 },
                    23,
                    2,
                ),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 23),
            ],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 4,
                    end_byte: 7,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 3,
                    start_byte: 0,
                    end_byte: 3,
                },
            ]
        );
    }

    #[test]
    fn a_prefix_of_an_identifier_lights_nothing() {
        let pre = ["foo_bar foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 2),
            ],
        );
        assert_eq!(
            model.selection().map(|s| (s.start_byte, s.end_byte)),
            Some((0, 3))
        );
        assert!(highlights(&model, &pre, &[], no_fold).is_empty());
    }

    #[test]
    fn one_letter_identifier_lights_every_other_unfolded_copy() {
        let pre = ["i i i i i i i i i i i i"];
        let post = ["i"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 2,
                    end_byte: 3,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 4,
                    end_byte: 5,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 6,
                    end_byte: 7,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 8,
                    end_byte: 9,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 10,
                    end_byte: 11,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 12,
                    end_byte: 13,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 14,
                    end_byte: 15,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 16,
                    end_byte: 17,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 18,
                    end_byte: 19,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 20,
                    end_byte: 21,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 1,
                    start_byte: 22,
                    end_byte: 23,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 0,
                    end_byte: 1,
                },
            ]
        );
    }

    #[test]
    fn multi_line_punctuation_and_single_cjk_light_nothing() {
        let pre = ["foo", "bar", "foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 3 }, 2),
            ],
        );
        assert_eq!(
            model.selection().map(|s| (s.start_line, s.end_line)),
            Some((1, 3))
        );
        assert!(highlights(&model, &pre, &[], no_fold).is_empty());

        let pre = ["a=b=c"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 1, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 1),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("="));
        assert!(highlights(&model, &pre, &[], no_fold).is_empty());

        for text in ["中 中", "あ あ", "ア ア", "한 한"] {
            let pre = [text];
            let mut model = Model::default();
            feed(
                &mut model,
                &pre,
                &[],
                &[
                    multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                    sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                ],
            );
            let one = text.chars().next().unwrap().len_utf8();
            assert_eq!(
                model.selection().map(|s| (s.start_byte, s.end_byte)),
                Some((0, one)),
                "{text}"
            );
            assert!(highlights(&model, &pre, &[], no_fold).is_empty(), "{text}");
        }
    }

    #[test]
    fn triple_click_lights_only_when_the_line_is_exactly_one_identifier() {
        let pre = ["foo", "foo bar"];
        let post = ["foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 3),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 2,
                    start_byte: 0,
                    end_byte: 3,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 0,
                    end_byte: 3,
                },
            ]
        );

        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 2 }, 0, 3),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 2 }, 0),
            ],
        );
        assert_eq!(
            model.clipboard(line(&pre, &post)).as_deref(),
            Some("foo bar")
        );
        assert!(highlights(&model, &pre, &post, no_fold).is_empty());
    }

    #[test]
    fn folded_lines_stay_out_until_the_caller_says_they_are_unfolded() {
        let pre = ["foo", "foo", "foo"];
        let post = ["foo"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        let hide_middle = |side: Side, ln: u32| side == Side::Preimage && ln == 2;
        assert_eq!(
            highlights(&model, &pre, &post, hide_middle),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 3,
                    start_byte: 0,
                    end_byte: 3,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 0,
                    end_byte: 3,
                },
            ]
        );
        assert_eq!(
            highlights(&model, &pre, &post, no_fold),
            vec![
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 2,
                    start_byte: 0,
                    end_byte: 3,
                },
                OccurrenceHighlight {
                    side: Side::Preimage,
                    line: 3,
                    start_byte: 0,
                    end_byte: 3,
                },
                OccurrenceHighlight {
                    side: Side::Postimage,
                    line: 1,
                    start_byte: 0,
                    end_byte: 3,
                },
            ]
        );
    }

    #[test]
    fn drag_below_scrolls_one_row_and_extends_to_the_line_that_enters() {
        let pre = ["aa", "bb", "cc", "dd", "ee"];
        let mut model = Model::default();
        let mut below = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        below.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 3 }),
            column: 0,
            room: true,
        });
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 2 }, 0),
                below,
            ],
        );
        assert_eq!(
            model.scroll_step(),
            ScrollStep {
                rows: 1,
                columns: 0,
                both_sides: false,
            }
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 2,
                start_byte: 0,
                end_line: 3,
                end_byte: 1,
            })
        );
    }

    #[test]
    fn drag_above_scrolls_one_row_toward_the_pointer_not_to_the_first_line() {
        let pre = ["aa", "bb", "cc", "dd", "ee"];
        let mut model = Model::default();
        let mut above = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        above.place.vertical = Some(VerticalEdge {
            dir: -1,
            entering: Some(RowClass::Text { ln: 3 }),
            column: 0,
            room: true,
        });
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 4 }, 0),
                above,
            ],
        );
        assert_eq!(model.scroll_step().rows, -1);
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 3,
                start_byte: 0,
                end_line: 4,
                end_byte: 1,
            })
        );
    }

    #[test]
    fn vertical_step_at_the_end_of_the_scroll_range_is_a_no_op() {
        let pre = ["aa", "bb", "cc", "dd", "ee"];
        let mut model = Model::default();
        let mut into = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        into.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 3 }),
            column: 0,
            room: true,
        });
        let mut stuck = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 5 }, 0);
        stuck.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 5 }),
            column: 0,
            room: false,
        });
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 2 }, 0),
                into,
                stuck,
            ],
        );
        assert_eq!(model.scroll_step(), ScrollStep::default());
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 2,
                start_byte: 0,
                end_line: 3,
                end_byte: 1,
            })
        );
    }

    #[test]
    fn entering_line_clamps_the_pointer_column_into_that_line() {
        let pre = ["abcdef", "xy"];
        let mut model = Model::default();
        let mut below = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        below.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 2 }),
            column: 50,
            room: true,
        });
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                below,
            ],
        );
        assert_eq!(model.scroll_step().rows, 1);
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 2,
                end_byte: 2,
            })
        );
    }

    #[test]
    fn drag_past_the_right_edge_scrolls_four_columns_on_the_pressed_side() {
        let pre = ["abcdefghijklmnopqrstuvwxyz"];
        let mut model = Model::default();
        let mut right = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 25);
        right.place.horizontal = Some(1);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                right,
            ],
        );
        assert_eq!(
            model.scroll_step(),
            ScrollStep {
                rows: 0,
                columns: 4,
                both_sides: false,
            }
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 1,
                end_byte: 5,
            })
        );
    }

    #[test]
    fn pointer_over_the_gutter_does_not_scroll_and_stays_on_the_pressed_side() {
        let pre = ["abcdef", "ghijkl"];
        let post = ["zzzzzz", "yyyyyy"];
        let mut model = Model::default();
        let mut gutter = sample(Phase::Move, Side::Postimage, RowClass::Text { ln: 2 }, 1);
        gutter.place.over_gutter = true;
        gutter.place.horizontal = Some(1);
        feed(
            &mut model,
            &pre,
            &post,
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                gutter,
            ],
        );
        assert_eq!(model.scroll_step(), ScrollStep::default());
        assert_eq!(model.gutter(), None);
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 2,
                end_byte: 2,
            })
        );
    }

    #[test]
    fn horizontal_sync_scrolls_both_sides_by_four_columns() {
        let pre = ["abcdefghijklmnopqrstuvwxyz"];
        let mut model = Model::default();
        let mut right = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 25);
        right.place.horizontal = Some(1);
        right.place.sync_horizontal = true;
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                right,
            ],
        );
        assert_eq!(
            model.scroll_step(),
            ScrollStep {
                rows: 0,
                columns: 4,
                both_sides: true,
            }
        );
        assert_eq!(model.selection().map(|s| s.side), Some(Side::Preimage));
        assert_eq!(model.selection().map(|s| s.end_byte), Some(5));
    }

    #[test]
    fn drag_past_the_left_edge_scrolls_four_columns_toward_the_start() {
        let pre = ["abcdefghijklmnopqrstuvwxyz"];
        let mut model = Model::default();
        let mut left = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 0);
        left.place.horizontal = Some(-1);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 8),
                left,
            ],
        );
        assert_eq!(model.scroll_step().columns, -4);
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 4,
                end_line: 1,
                end_byte: 9,
            })
        );
    }

    #[test]
    fn another_horizontal_sample_is_another_four_columns() {
        let pre = ["abcdefghijklmnopqrstuvwxyz"];
        let mut model = Model::default();
        let mut right = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 25);
        right.place.horizontal = Some(1);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                right,
                right,
            ],
        );
        assert_eq!(model.scroll_step().columns, 4);
        assert_eq!(
            model.selection().map(|s| (s.start_byte, s.end_byte)),
            Some((0, 9))
        );
    }

    #[test]
    fn soft_wrap_reports_no_horizontal_scroll() {
        let pre = ["abcdefghijklmnopqrstuvwxyz"];
        let mut model = Model::default();
        let mut right = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 25);
        right.place.horizontal = Some(1);
        right.place.soft_wrap = true;
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                right,
            ],
        );
        assert_eq!(model.scroll_step(), ScrollStep::default());
        assert_eq!(model.selection(), None);
    }

    #[test]
    fn autoscroll_does_not_light_occurrences_until_release() {
        let pre = ["foo foo"];
        let post = ["x foo y"];
        let mut model = Model::default();
        let mut below = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        below.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 1 }),
            column: 2,
            room: true,
        });
        feed(
            &mut model,
            &pre,
            &post,
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                below,
            ],
        );
        assert_eq!(model.scroll_step().rows, 1);
        assert!(highlights(&model, &pre, &post, no_fold).is_empty());
        feed(
            &mut model,
            &pre,
            &post,
            &[sample(
                Phase::Release,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                2,
            )],
        );
        assert_eq!(model.scroll_step(), ScrollStep::default());
        assert!(!highlights(&model, &pre, &post, no_fold).is_empty());
    }

    #[test]
    fn corner_scrolls_one_row_and_four_columns_from_the_anchor_not_the_line_end() {
        let pre = ["abcdefghij", "abcdefghij"];
        let mut model = Model::default();
        let mut corner = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        corner.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 2 }),
            column: 99,
            room: true,
        });
        corner.place.horizontal = Some(1);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                corner,
            ],
        );
        assert_eq!(
            model.scroll_step(),
            ScrollStep {
                rows: 1,
                columns: 4,
                both_sides: false,
            }
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 2,
                end_line: 2,
                end_byte: 7,
            })
        );
    }

    #[test]
    fn horizontal_edge_on_a_visible_row_extends_to_that_row() {
        let pre = ["abcdefghij", "abcdefghij"];
        let mut model = Model::default();
        let mut right = sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 2 }, 99);
        right.place.horizontal = Some(1);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                right,
            ],
        );
        assert_eq!(model.scroll_step().rows, 0);
        assert_eq!(model.scroll_step().columns, 4);
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 2,
                end_byte: 5,
            })
        );
    }

    #[test]
    fn press_on_an_omission_separator_expands_it_and_clears_the_selection() {
        let pre = ["keep me", "hidden"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 4),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 4),
            ],
        );
        assert!(model.selection().is_some());
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(
                    Phase::Press,
                    Side::Preimage,
                    RowClass::Omission { id: 3 },
                    0,
                ),
                sample(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Omission { id: 3 },
                    0,
                ),
            ],
        );
        assert_eq!(model.selection(), None, "the separator is not a text span");
        assert_eq!(model.expand_separator(), Some(3));
        assert_eq!(model.clipboard(line(&pre, &[])), None);
    }

    #[test]
    fn the_press_after_an_omission_is_a_single_click_even_when_reported_as_a_double_click() {
        let pre = ["foo_bar"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(
                    Phase::Press,
                    Side::Preimage,
                    RowClass::Omission { id: 1 },
                    0,
                ),
                sample(
                    Phase::Release,
                    Side::Preimage,
                    RowClass::Omission { id: 1 },
                    0,
                ),
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        assert_eq!(
            model.selection(),
            None,
            "the line that just appeared is not word-selected"
        );
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0, 2),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 0),
            ],
        );
        assert_eq!(
            model
                .selection()
                .map(|span| (span.start_byte, span.end_byte)),
            Some((0, "foo_bar".len())),
            "a later double-click still selects the word"
        );
    }

    #[test]
    fn only_a_press_on_a_text_row_can_drag_or_double_click() {
        let pre = ["foo_bar", "tail"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(
                    Phase::Press,
                    Side::Preimage,
                    RowClass::Omission { id: 2 },
                    0,
                    2,
                ),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 1),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 1),
            ],
        );
        assert_eq!(model.selection(), None);
        assert_eq!(model.expand_separator(), None);

        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Omission { id: 2 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 2 }, 1),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 0,
                end_line: 2,
                end_byte: 2,
            })
        );
        assert_eq!(model.expand_separator(), None);

        let mut below = sample(Phase::Move, Side::Preimage, RowClass::Outside, 0);
        below.place.vertical = Some(VerticalEdge {
            dir: 1,
            entering: Some(RowClass::Text { ln: 2 }),
            column: 0,
            room: true,
        });
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(
                    Phase::Press,
                    Side::Preimage,
                    RowClass::Omission { id: 2 },
                    0,
                ),
                below,
            ],
        );
        assert_eq!(model.scroll_step(), ScrollStep::default());
        assert_eq!(model.selection(), None);
    }

    #[test]
    fn a_drag_across_a_fold_includes_the_hidden_original_text() {
        let pre = ["above", "hidden", "below"];
        let post = ["hidden"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &post,
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 2),
                sample(Phase::Move, Side::Preimage, RowClass::Omission { id: 4 }, 0),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 3 }, 5),
            ],
        );
        assert_eq!(
            model.selection(),
            Some(TextSelection {
                side: Side::Preimage,
                start_line: 1,
                start_byte: 2,
                end_line: 3,
                end_byte: 5,
            })
        );
        assert_eq!(
            model.clipboard(line(&pre, &post)).as_deref(),
            Some("ove\nhidden\nbelow")
        );
        let sel = model.selection().expect("span");
        assert_eq!(sel.bytes_on(2, "hidden".len()), Some((0, "hidden".len())));
        assert!(
            highlights(&model, &pre, &post, |side, ln| {
                side == Side::Preimage && ln == 2
            })
            .is_empty(),
            "a span that covers a folded line is not an occurrence scan of that line"
        );
    }

    #[test]
    fn fold_and_expand_keep_the_span_and_its_bytes() {
        let pre = ["above", "hidden", "below"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                multi(Phase::Press, Side::Preimage, RowClass::Text { ln: 2 }, 0, 3),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 2 }, 0),
            ],
        );
        let sel = model.selection().expect("span");
        assert_eq!(
            (
                sel.side,
                sel.start_line,
                sel.start_byte,
                sel.end_line,
                sel.end_byte,
            ),
            (Side::Preimage, 2, 0, 2, "hidden".len())
        );
        // Fold and expand do not sample the model, so the bytes stay. The
        // pane paints them only on a text row; an omission row has none.
        assert_eq!(model.selection(), Some(sel));
        assert_eq!(sel.bytes_on(2, "hidden".len()), Some((0, "hidden".len())));
    }

    #[test]
    fn clear_text_drops_the_span_and_keeps_the_gutter_line_span() {
        let pre = ["alpha beta"];
        let mut model = Model::default();
        model.drag_gutter(Side::Postimage, 3, 4);
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 5),
            ],
        );
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("alpha "));
        model.clear_text();
        assert_eq!(model.selection(), None);
        assert_eq!(model.clipboard(line(&pre, &[])), None);
        assert!(highlights(&model, &pre, &[], no_fold).is_empty());
        feed(
            &mut model,
            &pre,
            &[],
            &[sample(
                Phase::Move,
                Side::Preimage,
                RowClass::Text { ln: 1 },
                8,
            )],
        );
        assert_eq!(model.selection(), None);
        assert_eq!(model.gutter(), Some((Side::Postimage, 3, 4)));
    }

    #[test]
    fn scroll_soft_wrap_and_code_font_keep_the_same_original_bytes() {
        // "café" is 4 display columns and 5 original bytes. Scroll, soft wrap,
        // and Code Font size are not samples, so they cannot rewrite the span.
        let pre = ["café bar"];
        let mut model = Model::default();
        feed(
            &mut model,
            &pre,
            &[],
            &[
                sample(Phase::Press, Side::Preimage, RowClass::Text { ln: 1 }, 0),
                sample(Phase::Move, Side::Preimage, RowClass::Text { ln: 1 }, 3),
                sample(Phase::Release, Side::Preimage, RowClass::Text { ln: 1 }, 3),
            ],
        );
        let sel = model.selection().expect("span");
        assert_eq!((sel.start_byte, sel.end_byte), (0, "café".len()));
        assert_ne!(sel.end_byte, 4, "the span is original bytes, not columns");
        assert_eq!(model.clipboard(line(&pre, &[])).as_deref(), Some("café"));
        assert_eq!(model.selection(), Some(sel));
    }
}

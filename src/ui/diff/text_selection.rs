//! Caret-free TextSelection on one side of the current ChangedPath.
//! Pure: no GPUI. The pane stores the result; the element maps pixels and paints.
//! See ADR-0017 and `.scratch/diff-text-selection/spec.md`.

use super::tabs::TAB_SIZE;
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

/// What the pointer is on, already classified from Layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowClass {
    Text {
        ln: u32,
    },
    Omission,
    Padding {
        ln: u32,
    },
    Hatch,
    /// Above or below the pressed side's rows. No scroll in this ticket.
    Outside,
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
}

/// Text gesture plus the gutter line span it must not write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    selection: Option<TextSelection>,
    gutter: Option<(Side, u32, u32)>,
    press: Option<Press>,
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

    pub fn press_side(&self) -> Option<Side> {
        self.press.as_ref().map(|p| p.side)
    }

    /// Drop an in-progress gesture without changing the span.
    pub fn cancel_press(&mut self) {
        self.press = None;
    }

    pub fn pointer<'a>(&mut self, sample: Sample, line: impl Fn(Side, u32) -> &'a str) {
        match sample.phase {
            Phase::Press => {
                let cell = cell_at(sample.class, sample.column, sample.side, &line);
                self.press = Some(Press {
                    side: sample.side,
                    origin: cell,
                    head: None,
                    prior: self.selection,
                    text_row: matches!(cell, Cell::At { .. } | Cell::After { .. }),
                    clicks: sample.clicks,
                });
            }
            Phase::Move | Phase::Release => {
                let side = self.press.as_ref().map(|p| p.side).unwrap_or(sample.side);
                let cell = cell_at(sample.class, sample.column, side, &line);
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
                self.selection = if press.clicks >= 3 {
                    triple_click(press.side, press.origin, press.prior, line)
                } else if press.clicks == 2 {
                    double_click(press.side, press.origin, press.prior, line)
                } else {
                    click_result(press.side, press.origin, press.prior, line)
                };
            }
        }
    }
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
        RowClass::Omission => Cell::Omit,
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
        Cell::After { ln } | Cell::Pad { ln } => Some(Anchor::After { ln }),
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
        }
    }

    fn multi(phase: Phase, side: Side, class: RowClass, column: usize, clicks: usize) -> Sample {
        Sample {
            clicks,
            ..sample(phase, side, class, column)
        }
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
}

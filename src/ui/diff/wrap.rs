//! Soft-wrap break points for one display line (post [`super::tabs::TabExpansion`]).
//! Pure: no GPUI. See `docs/dual-pane-diff.md` §3.1.1.

/// Where a wrapped line may start a new visual row, plus continuation indent.
#[derive(Debug, Clone, PartialEq)]
pub struct WrapBreaks {
    /// Byte offsets into the display line where a continuation row starts; sorted,
    /// char boundaries, never `0`.
    pub breaks: Vec<usize>,
    /// Leading indent width on continuation rows, in px (`0` when indent > half wrap).
    pub continuation_indent_px: f32,
}

/// Word characters for wrap (Zed `LineWrapper::is_word_char`, minus Zed-only `⋯`).
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '\u{00C0}'..='\u{00FF}')
        || matches!(c, '\u{0100}'..='\u{017F}')
        || matches!(c, '\u{0180}'..='\u{024F}')
        || matches!(c, '\u{0400}'..='\u{04FF}')
        || matches!(
            c,
            '-' | '_' | '.' | '\'' | '$' | '%' | '@' | '#'
                | '^' | '~' | ',' | '=' | ':'
        )
}

/// Do not break between `prev` (end of row above) and `c` (start of row below).
fn forbids_break_between(prev: char, c: char) -> bool {
    if prev == ' ' || c == ' ' {
        return false;
    }
    if prev == '-' && c == '>' {
        return true;
    }
    if prev == ':' && c == ':' {
        return true;
    }
    if (prev == '<' || prev == '>' || prev == '!') && c == '=' {
        return true;
    }
    !is_word_char(prev)
        && !is_word_char(c)
        && prev.is_ascii()
        && c.is_ascii()
}

fn segment_width(text: &str, start: usize, end: usize, char_width: &mut impl FnMut(char) -> f32) -> f32 {
    text[start..end].chars().map(|c| char_width(c)).sum()
}

fn glued_run_start(text: &str, break_ix: usize) -> usize {
    let indices: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = match indices.iter().position(|(b, _)| *b == break_ix) {
        Some(i) => i,
        None => return break_ix,
    };
    let mut start = break_ix;
    while i > 0 {
        let (_, prev) = indices[i - 1];
        let (_, cur) = indices[i];
        if forbids_break_between(prev, cur) {
            start = indices[i - 1].0;
            i -= 1;
        } else {
            break;
        }
    }
    start
}

const INDIVISIBLE_OPS: &[&str] = &[
    "->", "::", "<<=", ">>=", "&&", "//", "/*", "==", "!=", "<=", ">=",
];

fn run_is_indivisible(text: &str, start: usize, end: usize) -> bool {
    INDIVISIBLE_OPS.contains(&&text[start..end])
}

fn glued_run_end(text: &str, from_ix: usize) -> usize {
    let indices: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = match indices.iter().position(|(b, _)| *b == from_ix) {
        Some(i) => i,
        None => return from_ix + text[from_ix..].chars().next().map_or(0, |c| c.len_utf8()),
    };
    let mut end = indices[i].0 + indices[i].1.len_utf8();
    while i + 1 < indices.len() {
        let (_, prev) = indices[i];
        let (next_b, cur) = indices[i + 1];
        if forbids_break_between(prev, cur) {
            end = next_b + cur.len_utf8();
            i += 1;
        } else {
            break;
        }
    }
    end
}

fn next_char_boundary(text: &str, ix: usize) -> usize {
    if ix >= text.len() {
        return text.len();
    }
    text.char_indices()
        .map(|(b, _)| b)
        .find(|&b| b > ix)
        .unwrap_or(text.len())
}

/// Latest byte index `b` in `(last_wrap_ix, end_ix)` where the tail `b..end_ix` fits with indent.
fn tail_fit_break(
    text: &str,
    last_wrap_ix: usize,
    end_ix: usize,
    wrap_width: f32,
    indent: f32,
    char_width: &mut impl FnMut(char) -> f32,
) -> Option<usize> {
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .filter(|&b| b > last_wrap_ix && b < end_ix)
        .collect();
    for &b in bounds.iter().rev() {
        if segment_width(text, b, end_ix, char_width) + indent <= wrap_width {
            return Some(b);
        }
    }
    None
}

fn adjust_char_break_ix(
    text: &str,
    ix: usize,
    char_width: &mut impl FnMut(char) -> f32,
    wrap_width: f32,
) -> usize {
    if ix == 0 {
        return 0;
    }
    let Some(cur) = text[ix..].chars().next() else {
        return ix;
    };
    let Some(prev) = text[..ix].chars().last() else {
        return ix;
    };
    if !forbids_break_between(prev, cur) {
        return ix;
    }
    let run_start = glued_run_start(text, ix);
    let run_end = glued_run_end(text, ix);
    if run_is_indivisible(text, run_start, run_end) {
        return run_start;
    }
    let run_w = segment_width(text, run_start, run_end, char_width);
    if run_w > wrap_width {
        ix
    } else {
        run_start
    }
}

fn legalize_break_ix(
    text: &str,
    ix: usize,
    char_width: &mut impl FnMut(char) -> f32,
    wrap_width: f32,
) -> usize {
    if ix == 0 {
        return 0;
    }
    let Some(cur) = text[ix..].chars().next() else {
        return ix;
    };
    let Some(prev) = text[..ix].chars().last() else {
        return ix;
    };
    if forbids_break_between(prev, cur) {
        adjust_char_break_ix(text, ix, char_width, wrap_width)
    } else {
        ix
    }
}

pub fn wrap_display_line(
    text: &str,
    wrap_width: f32,
    mut char_width: impl FnMut(char) -> f32,
) -> WrapBreaks {
    let first_non_ws = text
        .char_indices()
        .find(|(_, c)| *c != ' ')
        .map(|(ix, _)| ix)
        .unwrap_or(text.len());

    let leading_indent_px: f32 = segment_width(text, 0, first_non_ws, &mut char_width);

    let max_glyph_w = text
        .chars()
        .map(|c| char_width(c))
        .fold(0_f32, f32::max);

    let continuation_indent_px = if leading_indent_px > wrap_width / 2.
        || leading_indent_px + max_glyph_w > wrap_width
    {
        0.
    } else {
        leading_indent_px
    };

    let mut breaks = Vec::new();
    let mut row_width = 0_f32;
    let mut last_candidate_ix = 0;
    let mut last_candidate_width = 0_f32;
    let mut last_wrap_ix = 0;
    let mut prev_c = '\0';

    for (ix, c) in text.char_indices() {
        if c == '\n' {
            continue;
        }

        let w = char_width(c);

        if ix >= first_non_ws {
            if is_word_char(c) {
                if prev_c == ' ' && c != ' ' {
                    last_candidate_ix = ix;
                    last_candidate_width = row_width;
                }
            } else if c != ' ' && !forbids_break_between(prev_c, c) {
                last_candidate_ix = ix;
                last_candidate_width = row_width;
            }
        }

        row_width += w;

        let end_ix = ix + c.len_utf8();
        loop {
            let indent = if breaks.is_empty() {
                0.
            } else {
                continuation_indent_px
            };
            if row_width + indent <= wrap_width || ix < last_wrap_ix {
                break;
            }

            let break_ix = legalize_break_ix(
                text,
                if last_candidate_ix > last_wrap_ix {
                    let b = last_candidate_ix;
                    row_width -= last_candidate_width;
                    b
                } else {
                    let b = adjust_char_break_ix(text, ix, &mut char_width, wrap_width);
                    if b <= last_wrap_ix {
                        break;
                    }
                    row_width = segment_width(text, b, end_ix, &mut char_width);
                    b
                },
                &mut char_width,
                wrap_width,
            );

            if break_ix <= last_wrap_ix {
                break;
            }

            last_wrap_ix = break_ix;
            last_candidate_ix = 0;
            breaks.push(break_ix);

            let indent = continuation_indent_px;
            while row_width + indent > wrap_width {
                let inner = legalize_break_ix(
                    text,
                    tail_fit_break(
                        text,
                        last_wrap_ix,
                        end_ix,
                        wrap_width,
                        indent,
                        &mut char_width,
                    )
                    .unwrap_or_else(|| {
                        let forced =
                            adjust_char_break_ix(text, end_ix, &mut char_width, wrap_width);
                        if forced <= last_wrap_ix {
                            next_char_boundary(text, last_wrap_ix)
                        } else {
                            forced
                        }
                    }),
                    &mut char_width,
                    wrap_width,
                );
                if inner <= last_wrap_ix || inner >= end_ix {
                    break;
                }
                last_wrap_ix = inner;
                row_width = segment_width(text, inner, end_ix, &mut char_width);
                breaks.push(inner);
            }
        }

        if c != ' ' {
            prev_c = c;
        } else {
            prev_c = ' ';
        }
    }

    WrapBreaks {
        breaks,
        continuation_indent_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::diff::tabs::TabExpansion;

    fn mono(w: f32) -> impl FnMut(char) -> f32 {
        move |c| {
            if c == '\t' {
                panic!("use TabExpansion text, not raw tabs");
            }
            w
        }
    }

    fn cjk_wide() -> impl FnMut(char) -> f32 {
        move |c| {
            if c.is_ascii() {
                10.
            } else {
                16.
            }
        }
    }

    fn row_segments<'a>(text: &'a str, breaks: &[usize]) -> Vec<&'a str> {
        let mut segs = Vec::new();
        let mut start = 0;
        for &b in breaks {
            segs.push(&text[start..b]);
            start = b;
        }
        segs.push(&text[start..]);
        segs
    }

    fn assert_width_invariant(
        text: &str,
        wrap_width: f32,
        result: &WrapBreaks,
        mut char_width: impl FnMut(char) -> f32,
    ) {
        let segs = row_segments(text, &result.breaks);
        for (i, seg) in segs.iter().enumerate() {
            let mut w: f32 = seg.chars().map(&mut char_width).sum();
            if i > 0 {
                w += result.continuation_indent_px;
            }
            let content_w = w
                - if i > 0 {
                    result.continuation_indent_px
                } else {
                    0.
                };
            let indivisible = run_is_indivisible(seg, 0, seg.len());
            assert!(
                w <= wrap_width + 0.001
                    || (seg.chars().count() == 1 && content_w > wrap_width)
                    || (indivisible && content_w > wrap_width),
                "row {i} width {w} > wrap {wrap_width}: {seg:?}"
            );
        }
    }

    fn assert_no_break_inside(text: &str, breaks: &[usize]) {
        for &b in breaks {
            let prev = text[..b].chars().last();
            let next = text[b..].chars().next();
            if let (Some(p), Some(n)) = (prev, next) {
                assert!(
                    !forbids_break_between(p, n),
                    "break at {b} splits glued pair {p:?}{n:?} in {text:?}"
                );
            }
            assert!(
                !text[..b].ends_with(':') || !text[b..].starts_with(':'),
                "break inside :: at {b} in {text:?}"
            );
        }
    }

    #[test]
    fn plain_words_break_on_spaces() {
        let text = "hello world foo";
        let w = 70.;
        let r = wrap_display_line(text, w, mono(10.));
        assert_eq!(r.breaks, vec![6, 12]);
        assert_eq!(r.continuation_indent_px, 0.);
        assert_width_invariant(text, w, &r, mono(10.));
    }

    #[test]
    fn long_type_path_wrap_45_no_colon_colon_split() {
        let text = "std::chrono::duration";
        let w = 45.;
        let r = wrap_display_line(text, w, mono(10.));
        assert_no_break_inside(text, &r.breaks);
        assert_width_invariant(text, w, &r, mono(10.));
    }

    #[test]
    fn arrow_splits_only_before_or_after_operator() {
        let w = 25.;
        let mut width = mono(10.);
        for text in ["a->b", "foo->bar"] {
            let r = wrap_display_line(text, w, &mut width);
            assert_no_break_inside(text, &r.breaks);
            let segs = row_segments(text, &r.breaks);
            assert!(!segs.iter().any(|s| *s == "a-" || *s == ">b"));
            assert_width_invariant(text, w, &r, mono(10.));
        }
    }

    #[test]
    fn shift_assign_and_block_comment_operators() {
        let w = 25.;
        let mut width = mono(10.);
        for text in ["x <<= 1", "x >>= 1", "/*a"] {
            let r = wrap_display_line(text, w, &mut width);
            assert_no_break_inside(text, &r.breaks);
            assert_width_invariant(text, w, &r, mono(10.));
        }
    }

    #[test]
    fn compound_operators_forced_wrap() {
        let cases: &[(&str, f32)] = &[
            ("a && b", 35.),
            ("// c", 25.),
            ("x <<= 1", 25.),
            ("y >>= 1", 25.),
            ("z >>= 1", 25.),
            ("w != 0", 25.),
            ("n >= 1", 25.),
            ("m <= 9", 25.),
            ("p == q", 35.),
            ("r >>= 1", 25.),
        ];
        let mut width = mono(10.);
        for (text, w) in cases {
            let r = wrap_display_line(text, *w, &mut width);
            assert!(
                !r.breaks.is_empty(),
                "{text:?} should wrap at width {w}"
            );
            assert_no_break_inside(text, &r.breaks);
            assert_width_invariant(text, *w, &r, mono(10.));
        }
    }

    #[test]
    fn cjk_breaks_per_char() {
        let text = "你好世界";
        let w = 17.;
        let r = wrap_display_line(text, w, cjk_wide());
        assert_eq!(r.breaks.len(), 3);
        assert_width_invariant(text, w, &r, cjk_wide());
    }

    #[test]
    fn long_token_char_splits() {
        let text = "aaaaaaaaaa";
        let w = 35.;
        let r = wrap_display_line(text, w, mono(10.));
        assert_eq!(r.breaks, vec![3, 6, 9]);
        assert_width_invariant(text, w, &r, mono(10.));
    }

    #[test]
    fn continuation_indent_kept_with_real_wrap_and_width_invariant() {
        let text = "    hello world extra";
        let w = 80.;
        let r = wrap_display_line(text, w, mono(10.));
        assert_eq!(r.continuation_indent_px, 40.);
        assert!(!r.breaks.is_empty());
        assert_width_invariant(text, w, &r, mono(10.));
    }

    #[test]
    fn continuation_indent_dropped_past_half_width() {
        let narrow = "      x";
        let w2 = 50.;
        let r2 = wrap_display_line(narrow, w2, mono(10.));
        assert_eq!(r2.continuation_indent_px, 0.);
        assert_width_invariant(narrow, w2, &r2, mono(10.));
    }

    #[test]
    fn cjk_with_leading_space_respects_width_invariant() {
        let text = " 你好世界";
        let w = 20.;
        let r = wrap_display_line(text, w, cjk_wide());
        assert_width_invariant(text, w, &r, cjk_wide());
    }

    #[test]
    fn tabs_via_tab_expansion() {
        let line = "ab\tc";
        let text = TabExpansion::new(line).text;
        let w = 30.;
        let r = wrap_display_line(&text, w, mono(10.));
        assert_eq!(text, "ab  c");
        assert_width_invariant(&text, w, &r, mono(10.));
    }

    #[test]
    fn line_that_fits_has_no_breaks() {
        let text = "short";
        let r = wrap_display_line(text, 100., mono(10.));
        assert!(r.breaks.is_empty());
    }

    #[test]
    fn single_char_wider_than_wrap_gets_own_row() {
        let text = "ab";
        let r = wrap_display_line(text, 15., |c| if c == 'a' { 10. } else { 20. });
        assert_eq!(r.breaks, vec![1]);
        assert_width_invariant(text, 15., &r, |c| if c == 'a' { 10. } else { 20. });
    }

    #[test]
    fn leading_indent_may_split_when_wrap_narrower_than_indent() {
        let text = "    code code";
        let w = 45.;
        let r = wrap_display_line(text, w, mono(10.));
        assert_width_invariant(text, w, &r, mono(10.));
    }
}

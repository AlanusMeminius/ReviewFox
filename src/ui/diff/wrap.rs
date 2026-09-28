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

/// Extra rule: do not break between two consecutive non-word, non-space chars when
/// both are ASCII punctuation (keeps `<<`, `&&`, `//`, … whole). CJK is excluded
/// so breaks still fall before each ideograph. `-` followed by `>` is glued so
/// `->` stays whole even though `-` is a word char elsewhere.
fn forbids_break_between(prev: char, c: char) -> bool {
    if prev == ' ' || c == ' ' {
        return false;
    }
    if prev == '-' && c == '>' {
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

    let leading_indent_px: f32 = text[..first_non_ws]
        .chars()
        .map(|c| char_width(c))
        .sum();

    let continuation_indent_px = if leading_indent_px > wrap_width / 2. {
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
                    if !forbids_break_between(prev_c, c) {
                        last_candidate_ix = ix;
                        last_candidate_width = row_width;
                    }
                }
            } else if c != ' ' && !forbids_break_between(prev_c, c) {
                last_candidate_ix = ix;
                last_candidate_width = row_width;
            }
        }

        row_width += w;
        if row_width > wrap_width && ix > last_wrap_ix {
            if last_candidate_ix > last_wrap_ix {
                last_wrap_ix = last_candidate_ix;
                row_width -= last_candidate_width;
                last_candidate_ix = 0;
            } else {
                last_wrap_ix = ix;
                row_width = w;
            }

            row_width += continuation_indent_px;
            breaks.push(last_wrap_ix);
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
            assert!(
                w <= wrap_width + 0.001
                    || (seg.chars().count() == 1
                        && w - if i > 0 {
                            result.continuation_indent_px
                        } else {
                            0.
                        } > wrap_width),
                "row {i} width {w} > wrap {wrap_width}: {seg:?}"
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
    fn long_type_path_stays_whole_until_char_split() {
        let text = "std::chrono::duration";
        let w = 100.;
        let r = wrap_display_line(text, w, mono(10.));
        for &b in &r.breaks {
            assert!(
                !text[..b].ends_with(':') && !text[b..].starts_with("::"),
                "split inside :: at {b}"
            );
        }
        assert_width_invariant(text, w, &r, mono(10.));
    }

    fn assert_no_operator_split(text: &str, segs: &[&str]) {
        for seg in segs {
            assert!(*seg != "a-" && *seg != ">b", "{text:?} split -> : {segs:?}");
        }
        for pair in segs.windows(2) {
            assert!(
                !(pair[0].ends_with('-') && pair[1].starts_with('>')),
                "{text:?} split -> across rows: {segs:?}"
            );
            assert!(
                !(pair[0].ends_with('<') && pair[1].starts_with('<')),
                "{text:?} split << across rows: {segs:?}"
            );
            assert!(
                !(pair[0].ends_with('&') && pair[1].starts_with('&')),
                "{text:?} split && across rows: {segs:?}"
            );
            assert!(
                !(pair[0].ends_with('/') && pair[1].starts_with('/')),
                "{text:?} split // across rows: {segs:?}"
            );
            assert!(
                !(pair[0].ends_with('<') && pair[1].starts_with('=')),
                "{text:?} split <<= across rows: {segs:?}"
            );
        }
    }

    #[test]
    fn operators_stay_whole() {
        let w = 40.;
        let mut width = mono(10.);

        for text in ["a->b", "x <<= 1", "a && b", "// comment"] {
            let r = wrap_display_line(text, w, &mut width);
            let segs = row_segments(text, &r.breaks);
            assert_no_operator_split(text, &segs);
            assert_width_invariant(text, w, &r, mono(10.));
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
    fn continuation_indent_kept_and_dropped_past_half_width() {
        let text = "    body";
        let w = 100.;
        let r = wrap_display_line(text, w, mono(10.));
        assert_eq!(r.continuation_indent_px, 40.);

        let narrow = "      x";
        let w2 = 50.;
        let r2 = wrap_display_line(narrow, w2, mono(10.));
        assert_eq!(r2.continuation_indent_px, 0.);
        assert_width_invariant(narrow, w2, &r2, mono(10.));
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
    fn never_breaks_inside_leading_indent_only_prefix() {
        let text = "    code code";
        let w = 45.;
        let r = wrap_display_line(text, w, mono(10.));
        assert!(r.breaks.iter().all(|&b| b >= 4));
        assert_width_invariant(text, w, &r, mono(10.));
    }
}

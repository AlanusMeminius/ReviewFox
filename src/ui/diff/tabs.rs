//! Display-only tab expansion for code text. GPUI shapes `'\t'` with zero
//! width, so each row is shaped from a copy with tabs replaced by spaces up to
//! the next tab stop. Layout's stored text is never changed: search, export,
//! comments and copy keep using the original line, and byte ranges into it
//! (word-mark runs) go through [`TabExpansion::display_offset`].
//!
//! Columns: every char other than a tab counts as one column, CJK included.
//! This matches the `chars × mono advance` width estimate, so tab stops and
//! the horizontal-scroll bound agree; a tab after wide glyphs therefore lands
//! on a char-count stop, not on a pixel grid.

/// Tab stop spacing in columns.
pub const TAB_SIZE: usize = 4;

/// A line as shaped: tabs expanded to spaces, plus the byte-offset map back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabExpansion {
    pub text: String,
    /// Per tab, `(original byte just after it, display byte just after it)`;
    /// sorted. Empty when the line has no tabs.
    stops: Vec<(usize, usize)>,
}

impl TabExpansion {
    /// Expand `line`'s tabs; without tabs the text is copied as is.
    pub fn new(line: &str) -> Self {
        if !line.contains('\t') {
            return Self {
                text: line.to_string(),
                stops: Vec::new(),
            };
        }
        let mut text = String::with_capacity(line.len() + TAB_SIZE * 2);
        let mut stops = Vec::new();
        let mut col = 0;
        for (i, c) in line.char_indices() {
            if c == '\t' {
                let w = TAB_SIZE - col % TAB_SIZE;
                text.extend(std::iter::repeat_n(' ', w));
                col += w;
                stops.push((i + 1, text.len()));
            } else {
                text.push(c);
                col += 1;
            }
        }
        Self { text, stops }
    }

    /// Display byte offset of original byte offset `b` (a char boundary of
    /// the original line, `0..=len`). A tab's start maps to the start of its
    /// spaces, the byte after it to their end.
    pub fn display_offset(&self, b: usize) -> usize {
        let after = self.stops.partition_point(|&(o, _)| o <= b);
        match after.checked_sub(1).map(|i| self.stops[i]) {
            Some((o, d)) => d + (b - o),
            None => b,
        }
    }
}

/// Display width of `line` in columns, tabs expanded (see module docs).
pub fn display_columns(line: &str) -> usize {
    line.chars().fold(0, |col, c| {
        if c == '\t' {
            col + TAB_SIZE - col % TAB_SIZE
        } else {
            col + 1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tabs_is_identity() {
        let e = TabExpansion::new("abc");
        assert_eq!(e.text, "abc");
        assert_eq!((0..=3).map(|b| e.display_offset(b)).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(display_columns("abc"), 3);
        assert_eq!(display_columns(""), 0);
    }

    #[test]
    fn leading_tabs_are_full_stops() {
        let e = TabExpansion::new("\t\tx");
        assert_eq!(e.text, "        x");
        assert_eq!(e.display_offset(0), 0);
        assert_eq!(e.display_offset(1), 4);
        assert_eq!(e.display_offset(2), 8);
        assert_eq!(e.display_offset(3), 9);
        assert_eq!(display_columns("\t\tx"), 9);
    }

    #[test]
    fn mid_line_tab_advances_to_next_stop() {
        // "ab" is at columns 0..2, the tab fills 2..4; "abcd" + tab fills 4..8.
        let e = TabExpansion::new("ab\tc");
        assert_eq!(e.text, "ab  c");
        assert_eq!(e.display_offset(2), 2);
        assert_eq!(e.display_offset(3), 4);
        assert_eq!(e.display_offset(4), 5);
        assert_eq!(TabExpansion::new("abcd\te").text, "abcd    e");
        assert_eq!(TabExpansion::new("abc\t").text, "abc ");
    }

    #[test]
    fn multiple_tabs_between_text() {
        let line = "a\tbb\t\tc";
        let e = TabExpansion::new(line);
        assert_eq!(e.text, "a   bb      c");
        let map: Vec<_> = (0..=line.len()).map(|b| e.display_offset(b)).collect();
        assert_eq!(map, [0, 1, 4, 5, 6, 8, 12, 13]);
        assert_eq!(display_columns(line), e.text.chars().count());
    }

    #[test]
    fn cjk_counts_one_column_per_char() {
        // "中文" is 2 columns (6 bytes); the tab fills columns 2..4.
        let line = "中文\tx";
        let e = TabExpansion::new(line);
        assert_eq!(e.text, "中文  x");
        assert_eq!(e.display_offset(3), 3);
        assert_eq!(e.display_offset(6), 6);
        assert_eq!(e.display_offset(7), 8);
        assert_eq!(e.display_offset(8), 9);
        assert_eq!(display_columns(line), 5);
    }

    #[test]
    fn run_ranges_map_across_tabs() {
        // Runs over "foo" and over "\tbar" (the tab itself changed).
        let line = "\tfoo\tbar";
        let e = TabExpansion::new(line);
        assert_eq!(e.text, "    foo bar");
        let map = |(a, b): (usize, usize)| (e.display_offset(a), e.display_offset(b));
        assert_eq!(map((1, 4)), (4, 7));
        assert_eq!(&e.text[4..7], "foo");
        assert_eq!(map((4, 8)), (7, 11));
        assert_eq!(&e.text[7..11], " bar");
        assert_eq!(map((0, 8)), (0, 11));
    }
}

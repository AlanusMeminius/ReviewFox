use gpui::SharedString;

/// What the font picker shows: font names filtered by the query, and the
/// selected match (Zed `FontPickerDelegate`, minus the rendering).
pub struct FontList {
    fonts: Vec<SharedString>,
    current: SharedString,
    /// Indices into `fonts` that match the query, in list order.
    matches: Vec<usize>,
    /// Index into `matches`; meaningless while `matches` is empty.
    selected: usize,
}

impl FontList {
    /// All of `fonts`, the selection on `current` (else the first). An empty
    /// `fonts` (not loaded yet) lists just `current`.
    pub fn new(fonts: Vec<SharedString>, current: SharedString) -> Self {
        let fonts = if fonts.is_empty() {
            vec![current.clone()]
        } else {
            fonts
        };
        let mut list = Self {
            fonts,
            current,
            matches: Vec::new(),
            selected: 0,
        };
        list.set_query("");
        list
    }

    /// Keep the fonts containing `query` (any case); the selection goes to
    /// the current font when it matches, else to the first match.
    pub fn set_query(&mut self, query: &str) {
        let query = query.to_lowercase();
        self.matches = (0..self.fonts.len())
            .filter(|&ix| self.fonts[ix].to_lowercase().contains(&query))
            .collect();
        self.selected = self
            .matches
            .iter()
            .position(|&ix| self.fonts[ix] == self.current)
            .unwrap_or(0);
    }

    /// Number of matches.
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// The `ix`th match.
    pub fn get(&self, ix: usize) -> Option<&SharedString> {
        self.matches.get(ix).map(|&font| &self.fonts[font])
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The font Enter would choose; `None` when nothing matches.
    pub fn selected(&self) -> Option<&SharedString> {
        self.get(self.selected)
    }

    /// Select the `ix`th match (clamped to the last).
    pub fn select(&mut self, ix: usize) {
        self.selected = ix.min(self.len().saturating_sub(1));
    }

    /// Down: the next match, wrapping to the first.
    pub fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.len();
        }
    }

    /// Up: the previous match, wrapping to the last.
    pub fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.len() - 1) % self.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(fonts: &[&'static str], current: &'static str) -> FontList {
        FontList::new(fonts.iter().map(|&f| f.into()).collect(), current.into())
    }

    fn shown(list: &FontList) -> Vec<&str> {
        (0..list.len())
            .map(|ix| list.get(ix).unwrap().as_ref())
            .collect()
    }

    fn selected(list: &FontList) -> Option<&str> {
        list.selected().map(|f| f.as_ref())
    }

    #[test]
    fn selection_starts_on_the_current_font() {
        let fonts = list(&["Arial", "Consolas", "Inter"], "Consolas");
        assert_eq!(shown(&fonts), ["Arial", "Consolas", "Inter"]);
        assert_eq!(fonts.selected_index(), 1);
        assert_eq!(selected(&fonts), Some("Consolas"));
    }

    #[test]
    fn a_current_font_not_listed_selects_the_first() {
        let fonts = list(&["Arial", "Inter"], "Gone Sans");
        assert_eq!(selected(&fonts), Some("Arial"));
    }

    #[test]
    fn until_loaded_the_list_is_just_the_current_font() {
        let fonts = list(&[], "IBM Plex Sans");
        assert_eq!(shown(&fonts), ["IBM Plex Sans"]);
        assert_eq!(selected(&fonts), Some("IBM Plex Sans"));
    }

    #[test]
    fn query_filters_by_case_insensitive_substring_in_list_order() {
        let mut fonts = list(
            &["Arial", "Cascadia Mono", "IBM Plex Mono", "Inter"],
            "Arial",
        );
        fonts.set_query("MONO");
        assert_eq!(shown(&fonts), ["Cascadia Mono", "IBM Plex Mono"]);
        fonts.set_query("plex m");
        assert_eq!(shown(&fonts), ["IBM Plex Mono"]);
        fonts.set_query("");
        assert_eq!(shown(&fonts).len(), 4);
    }

    #[test]
    fn after_a_query_the_current_font_stays_selected_else_the_first_match() {
        let mut fonts = list(
            &["Arial", "Cascadia Mono", "IBM Plex Mono", "Inter"],
            "IBM Plex Mono",
        );
        fonts.set_query("mono");
        assert_eq!(selected(&fonts), Some("IBM Plex Mono"));
        fonts.set_query("in");
        assert_eq!(fonts.selected_index(), 0);
        assert_eq!(selected(&fonts), Some("Inter"));
        fonts.set_query("");
        assert_eq!(selected(&fonts), Some("IBM Plex Mono"));
    }

    #[test]
    fn no_match_selects_nothing() {
        let mut fonts = list(&["Arial", "Inter"], "Arial");
        fonts.set_query("zzz");
        assert_eq!(fonts.len(), 0);
        assert_eq!(selected(&fonts), None);
        fonts.select_next();
        fonts.select_prev();
        assert_eq!(selected(&fonts), None);
    }

    #[test]
    fn up_and_down_wrap_around() {
        let mut fonts = list(&["Arial", "Consolas", "Inter"], "Inter");
        fonts.select_next();
        assert_eq!(selected(&fonts), Some("Arial"));
        fonts.select_prev();
        assert_eq!(selected(&fonts), Some("Inter"));
        fonts.select_prev();
        assert_eq!(selected(&fonts), Some("Consolas"));
    }

    #[test]
    fn select_picks_a_match_by_index() {
        let mut fonts = list(&["Arial", "Consolas", "Inter"], "Arial");
        fonts.set_query("n");
        fonts.select(1);
        assert_eq!(selected(&fonts), Some("Inter"));
        fonts.select(9);
        assert_eq!(selected(&fonts), Some("Inter"));
    }
}

//! Pure Settings nav tree: pages (level 1) → sections (level 2), plus the
//! selection / expand / collapse moves the keyboard and mouse drive. No GPUI.

/// One level-1 entry. `sections` are its level-2 children, in content order.
pub struct NavPage<S: 'static> {
    pub title: &'static str,
    pub sections: &'static [S],
    /// Expanded when the window opens.
    pub expanded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavEntry {
    Page(usize),
    Section { page: usize, section: usize },
}

impl NavEntry {
    /// The page this entry shows.
    pub fn page(self) -> usize {
        match self {
            NavEntry::Page(page) | NavEntry::Section { page, .. } => page,
        }
    }
}

/// Expansion + selection over a fixed tree shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavState {
    section_counts: Vec<usize>,
    expanded: Vec<bool>,
    selected: NavEntry,
}

impl NavState {
    /// Selects the first page. `pages` must not be empty.
    pub fn new<S>(pages: &[NavPage<S>]) -> Self {
        assert!(!pages.is_empty(), "settings nav needs at least one page");
        Self {
            section_counts: pages.iter().map(|p| p.sections.len()).collect(),
            expanded: pages.iter().map(|p| p.expanded).collect(),
            selected: NavEntry::Page(0),
        }
    }

    pub fn selected(&self) -> NavEntry {
        self.selected
    }

    pub fn is_expanded(&self, page: usize) -> bool {
        self.expanded.get(page).copied().unwrap_or(false)
    }

    /// Rows in display order: each page, then its sections when expanded.
    pub fn visible(&self) -> Vec<NavEntry> {
        let mut rows = Vec::new();
        for (page, &count) in self.section_counts.iter().enumerate() {
            rows.push(NavEntry::Page(page));
            if self.expanded[page] {
                rows.extend((0..count).map(|section| NavEntry::Section { page, section }));
            }
        }
        rows
    }

    /// Selects `entry` (mouse click). A section inside a collapsed page expands it.
    pub fn select(&mut self, entry: NavEntry) {
        if !self.contains(entry) {
            return;
        }
        if let NavEntry::Section { page, .. } = entry {
            self.expanded[page] = true;
        }
        self.selected = entry;
    }

    /// Down. Stops at the last row; returns whether the selection moved.
    pub fn select_next(&mut self) -> bool {
        self.step(1)
    }

    /// Up. Stops at the first row; returns whether the selection moved.
    pub fn select_prev(&mut self) -> bool {
        self.step(-1)
    }

    /// Right: expand a collapsed page, else step into its first section.
    /// Returns whether the selection moved.
    pub fn expand(&mut self) -> bool {
        let NavEntry::Page(page) = self.selected else {
            return false;
        };
        if !self.expanded[page] {
            self.expanded[page] = true;
            false
        } else if self.section_counts[page] > 0 {
            self.selected = NavEntry::Section { page, section: 0 };
            true
        } else {
            false
        }
    }

    /// Left: a section moves to its page; an expanded page collapses.
    /// Returns whether the selection moved.
    pub fn collapse(&mut self) -> bool {
        match self.selected {
            NavEntry::Section { page, .. } => {
                self.selected = NavEntry::Page(page);
                true
            }
            NavEntry::Page(page) => {
                self.expanded[page] = false;
                false
            }
        }
    }

    /// Chevron click. Collapsing the page that holds the selection selects the page.
    /// Returns whether the selection moved.
    pub fn toggle(&mut self, page: usize) -> bool {
        let Some(expanded) = self.expanded.get_mut(page) else {
            return false;
        };
        *expanded = !*expanded;
        if !*expanded && matches!(self.selected, NavEntry::Section { page: p, .. } if p == page) {
            self.selected = NavEntry::Page(page);
            return true;
        }
        false
    }

    fn contains(&self, entry: NavEntry) -> bool {
        match entry {
            NavEntry::Page(page) => page < self.section_counts.len(),
            NavEntry::Section { page, section } => self
                .section_counts
                .get(page)
                .is_some_and(|&count| section < count),
        }
    }

    fn step(&mut self, delta: isize) -> bool {
        let rows = self.visible();
        let Some(ix) = rows.iter().position(|&row| row == self.selected) else {
            return false;
        };
        let Some(next) = ix.checked_add_signed(delta).and_then(|ix| rows.get(ix)) else {
            return false;
        };
        self.selected = *next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_PAGES: &[NavPage<&str>] = &[
        NavPage {
            title: "Accounts",
            sections: &["GitLab", "GitHub"],
            expanded: true,
        },
        NavPage {
            title: "Appearance",
            sections: &["Theme"],
            expanded: false,
        },
    ];

    fn section(page: usize, section: usize) -> NavEntry {
        NavEntry::Section { page, section }
    }

    #[test]
    fn starts_on_first_page_with_default_expansion() {
        let nav = NavState::new(TWO_PAGES);
        assert_eq!(nav.selected(), NavEntry::Page(0));
        assert_eq!(
            nav.visible(),
            vec![
                NavEntry::Page(0),
                section(0, 0),
                section(0, 1),
                NavEntry::Page(1)
            ]
        );
    }

    #[test]
    fn down_and_up_walk_visible_rows_and_stop_at_the_ends() {
        let mut nav = NavState::new(TWO_PAGES);
        assert!(!nav.select_prev());
        assert!(nav.select_next());
        assert_eq!(nav.selected(), section(0, 0));
        assert!(nav.select_next());
        assert!(nav.select_next());
        assert_eq!(nav.selected(), NavEntry::Page(1));
        // Page 1 is collapsed, so its section is skipped and Down stops.
        assert!(!nav.select_next());
        assert!(nav.select_prev());
        assert_eq!(nav.selected(), section(0, 1));
    }

    #[test]
    fn right_expands_then_enters_first_section() {
        let mut nav = NavState::new(TWO_PAGES);
        nav.select(NavEntry::Page(1));
        assert!(!nav.expand());
        assert!(nav.is_expanded(1));
        assert_eq!(nav.selected(), NavEntry::Page(1));
        assert!(nav.expand());
        assert_eq!(nav.selected(), section(1, 0));
        // Right on a section does nothing.
        assert!(!nav.expand());
    }

    #[test]
    fn left_moves_to_parent_then_collapses() {
        let mut nav = NavState::new(TWO_PAGES);
        nav.select(section(0, 1));
        assert!(nav.collapse());
        assert_eq!(nav.selected(), NavEntry::Page(0));
        assert!(nav.is_expanded(0));
        assert!(!nav.collapse());
        assert!(!nav.is_expanded(0));
        assert_eq!(nav.visible(), vec![NavEntry::Page(0), NavEntry::Page(1)]);
    }

    #[test]
    fn collapsing_a_page_via_chevron_pulls_selection_up() {
        let mut nav = NavState::new(TWO_PAGES);
        nav.select(section(0, 0));
        assert!(nav.toggle(0));
        assert_eq!(nav.selected(), NavEntry::Page(0));
        // Toggling another page leaves the selection alone.
        assert!(!nav.toggle(1));
        assert!(nav.is_expanded(1));
        assert_eq!(nav.selected(), NavEntry::Page(0));
    }

    #[test]
    fn selecting_a_hidden_section_expands_its_page_and_rejects_bad_entries() {
        let mut nav = NavState::new(TWO_PAGES);
        nav.select(section(1, 0));
        assert!(nav.is_expanded(1));
        assert_eq!(nav.selected(), section(1, 0));
        assert_eq!(nav.selected().page(), 1);
        nav.select(section(1, 5));
        nav.select(NavEntry::Page(9));
        assert_eq!(nav.selected(), section(1, 0));
    }
}

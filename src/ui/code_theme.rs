//! Code Theme resolution. [`resolve`] is the seam Diff paints from.

use std::sync::{LazyLock, Mutex};

use gpui::{Rgba, rgb};

const ONE_LIGHT_SYNTAX: &[(&str, u32)] = &[
    ("attribute", 0x4b61b9),
    ("comment", 0x666769),
    ("comment.documentation", 0x65676d),
    ("constant", 0x8b5e00),
    ("constant.builtin", 0x8b5e00),
    ("constructor", 0x4b61b9),
    ("delimiter", 0x4d4f52),
    ("escape", 0x65676d),
    ("function", 0x4961b8),
    ("function.macro", 0x4961b8),
    ("function.method", 0x4961b8),
    ("function.special", 0x4961b8),
    ("keyword", 0x9a44a0),
    ("label", 0x4b61b9),
    ("number", 0x905b1f),
    ("operator", 0x2e6c98),
    ("property", 0xa64b3e),
    ("punctuation.bracket", 0x4d4f52),
    ("punctuation.delimiter", 0x4d4f52),
    ("string", 0x46713d),
    ("type", 0x2e6c98),
    ("type.builtin", 0x2e6c98),
    ("variable", 0x242529),
    ("variable.builtin", 0x242529),
    ("variable.parameter", 0xa64b3e),
];

fn syntax_slots(rows: &[(&str, u32)]) -> Vec<(String, Rgba)> {
    rows.iter()
        .map(|(name, hex)| ((*name).to_string(), rgb(*hex)))
        .collect()
}

fn one_light() -> CodeTheme {
    CodeTheme {
        id: "one-light".into(),
        label: "One Light".into(),
        slots: AuthoredSlots {
            paper: rgb(0xffffff),
            added_band: rgb(0xe8f7ee),
            deleted_band: rgb(0xe4e6e9),
            replaced_band: rgb(0xe8f0fe),
            word_difference: rgb(0xb9ceee),
            line_number: rgb(0x596579),
            default_foreground: rgb(0x172033),
            comment: rgb(0x2457d6),
            idle_comment: rgb(0x596579),
            selection: Rgba {
                a: 0.18,
                ..rgb(0x2457d6)
            },
            search_hit: rgb(0xfff4c7),
            search_current: rgb(0xffedac),
            syntax: syntax_slots(ONE_LIGHT_SYNTAX),
        },
        marks: DiffMarks {
            added_edge: rgb(0x31804a),
            deleted_edge: rgb(0x737b86),
            replaced_edge: rgb(0x4273c2),
            unchanged_edge: rgb(0x888f99),
            omission_fill: rgb(0xe8eaef),
            omission_wave: rgb(0x6b7482),
            open_comment_pad: rgb(0xf1f5ff),
            drafting_band: rgb(0xe8eefc),
            drafting_edge: rgb(0x4963a8),
        },
    }
}

fn one_dark() -> CodeTheme {
    CodeTheme {
        id: "one-dark".into(),
        label: "One Dark".into(),
        slots: AuthoredSlots {
            paper: rgb(0x282c34),
            added_band: rgb(0x263b32),
            deleted_band: rgb(0x3b3032),
            replaced_band: rgb(0x30384a),
            // Keep the earlier filled mark, darkened enough for One Dark text.
            word_difference: rgb(0x3e4b66),
            line_number: rgb(0xa3aebd),
            default_foreground: rgb(0xabb2bf),
            comment: rgb(0x82c4f5),
            idle_comment: rgb(0xa3aebd),
            selection: Rgba {
                a: 0.20,
                ..rgb(0x61afef)
            },
            search_hit: rgb(0x313020),
            search_current: rgb(0x392e20),
            syntax: syntax_slots(&[
                ("attribute", 0xe5868d),
                ("comment", 0x9da1a8),
                ("comment.documentation", 0x9da1a8),
                ("constant", 0xd19a66),
                ("constant.builtin", 0xd19a66),
                ("constructor", 0xe5c07b),
                ("delimiter", 0xabb2bf),
                ("escape", 0x56b6c2),
                ("function", 0x61afef),
                ("function.macro", 0x61afef),
                ("function.method", 0x61afef),
                ("function.special", 0x61afef),
                ("keyword", 0xcc87e1),
                ("label", 0xe5c07b),
                ("number", 0xd19a66),
                ("operator", 0x56b6c2),
                ("property", 0xe5868d),
                ("punctuation.bracket", 0xabb2bf),
                ("punctuation.delimiter", 0xabb2bf),
                ("string", 0x98c379),
                ("type", 0xe5c07b),
                ("type.builtin", 0xe5c07b),
                ("variable", 0xabb2bf),
                ("variable.builtin", 0xe5868d),
                ("variable.parameter", 0xe5868d),
            ]),
        },
        marks: DiffMarks {
            added_edge: rgb(0x7acb98),
            deleted_edge: rgb(0xdb8e98),
            replaced_edge: rgb(0x96b7ef),
            unchanged_edge: rgb(0xa3acb9),
            omission_fill: rgb(0x343b47),
            omission_wave: rgb(0xaab6c8),
            open_comment_pad: rgb(0x344459),
            drafting_band: rgb(0x303748),
            drafting_edge: rgb(0xa6c5fa),
        },
    }
}

fn atom_one_light() -> CodeTheme {
    CodeTheme {
        id: "atom-one-light".into(),
        label: "Atom One Light".into(),
        slots: AuthoredSlots {
            paper: rgb(0xfafafa),
            added_band: rgb(0xe6f4ea),
            deleted_band: rgb(0xf0e1e1),
            replaced_band: rgb(0xe5eefb),
            word_difference: rgb(0xb8d2f2),
            line_number: rgb(0x59636e),
            default_foreground: rgb(0x242936),
            comment: rgb(0x4078c0),
            idle_comment: rgb(0x59636e),
            selection: Rgba {
                a: 0.18,
                ..rgb(0x4078c0)
            },
            search_hit: rgb(0xfff4c7),
            search_current: rgb(0xffedac),
            syntax: syntax_slots(&[
                ("attribute", 0xa626a4),
                ("comment", 0x5f6770),
                ("comment.documentation", 0x59636e),
                ("constant", 0x895d01),
                ("constant.builtin", 0x895d01),
                ("constructor", 0x005cc5),
                ("delimiter", 0x586069),
                ("escape", 0x005cc5),
                ("function", 0x005cc5),
                ("function.macro", 0x005cc5),
                ("function.method", 0x005cc5),
                ("function.special", 0x005cc5),
                ("keyword", 0xa626a4),
                ("label", 0x005cc5),
                ("number", 0x895d01),
                ("operator", 0x016d9b),
                ("property", 0xaf4238),
                ("punctuation.bracket", 0x586069),
                ("punctuation.delimiter", 0x586069),
                ("string", 0x397238),
                ("type", 0x016d9b),
                ("type.builtin", 0x016d9b),
                ("variable", 0x242936),
                ("variable.builtin", 0xa626a4),
                ("variable.parameter", 0xaf4238),
            ]),
        },
        marks: DiffMarks {
            added_edge: rgb(0x31804a),
            deleted_edge: rgb(0xa04c56),
            replaced_edge: rgb(0x4273c2),
            unchanged_edge: rgb(0x848b95),
            omission_fill: rgb(0xe9ebef),
            omission_wave: rgb(0x6b7482),
            open_comment_pad: rgb(0xedf3fb),
            drafting_band: rgb(0xe9effb),
            drafting_edge: rgb(0x4963a8),
        },
    }
}

/// Light or dark appearance of the application chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftwareThemeMode {
    Light,
    Dark,
}

/// Which Code Theme applies in each Software Theme mode. Absent means unset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeThemePairing {
    pub light: Option<String>,
    pub dark: Option<String>,
}

/// One built-in palette: authored code colors and Diff decoration colors.
#[derive(Clone, Debug)]
pub struct CodeTheme {
    pub id: String,
    pub label: String,
    pub slots: AuthoredSlots,
    pub marks: DiffMarks,
}

/// Code paper, foregrounds, and in-line mark colors authored by a Code Theme.
#[derive(Clone, Debug)]
pub struct AuthoredSlots {
    pub paper: Rgba,
    pub added_band: Rgba,
    pub deleted_band: Rgba,
    pub replaced_band: Rgba,
    pub word_difference: Rgba,
    pub line_number: Rgba,
    pub default_foreground: Rgba,
    pub comment: Rgba,
    pub idle_comment: Rgba,
    pub selection: Rgba,
    pub search_hit: Rgba,
    pub search_current: Rgba,
    pub syntax: Vec<(String, Rgba)>,
}

/// The palette Diff paints from. It is independent of Software Theme colors.
#[derive(Clone, Debug)]
pub struct ResolvedCodeTheme {
    pub id: String,
    /// Display label; asserted in tests, read by pickers through the catalog.
    #[allow(dead_code)]
    pub label: String,
    pub slots: AuthoredSlots,
    pub marks: DiffMarks,
}

impl ResolvedCodeTheme {
    /// Exact capture name, then drop the last dotted segment, else the theme's
    /// default foreground. Not the window chrome text color.
    pub fn capture_color(&self, name: &str) -> Rgba {
        let mut key = name;
        loop {
            if let Some((_, color)) = self.slots.syntax.iter().find(|(n, _)| n == key) {
                return *color;
            }
            match key.rsplit_once('.') {
                Some((prefix, _)) => key = prefix,
                None => return self.slots.default_foreground,
            }
        }
    }
}

/// Diff decorations are authored per palette because shifting light RGB values
/// does not preserve their contrast or status meaning on dark paper.
#[derive(Clone, Debug)]
pub struct DiffMarks {
    pub added_edge: Rgba,
    pub deleted_edge: Rgba,
    pub replaced_edge: Rgba,
    pub unchanged_edge: Rgba,
    pub omission_fill: Rgba,
    pub omission_wave: Rgba,
    pub open_comment_pad: Rgba,
    pub drafting_band: Rgba,
    pub drafting_edge: Rgba,
}

pub fn builtin_catalog() -> &'static [CodeTheme] {
    static CATALOG: LazyLock<Vec<CodeTheme>> =
        LazyLock::new(|| vec![one_light(), atom_one_light(), one_dark()]);
    CATALOG.as_slice()
}

pub fn default_id(mode: SoftwareThemeMode) -> &'static str {
    match mode {
        SoftwareThemeMode::Light => crate::theme_defaults::LIGHT_CODE_THEME_ID,
        SoftwareThemeMode::Dark => crate::theme_defaults::DARK_CODE_THEME_ID,
    }
}

pub fn resolve(
    mode: SoftwareThemeMode,
    pairing: &CodeThemePairing,
    catalog: &[CodeTheme],
) -> ResolvedCodeTheme {
    let choice = match mode {
        SoftwareThemeMode::Light => pairing.light.as_deref(),
        SoftwareThemeMode::Dark => pairing.dark.as_deref(),
    };
    let entry = choice
        .and_then(|id| catalog.iter().find(|theme| theme.id == id))
        .or_else(|| catalog.iter().find(|theme| theme.id == default_id(mode)))
        .expect("catalog includes each Software Theme default");
    ResolvedCodeTheme {
        id: entry.id.clone(),
        label: entry.label.clone(),
        slots: entry.slots.clone(),
        marks: entry.marks.clone(),
    }
}

/// Pairing [`active`] resolves. Absent until [`remember_pairing`].
static REMEMBERED: Mutex<CodeThemePairing> = Mutex::new(CodeThemePairing {
    light: None,
    dark: None,
});
static ACTIVE_MODE: Mutex<SoftwareThemeMode> = Mutex::new(SoftwareThemeMode::Light);

/// Remember the Code Theme Pairing for active Diff rendering.
pub fn remember_pairing(pairing: CodeThemePairing) {
    *REMEMBERED.lock().unwrap_or_else(|err| err.into_inner()) = pairing;
}

pub fn remember_mode(mode: SoftwareThemeMode) {
    *ACTIVE_MODE.lock().unwrap_or_else(|err| err.into_inner()) = mode;
}

pub fn active_mode() -> SoftwareThemeMode {
    *ACTIVE_MODE.lock().unwrap_or_else(|err| err.into_inner())
}

/// Palette Diff paints from the active Software Theme mode and pairing.
pub fn active() -> ResolvedCodeTheme {
    let pairing = REMEMBERED
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let mode = *ACTIVE_MODE.lock().unwrap_or_else(|err| err.into_inner());
    resolve(mode, &pairing, builtin_catalog())
}

/// Capture id → color for `theme`.
///
/// The first call compiles highlight queries. Call it beside the first highlight.
/// A later call only recolors ids that are already compiled.
pub fn capture_colors(theme: &ResolvedCodeTheme) -> Vec<Rgba> {
    crate::syntax::capture_names()
        .iter()
        .map(|name| theme.capture_color(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Rgba) -> (u8, u8, u8) {
        (
            (c.r * 255.).round() as u8,
            (c.g * 255.).round() as u8,
            (c.b * 255.).round() as u8,
        )
    }

    fn assert_hex(c: Rgba, expected: u32) {
        let got = hex(c);
        let expect = (
            ((expected >> 16) & 0xff) as u8,
            ((expected >> 8) & 0xff) as u8,
            (expected & 0xff) as u8,
        );
        assert_eq!(got, expect, "#{expected:06x}");
    }

    #[test]
    fn light_mode_with_no_pairing_resolves_one_light_authored_slots() {
        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing::default(),
            builtin_catalog(),
        );
        assert_eq!(palette.id, "one-light");
        assert_eq!(palette.label, "One Light");
        assert_hex(palette.slots.paper, 0xffffff);
        assert_hex(palette.slots.added_band, 0xe8f7ee);
        assert_hex(palette.slots.deleted_band, 0xe4e6e9);
        assert_hex(palette.slots.replaced_band, 0xe8f0fe);
        assert_hex(palette.slots.word_difference, 0xb9ceee);
        assert_hex(palette.slots.line_number, 0x596579);
        assert_hex(palette.slots.default_foreground, 0x172033);
        assert_hex(palette.slots.comment, 0x2457d6);
        assert_hex(palette.slots.idle_comment, 0x596579);
        assert_hex(palette.slots.selection, 0x2457d6);
        assert!(
            (palette.slots.selection.a - 0.18).abs() < 1e-5,
            "selection alpha {}",
            palette.slots.selection.a
        );
        assert_hex(palette.slots.search_hit, 0xfff4c7);
        assert_hex(palette.slots.search_current, 0xffedac);
        let syntax = [
            ("attribute", 0x4b61b9),
            ("comment", 0x666769),
            ("comment.documentation", 0x65676d),
            ("constant", 0x8b5e00),
            ("constant.builtin", 0x8b5e00),
            ("constructor", 0x4b61b9),
            ("delimiter", 0x4d4f52),
            ("escape", 0x65676d),
            ("function", 0x4961b8),
            ("function.macro", 0x4961b8),
            ("function.method", 0x4961b8),
            ("function.special", 0x4961b8),
            ("keyword", 0x9a44a0),
            ("label", 0x4b61b9),
            ("number", 0x905b1f),
            ("operator", 0x2e6c98),
            ("property", 0xa64b3e),
            ("punctuation.bracket", 0x4d4f52),
            ("punctuation.delimiter", 0x4d4f52),
            ("string", 0x46713d),
            ("type", 0x2e6c98),
            ("type.builtin", 0x2e6c98),
            ("variable", 0x242529),
            ("variable.builtin", 0x242529),
            ("variable.parameter", 0xa64b3e),
        ];
        assert_eq!(palette.slots.syntax.len(), syntax.len());
        for (name, expected) in syntax {
            let color = palette
                .slots
                .syntax
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, c)| *c);
            let Some(color) = color else {
                panic!("missing authored syntax slot {name}");
            };
            assert_hex(color, expected);
        }
    }

    fn one_light_palette() -> ResolvedCodeTheme {
        resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing::default(),
            builtin_catalog(),
        )
    }

    #[test]
    fn builtin_catalog_exposes_atom_one_light_for_picker_switching() {
        let catalog = builtin_catalog();
        assert_eq!(catalog.len(), 3);
        assert_eq!(catalog[1].id, "atom-one-light");
        assert_eq!(catalog[1].label, "Atom One Light");
        assert_eq!(catalog[2].id, "one-dark");

        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing {
                light: Some("atom-one-light".into()),
                dark: None,
            },
            catalog,
        );
        assert_eq!(palette.id, "atom-one-light");
        assert_hex(palette.slots.paper, 0xfafafa);
        assert_hex(palette.slots.default_foreground, 0x242936);
    }

    #[test]
    fn one_light_diff_marks_are_authored() {
        let palette = one_light_palette();
        assert_hex(palette.marks.added_edge, 0x31804a);
        assert_hex(palette.marks.deleted_edge, 0x737b86);
        assert_hex(palette.marks.replaced_edge, 0x4273c2);
        assert_hex(palette.marks.unchanged_edge, 0x888f99);
        assert_hex(palette.marks.omission_fill, 0xe8eaef);
        assert_hex(palette.marks.omission_wave, 0x6b7482);
        assert_hex(palette.marks.open_comment_pad, 0xf1f5ff);
    }

    #[test]
    fn diff_marks_follow_the_chosen_palette() {
        let original = one_light_palette();
        let mut theme = fixture("changed-slots", 0x202020);
        theme.marks.added_edge = rgb(0x90a090);
        theme.marks.deleted_edge = rgb(0x909090);
        theme.marks.replaced_edge = rgb(0x9090a0);
        theme.marks.open_comment_pad = rgb(0x804020);
        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing {
                light: Some(theme.id.clone()),
                dark: None,
            },
            &[theme, one_light()],
        );
        for (changed, previous) in [
            (palette.marks.added_edge, original.marks.added_edge),
            (palette.marks.deleted_edge, original.marks.deleted_edge),
            (palette.marks.replaced_edge, original.marks.replaced_edge),
            (
                palette.marks.open_comment_pad,
                original.marks.open_comment_pad,
            ),
        ] {
            assert_ne!(hex(changed), hex(previous));
        }
    }

    #[test]
    fn explicit_code_theme_has_same_diff_colors_in_either_software_mode() {
        for theme in builtin_catalog() {
            let pairing = CodeThemePairing {
                light: Some(theme.id.clone()),
                dark: Some(theme.id.clone()),
            };
            let light = resolve(SoftwareThemeMode::Light, &pairing, builtin_catalog());
            let dark = resolve(SoftwareThemeMode::Dark, &pairing, builtin_catalog());
            assert_eq!(hex(light.slots.paper), hex(dark.slots.paper));
            assert_eq!(
                hex(light.slots.word_difference),
                hex(dark.slots.word_difference)
            );
            assert_eq!(
                hex(light.marks.omission_wave),
                hex(dark.marks.omission_wave)
            );
            assert_eq!(
                hex(light.marks.drafting_band),
                hex(dark.marks.drafting_band)
            );
            assert_eq!(
                hex(light.capture_color("comment")),
                hex(dark.capture_color("comment"))
            );
        }
    }

    #[test]
    fn capture_lookup_drops_dotted_segments_then_uses_default_foreground() {
        let palette = one_light_palette();
        assert_hex(palette.capture_color("function"), 0x4961b8);
        assert_hex(palette.capture_color("function.method.call"), 0x4961b8);
        assert_hex(palette.capture_color("no.such.capture"), 0x172033);
        assert_hex(palette.capture_color("unknown"), 0x172033);
    }

    #[test]
    fn unknown_capture_uses_the_resolved_themes_own_foreground() {
        use gpui::rgb;
        let mut theme = one_light();
        theme.slots.default_foreground = rgb(0xabcdee);
        theme.slots.syntax.retain(|(name, _)| name == "function");
        theme.slots.syntax[0].1 = rgb(0x112233);
        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing::default(),
            &[theme],
        );
        assert_hex(palette.capture_color("function.method.call"), 0x112233);
        assert_hex(palette.capture_color("not.a.capture"), 0xabcdee);
    }

    #[test]
    fn every_emitted_capture_has_an_explicit_one_light_color() {
        let palette = one_light_palette();
        let names = crate::syntax::capture_names();
        assert!(
            !names.is_empty(),
            "queries should emit at least one capture name"
        );
        for name in names {
            let authored = palette.slots.syntax.iter().find(|(n, _)| n == name);
            let Some((_, color)) = authored else {
                panic!("missing deliberate One Light entry for {name:?}");
            };
            assert_ne!(
                hex(*color),
                hex(palette.slots.default_foreground),
                "{name} would be indistinguishable from the default foreground"
            );
            assert_eq!(hex(palette.capture_color(name)), hex(*color), "{name}");
        }
        assert_hex(palette.capture_color("delimiter"), 0x4d4f52);
    }

    fn relative_luminance(c: Rgba) -> f32 {
        let channel = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }

    fn contrast_ratio(fg: Rgba, bg: Rgba) -> f32 {
        let (a, b) = (relative_luminance(fg), relative_luminance(bg));
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn composite(fg: Rgba, bg: Rgba) -> Rgba {
        Rgba {
            r: fg.r * fg.a + bg.r * (1. - fg.a),
            g: fg.g * fg.a + bg.g * (1. - fg.a),
            b: fg.b * fg.a + bg.b * (1. - fg.a),
            a: 1.,
        }
    }

    #[test]
    fn builtin_syntax_foregrounds_meet_text_contrast_on_actual_fills() {
        let mut failures = Vec::new();
        for theme in builtin_catalog() {
            let slots = &theme.slots;
            let bgs = [
                ("paper", slots.paper),
                ("added band", slots.added_band),
                ("deleted band", slots.deleted_band),
                ("replaced band", slots.replaced_band),
                ("word-difference mark", slots.word_difference),
                ("search hit", slots.search_hit),
                ("current search hit", slots.search_current),
                ("drafting band", theme.marks.drafting_band),
            ];
            for (name, fg) in std::iter::once((&"default".to_string(), &slots.default_foreground))
                .chain(slots.syntax.iter().map(|(name, color)| (name, color)))
            {
                for (bg_name, bg) in bgs {
                    let painted_surfaces = if matches!(
                        bg_name,
                        "paper" | "added band" | "deleted band" | "replaced band" | "drafting band"
                    ) {
                        vec![
                            (bg_name.to_string(), bg),
                            (
                                format!("{bg_name} with selection"),
                                composite(slots.selection, bg),
                            ),
                        ]
                    } else {
                        vec![(bg_name.to_string(), bg)]
                    };
                    for (surface, painted) in painted_surfaces {
                        let ratio = contrast_ratio(*fg, painted);
                        let target = if surface.contains("with selection")
                            || surface == "word-difference mark"
                        {
                            3.
                        } else {
                            4.5
                        };
                        if ratio < target {
                            failures.push(format!("{} {name} on {surface}: {ratio:.2}", theme.id));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn builtin_diff_decoration_marks_are_distinct_on_painted_surfaces() {
        let mut failures = Vec::new();
        for theme in builtin_catalog() {
            let s = &theme.slots;
            let m = &theme.marks;
            let bands = [s.paper, s.added_band, s.deleted_band, s.replaced_band];
            let checks = [
                ("added edge", m.added_edge, s.added_band),
                ("deleted edge", m.deleted_edge, s.deleted_band),
                ("replaced edge", m.replaced_edge, s.replaced_band),
                ("unchanged edge", m.unchanged_edge, s.paper),
                ("omission wave", m.omission_wave, m.omission_fill),
                ("omission wave on paper", m.omission_wave, s.paper),
                ("drafting edge", m.drafting_edge, m.drafting_band),
                ("comment icon on pad", s.comment, m.open_comment_pad),
            ];
            for (name, fg, bg) in checks {
                let ratio = contrast_ratio(fg, bg);
                if ratio < 3. {
                    failures.push(format!("{} {name}: {ratio:.2}", theme.id));
                }
            }
            for (name, fg) in [
                ("line number", s.line_number),
                ("comment marker", s.comment),
                ("idle comment", s.idle_comment),
            ] {
                for (ix, bg) in bands.into_iter().enumerate() {
                    let target = if name == "line number" { 4.5 } else { 3. };
                    let ratio = contrast_ratio(fg, bg);
                    if ratio < target {
                        failures.push(format!("{} {name} on band {ix}: {ratio:.2}", theme.id));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    fn fixture(id: &str, paper: u32) -> CodeTheme {
        use gpui::rgb;
        let mut theme = one_light();
        theme.id = id.into();
        theme.label = id.into();
        theme.slots.paper = rgb(paper);
        theme
    }

    #[test]
    fn light_mode_uses_the_light_choice_and_not_the_dark_one() {
        let catalog = [
            fixture("fixture-light", 0x111111),
            fixture("fixture-dark", 0x222222),
            one_light(),
        ];
        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing {
                light: Some("fixture-light".into()),
                dark: Some("fixture-dark".into()),
            },
            &catalog,
        );
        assert_eq!(palette.id, "fixture-light");
        assert_hex(palette.slots.paper, 0x111111);
    }

    #[test]
    fn dark_mode_uses_the_dark_choice_and_not_the_light_one() {
        let catalog = [
            fixture("fixture-light", 0x111111),
            fixture("fixture-dark", 0x222222),
            one_light(),
        ];
        let palette = resolve(
            SoftwareThemeMode::Dark,
            &CodeThemePairing {
                light: Some("fixture-light".into()),
                dark: Some("fixture-dark".into()),
            },
            &catalog,
        );
        assert_eq!(palette.id, "fixture-dark");
        assert_hex(palette.slots.paper, 0x222222);
    }

    #[test]
    fn absent_choices_resolve_to_their_modes_defaults() {
        let catalog = [fixture("fixture-first", 0x333333), one_light(), one_dark()];
        let light = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing::default(),
            &catalog,
        );
        let dark = resolve(
            SoftwareThemeMode::Dark,
            &CodeThemePairing::default(),
            &catalog,
        );
        assert_eq!(light.id, "one-light");
        assert_hex(light.slots.paper, 0xffffff);
        assert_eq!(dark.id, "one-dark");
        assert_hex(dark.slots.paper, 0x282c34);
    }

    #[test]
    fn an_unknown_id_resolves_to_one_light_not_the_first_catalog_entry() {
        let catalog = [fixture("fixture-first", 0x333333), one_light()];
        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing {
                light: Some("not-a-theme".into()),
                dark: Some("fixture-first".into()),
            },
            &catalog,
        );
        assert_eq!(palette.id, "one-light");
        assert_hex(palette.slots.paper, 0xffffff);
    }

    #[test]
    fn dark_unknown_falls_back_but_explicit_one_light_remains_valid() {
        let catalog = builtin_catalog();
        let pairing = CodeThemePairing {
            light: Some("one-dark".into()),
            dark: Some("future-theme".into()),
        };
        assert_eq!(
            resolve(SoftwareThemeMode::Dark, &pairing, catalog).id,
            "one-dark"
        );
        assert_eq!(
            resolve(SoftwareThemeMode::Light, &pairing, catalog).id,
            "one-dark"
        );
        let explicit = CodeThemePairing {
            dark: Some("one-light".into()),
            ..pairing
        };
        assert_eq!(
            resolve(SoftwareThemeMode::Dark, &explicit, catalog).id,
            "one-light"
        );
    }
}

//! Code Theme resolution. [`resolve`] is the seam Diff paints from.

use std::sync::{LazyLock, Mutex};

use gpui::{Rgba, rgb};

const ONE_LIGHT_SYNTAX: &[(&str, u32)] = &[
    ("attribute", 0x526bcb),
    ("comment", 0x717274),
    ("comment.documentation", 0x6f7178),
    ("constant", 0x966600),
    ("constant.builtin", 0x966600),
    ("constructor", 0x526bcb),
    ("delimiter", 0x4d4f52),
    ("escape", 0x6f7178),
    ("function", 0x516ccc),
    ("function.macro", 0x516ccc),
    ("function.method", 0x516ccc),
    ("function.special", 0x516ccc),
    ("keyword", 0xa449ab),
    ("label", 0x526bcb),
    ("number", 0x9f6522),
    ("operator", 0x3377a8),
    ("property", 0xb55243),
    ("punctuation.bracket", 0x4d4f52),
    ("punctuation.delimiter", 0x4d4f52),
    ("string", 0x4d7c43),
    ("type", 0x3377a8),
    ("type.builtin", 0x3377a8),
    ("variable", 0x242529),
    ("variable.builtin", 0x242529),
    ("variable.parameter", 0xb55243),
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
            deleted_band: rgb(0xd8dce1),
            replaced_band: rgb(0xe8f0fe),
            word_difference: rgb(0xb9ceee),
            line_number: rgb(0x98a2b3),
            default_foreground: rgb(0x172033),
            comment: rgb(0x2457d6),
            idle_comment: rgb(0x596579),
            selection: Rgba {
                a: 0.18,
                ..rgb(0x2457d6)
            },
            search_hit: rgb(0xffe08a),
            search_current: rgb(0xffc53d),
            search_ring: rgb(0xe6a800),
            syntax: syntax_slots(ONE_LIGHT_SYNTAX),
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
            word_difference: rgb(0x52658a),
            line_number: rgb(0x636d83),
            default_foreground: rgb(0xabb2bf),
            comment: rgb(0x61afef),
            idle_comment: rgb(0x636d83),
            selection: Rgba {
                a: 0.25,
                ..rgb(0x61afef)
            },
            search_hit: rgb(0x665c24),
            search_current: rgb(0x9e791e),
            search_ring: rgb(0xe5c07b),
            syntax: syntax_slots(&[
                ("attribute", 0xe06c75),
                ("comment", 0x7f848e),
                ("comment.documentation", 0x7f848e),
                ("constant", 0xd19a66),
                ("constant.builtin", 0xd19a66),
                ("constructor", 0xe5c07b),
                ("delimiter", 0xabb2bf),
                ("escape", 0x56b6c2),
                ("function", 0x61afef),
                ("function.macro", 0x61afef),
                ("function.method", 0x61afef),
                ("function.special", 0x61afef),
                ("keyword", 0xc678dd),
                ("label", 0xe5c07b),
                ("number", 0xd19a66),
                ("operator", 0x56b6c2),
                ("property", 0xe06c75),
                ("punctuation.bracket", 0xabb2bf),
                ("punctuation.delimiter", 0xabb2bf),
                ("string", 0x98c379),
                ("type", 0xe5c07b),
                ("type.builtin", 0xe5c07b),
                ("variable", 0xabb2bf),
                ("variable.builtin", 0xe06c75),
                ("variable.parameter", 0xe06c75),
            ]),
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
            line_number: rgb(0x7f8794),
            default_foreground: rgb(0x242936),
            comment: rgb(0x4078c0),
            idle_comment: rgb(0x8a93a2),
            selection: Rgba {
                a: 0.18,
                ..rgb(0x4078c0)
            },
            search_hit: rgb(0xffe6a3),
            search_current: rgb(0xffc857),
            search_ring: rgb(0xc58900),
            syntax: syntax_slots(&[
                ("attribute", 0xa626a4),
                ("comment", 0x6a737d),
                ("comment.documentation", 0x59636e),
                ("constant", 0x986801),
                ("constant.builtin", 0x986801),
                ("constructor", 0x005cc5),
                ("delimiter", 0x586069),
                ("escape", 0x005cc5),
                ("function", 0x005cc5),
                ("function.macro", 0x005cc5),
                ("function.method", 0x005cc5),
                ("function.special", 0x005cc5),
                ("keyword", 0xa626a4),
                ("label", 0x005cc5),
                ("number", 0x986801),
                ("operator", 0x0184bc),
                ("property", 0xe45649),
                ("punctuation.bracket", 0x586069),
                ("punctuation.delimiter", 0x586069),
                ("string", 0x50a14f),
                ("type", 0x0184bc),
                ("type.builtin", 0x0184bc),
                ("variable", 0x242936),
                ("variable.builtin", 0xa626a4),
                ("variable.parameter", 0xe45649),
            ]),
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

/// One built-in palette: id, label, and authored slots. Derived roles are not stored.
#[derive(Clone, Debug)]
pub struct CodeTheme {
    pub id: String,
    pub label: String,
    pub slots: AuthoredSlots,
}

/// Colors a Code Theme authors. Derived edges and pads are not fields here.
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
    pub search_ring: Rgba,
    pub syntax: Vec<(String, Rgba)>,
}

/// The palette Diff paints from: the chosen entry's authored slots, plus derived roles.
#[derive(Clone, Debug)]
pub struct ResolvedCodeTheme {
    pub id: String,
    /// Display label; asserted in tests, read by pickers through the catalog.
    #[allow(dead_code)]
    pub label: String,
    pub slots: AuthoredSlots,
    pub derived: DerivedRoles,
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

/// Roles computed from authored slots. Not chosen in Settings.
#[derive(Clone, Debug)]
pub struct DerivedRoles {
    pub added_edge: Rgba,
    pub deleted_edge: Rgba,
    pub replaced_edge: Rgba,
    pub unchanged_edge: Rgba,
    pub omission_fill: Rgba,
    pub omission_wave: Rgba,
    pub knockout: Rgba,
    pub open_comment_pad: Rgba,
}

pub fn builtin_catalog() -> &'static [CodeTheme] {
    static CATALOG: LazyLock<Vec<CodeTheme>> =
        LazyLock::new(|| vec![one_light(), atom_one_light(), one_dark()]);
    CATALOG.as_slice()
}

const FALLBACK_ID: &str = "one-light";

/// Derived roles of the authored slots. Does not branch on theme id.
///
/// Preserve the One Light pins while following changes to the source slot.
/// ponytail: calibrated RGB offsets clamp at channel limits; validate these
/// derived colors before shipping another palette.
fn shifted(base: u32, reference: u32, source: Rgba) -> Rgba {
    let channel = |base: u8, reference: u8, source: f32| {
        (f32::from(base) + (source * 255. - f32::from(reference))).clamp(0., 255.) / 255.
    };
    Rgba {
        r: channel((base >> 16) as u8, (reference >> 16) as u8, source.r),
        g: channel((base >> 8) as u8, (reference >> 8) as u8, source.g),
        b: channel(base as u8, reference as u8, source.b),
        a: source.a,
    }
}

/// Derive a pad toward paper, calibrated to One Light's comment and pad colors.
fn comment_pad(comment: Rgba, paper: Rgba) -> Rgba {
    let channel = |comment: f32, paper: f32, reference: u8, pad: u8| {
        let amount = f32::from(pad - reference) / f32::from(255 - reference);
        comment + (paper - comment) * amount
    };
    Rgba {
        r: channel(comment.r, paper.r, 0x24, 0xf1),
        g: channel(comment.g, paper.g, 0x57, 0xf5),
        b: channel(comment.b, paper.b, 0xd6, 0xff),
        a: paper.a,
    }
}

fn derive(slots: &AuthoredSlots) -> DerivedRoles {
    DerivedRoles {
        added_edge: shifted(0x7ccf98, 0xe8f7ee, slots.added_band),
        deleted_edge: shifted(0xa3aab3, 0xd8dce1, slots.deleted_band),
        replaced_edge: shifted(0x7fa6ea, 0xe8f0fe, slots.replaced_band),
        unchanged_edge: shifted(0xdfe3ea, 0xffffff, slots.paper),
        omission_fill: shifted(0xe8eaef, 0xffffff, slots.paper),
        omission_wave: shifted(0x98a2b3, 0xffffff, slots.paper),
        knockout: slots.paper,
        open_comment_pad: comment_pad(slots.comment, slots.paper),
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
        .or_else(|| catalog.iter().find(|theme| theme.id == FALLBACK_ID))
        .expect("catalog includes one-light");
    ResolvedCodeTheme {
        id: entry.id.clone(),
        label: entry.label.clone(),
        slots: entry.slots.clone(),
        derived: derive(&entry.slots),
    }
}

/// Pairing [`active`] resolves. Absent until [`remember_pairing`].
static REMEMBERED: Mutex<CodeThemePairing> = Mutex::new(CodeThemePairing {
    light: None,
    dark: None,
});
static ACTIVE_MODE: Mutex<SoftwareThemeMode> = Mutex::new(SoftwareThemeMode::Light);

/// Remember the Code Theme Pairing. The dark choice stays here while production
/// still resolves in Software Theme mode light.
pub fn remember_pairing(pairing: CodeThemePairing) {
    *REMEMBERED.lock().unwrap_or_else(|err| err.into_inner()) = pairing;
}

pub fn remember_mode(mode: SoftwareThemeMode) {
    *ACTIVE_MODE.lock().unwrap_or_else(|err| err.into_inner()) = mode;
}

pub fn is_dark() -> bool {
    *ACTIVE_MODE.lock().unwrap_or_else(|err| err.into_inner()) == SoftwareThemeMode::Dark
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
        assert_hex(palette.slots.deleted_band, 0xd8dce1);
        assert_hex(palette.slots.replaced_band, 0xe8f0fe);
        assert_hex(palette.slots.word_difference, 0xb9ceee);
        assert_hex(palette.slots.line_number, 0x98a2b3);
        assert_hex(palette.slots.default_foreground, 0x172033);
        assert_hex(palette.slots.comment, 0x2457d6);
        assert_hex(palette.slots.idle_comment, 0x596579);
        assert_hex(palette.slots.selection, 0x2457d6);
        assert!(
            (palette.slots.selection.a - 0.18).abs() < 1e-5,
            "selection alpha {}",
            palette.slots.selection.a
        );
        assert_hex(palette.slots.search_hit, 0xffe08a);
        assert_hex(palette.slots.search_current, 0xffc53d);
        assert_hex(palette.slots.search_ring, 0xe6a800);
        let syntax = [
            ("attribute", 0x526bcb),
            ("comment", 0x717274),
            ("comment.documentation", 0x6f7178),
            ("constant", 0x966600),
            ("constant.builtin", 0x966600),
            ("constructor", 0x526bcb),
            ("delimiter", 0x4d4f52),
            ("escape", 0x6f7178),
            ("function", 0x516ccc),
            ("function.macro", 0x516ccc),
            ("function.method", 0x516ccc),
            ("function.special", 0x516ccc),
            ("keyword", 0xa449ab),
            ("label", 0x526bcb),
            ("number", 0x9f6522),
            ("operator", 0x3377a8),
            ("property", 0xb55243),
            ("punctuation.bracket", 0x4d4f52),
            ("punctuation.delimiter", 0x4d4f52),
            ("string", 0x4d7c43),
            ("type", 0x3377a8),
            ("type.builtin", 0x3377a8),
            ("variable", 0x242529),
            ("variable.builtin", 0x242529),
            ("variable.parameter", 0xb55243),
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
    fn one_light_derived_roles_match_todays_diff_colors() {
        let palette = one_light_palette();
        assert_hex(palette.derived.added_edge, 0x7ccf98);
        assert_hex(palette.derived.deleted_edge, 0xa3aab3);
        assert_hex(palette.derived.replaced_edge, 0x7fa6ea);
        assert_hex(palette.derived.unchanged_edge, 0xdfe3ea);
        assert_hex(palette.derived.omission_fill, 0xe8eaef);
        assert_hex(palette.derived.omission_wave, 0x98a2b3);
        assert_hex(palette.derived.knockout, 0xffffff);
        assert_hex(palette.derived.open_comment_pad, 0xf1f5ff);
    }

    #[test]
    fn derived_roles_follow_the_chosen_palettes_slots() {
        let original = one_light_palette();
        let mut theme = fixture("changed-slots", 0x202020);
        theme.slots.added_band = rgb(0x90a090);
        theme.slots.deleted_band = rgb(0x909090);
        theme.slots.replaced_band = rgb(0x9090a0);
        theme.slots.comment = rgb(0x804020);
        let palette = resolve(
            SoftwareThemeMode::Light,
            &CodeThemePairing {
                light: Some(theme.id.clone()),
                dark: None,
            },
            &[theme, one_light()],
        );
        for (changed, previous) in [
            (palette.derived.added_edge, original.derived.added_edge),
            (palette.derived.deleted_edge, original.derived.deleted_edge),
            (
                palette.derived.replaced_edge,
                original.derived.replaced_edge,
            ),
            (
                palette.derived.unchanged_edge,
                original.derived.unchanged_edge,
            ),
            (
                palette.derived.omission_fill,
                original.derived.omission_fill,
            ),
            (
                palette.derived.omission_wave,
                original.derived.omission_wave,
            ),
            (
                palette.derived.open_comment_pad,
                original.derived.open_comment_pad,
            ),
        ] {
            assert_ne!(hex(changed), hex(previous));
        }
        assert_eq!(hex(palette.derived.knockout), hex(palette.slots.paper));
    }

    #[test]
    fn capture_lookup_drops_dotted_segments_then_uses_default_foreground() {
        let palette = one_light_palette();
        assert_hex(palette.capture_color("function"), 0x516ccc);
        assert_hex(palette.capture_color("function.method.call"), 0x516ccc);
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

    #[test]
    fn one_light_syntax_foregrounds_meet_contrast_on_code_surfaces() {
        let palette = one_light_palette();
        let bgs = [
            ("paper", palette.slots.paper),
            ("added band", palette.slots.added_band),
            ("deleted band", palette.slots.deleted_band),
            ("replaced band", palette.slots.replaced_band),
            ("word-difference mark", palette.slots.word_difference),
            ("search hit", palette.slots.search_hit),
            ("current search hit", palette.slots.search_current),
        ];
        for (name, fg) in &palette.slots.syntax {
            for (bg_name, bg) in bgs {
                let ratio = contrast_ratio(*fg, bg);
                assert!(
                    ratio >= 3.0,
                    "{name} on {bg_name}: contrast {ratio:.2} < 3.0"
                );
            }
        }
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
    fn an_absent_choice_resolves_to_one_light_not_the_first_catalog_entry() {
        let catalog = [fixture("fixture-first", 0x333333), one_light()];
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
        assert_eq!(dark.id, "one-light");
        assert_hex(dark.slots.paper, 0xffffff);
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
}

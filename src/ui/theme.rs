//! Visual tokens aligned with BeadsViewer + accepted prototype A.

use std::sync::LazyLock;

#[cfg(target_os = "windows")]
use gpui::rgba;
use gpui::{BoxShadow, Hsla, Pixels, Rgba, hsla, point, px, rgb};

pub const SIDEBAR_WIDTH: Pixels = gpui::px(188.);
pub const FILES_WIDTH: Pixels = gpui::px(280.);
pub const DIFF_TREE_WIDTH: Pixels = gpui::px(200.);
/// The window titlebar band, and the only chrome row left: it carries the caption
/// buttons, so it is sized to clear them and nothing more. Every pixel it gives up
/// is one the islands below it gain.
pub const TITLEBAR_HEIGHT: Pixels = gpui::px(32.);
pub const TOGGLE_SIZE: Pixels = gpui::px(26.);
/// Every chrome icon; drawn inside a [`TOGGLE_SIZE`] button box.
pub const ICON_SIZE: Pixels = gpui::px(16.);
/// Icons riding inside text-sized controls, such as the settings buttons.
pub const ICON_SIZE_SM: Pixels = gpui::px(14.);
// Font families live in `appearance` (UI Font / Code Font settings).

/// Inset of floating capsules (Changes / Commit / MR detail) from the stage edges.
pub const CHANGES_INSET: f32 = 12.;
/// Inset from the titlebar only. Tighter than [`CHANGES_INSET`] because the
/// titlebar band is frost as well, so a full gap there reads as more titlebar
/// rather than as separation — the two add up to the height the user perceives as
/// chrome. Still non-zero, so the island reads as floating on the desk instead of
/// growing out of the titlebar.
pub const CHANGES_TOP_INSET: f32 = 6.;
/// Gap between neighboring floating capsules — same as the edge inset.
pub const CHANGES_SHADOW_GAP: f32 = CHANGES_INSET;
/// Corner radius of floating capsules.
pub const CHANGES_RADIUS: f32 = 12.;
/// Flex `gap_2` used in chrome rows (traffic lights / toggles / pills).
pub const CHROME_GAP: f32 = 8.;

/// Right inset for the commits column: Changes width + stage inset + gap to Changes.
pub fn changes_float_clearance(files_width: f32) -> f32 {
    files_width + CHANGES_INSET + CHANGES_SHADOW_GAP
}

/// Soft cast for expanded branch/MR pickers — separates the panel from islands below.
/// Includes a zero-offset ambient layer so side edges soften against the frosted desk.
pub fn picker_shadow() -> Vec<BoxShadow> {
    let ink = |a: f32| -> Hsla { hsla(220. / 360., 0.38, 0.14, a) };
    vec![
        BoxShadow {
            color: ink(0.06),
            offset: point(px(0.), px(0.)),
            blur_radius: px(10.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: ink(0.10),
            offset: point(px(0.), px(8.)),
            blur_radius: px(28.),
            spread_radius: px(0.),
        },
    ]
}

#[cfg(target_os = "macos")]
pub const TRAFFIC_LIGHT_TOP_INSET: f32 = 11.;
#[cfg(target_os = "macos")]
pub const TRAFFIC_LIGHT_LEFT_INSET: f32 = 12.;
#[cfg(target_os = "macos")]
pub const TRAFFIC_LIGHTS_WIDTH: f32 = 76.;

pub fn text() -> Rgba {
    rgb(0x172033)
}
pub fn muted() -> Rgba {
    rgb(0x596579)
}
pub fn faint() -> Rgba {
    rgb(0x98a2b3)
}
pub fn line() -> Rgba {
    rgb(0xdfe3ea)
}
pub fn white() -> Rgba {
    rgb(0xffffff)
}
/// Paints nothing; lets whatever is behind the element show through.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const CLEAR: Rgba = Rgba {
    r: 0.,
    g: 0.,
    b: 0.,
    a: 0.,
};

/// The window's frosted material, and the *only* translucent fill in the tree.
///
/// Exactly one element per window may paint it — the window root. Everything
/// between the root and the islands stays [`sidebar`]-clear, because stacked
/// translucent layers compound (0.9 over 0.9 reads as 0.99) and the sidebar
/// would drift lighter than the stage.
#[cfg(target_os = "macos")]
pub fn frost() -> Rgba {
    // The DIY NSVisualEffectView sits *under* the Metal view, so the material is
    // already behind us; painting here would only sit on top of it.
    CLEAR
}

#[cfg(target_os = "windows")]
pub fn frost() -> Rgba {
    // gpui asks Windows for acrylic with a fully clear tint (`AccentPolicy`
    // gradient 0x00000000), so the entire window tone comes from this fill.
    // 0.9 keeps `muted()` legible over a dark wallpaper; below ~0.8 it stops
    // being.
    rgba(0xf4f5f7e6)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn frost() -> Rgba {
    // No compositor backdrop requested (`WindowBackgroundAppearance::Opaque`).
    rgb(0xf4f5f7)
}

/// Columns that sit directly on the frosted desk: the workspace sidebar, the
/// Diff file tree, the stage between islands. Clear wherever [`frost`] is
/// translucent — see its note on compounding.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn sidebar() -> Rgba {
    CLEAR
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn sidebar() -> Rgba {
    rgb(0xf4f5f7)
}
pub fn hover() -> Rgba {
    rgb(0xe9ebef)
}
/// Chrome pill fill (branch / value / unselected kind toggles) — white on the frosted desk.
pub fn capsule() -> Rgba {
    white()
}
/// Hover fill on capsule chrome (prototype `--capsule-bg-hover`).
pub fn capsule_track_hover() -> Rgba {
    rgb(0xe4e7ed)
}
/// Sidebar active row — same blue as the app icon; chroma survives vibrancy.
pub fn sidebar_selected() -> Rgba {
    accent()
}
/// Primary label on [`sidebar_selected`].
pub fn on_sidebar_selected() -> Rgba {
    white()
}
pub fn range() -> Rgba {
    rgb(0xf1f5ff)
}
pub fn accent() -> Rgba {
    rgb(0x2457d6)
}
pub fn mod_bg() -> Rgba {
    rgb(0xe8f0fe)
}
/// Darker span inside a Replace line for tokens that differ (prototype `--chg`).
pub fn mod_chg() -> Rgba {
    rgb(0xb9ceee)
}
pub fn add_bg() -> Rgba {
    rgb(0xe8f7ee)
}
pub fn del_bg() -> Rgba {
    // Prototype delete tint: grey, same color the connector uses.
    rgb(0xd8dce1)
}
pub fn gap_bg() -> Rgba {
    rgb(0xe8eaef)
}

// Settings tokens: Zed One Light roles, tuned to the palette above. [`line`]
// plays One Light's `border`.

/// Row dividers and card outlines — a step lighter than [`line`].
pub fn border_variant() -> Rgba {
    rgb(0xe8ebf0)
}
/// Focused input / keyboard-focused control border; a softened [`accent`].
pub fn border_focused() -> Rgba {
    rgb(0x7c9be6)
}
/// Pressed controls and selected rows; one step past [`hover`].
pub fn element_active() -> Rgba {
    rgb(0xdfe3e9)
}
pub fn success() -> Rgba {
    rgb(0x2f8f55)
}
pub fn success_background() -> Rgba {
    add_bg()
}
pub fn success_border() -> Rgba {
    rgb(0xc3e5cf)
}
pub fn error() -> Rgba {
    rgb(0xcf4a3c)
}
pub fn error_background() -> Rgba {
    rgb(0xfdeceb)
}
pub fn error_border() -> Rgba {
    rgb(0xf5c8c2)
}
/// Settings nav row under the pointer: a half-step toward the white
/// [`capsule`] that marks the selected row, so hover never reads as selection.
pub fn settings_nav_hover() -> Rgba {
    Rgba { a: 0.55, ..white() }
}

// --- Diff syntax palette (ADR-0010, issue 02) ---------------------------------

/// Capture-name → color, re-typed from Zed One Light `syntax` roles and tuned
/// so each stays ≥ [`SYNTAX_MIN_CONTRAST`] against add/del/mod/chg backgrounds.
/// Every name the v1 queries emit has an explicit row (including bare
/// `delimiter`, which drop-last-segment cannot map to `punctuation`).
const SYNTAX_PALETTE: &[(&str, u32)] = &[
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

/// WCAG contrast floor for syntax fg on Diff row / word-mark backgrounds.
const SYNTAX_MIN_CONTRAST: f32 = 3.0;

/// Exact name, then drop the last dotted segment repeatedly; else [`text`].
pub fn resolve_syntax_color(name: &str) -> Rgba {
    let mut key = name;
    loop {
        if let Some((_, hex)) = SYNTAX_PALETTE.iter().find(|(n, _)| *n == key) {
            return rgb(*hex);
        }
        match key.rsplit_once('.') {
            Some((prefix, _)) => key = prefix,
            None => return text(),
        }
    }
}

/// Colors indexed by [`crate::syntax::CaptureId`], matching [`crate::syntax::capture_names`].
///
/// Resolved lazily on first call — do not touch from theme build or first paint
/// on the UI thread. Compiling the highlight queries (~50–150 ms) happens when
/// `capture_names()` runs; call this beside the first highlight result (or on
/// the background path) instead.
pub fn syntax_colors() -> &'static [Rgba] {
    &SYNTAX_COLORS
}

pub fn syntax_color(id: crate::syntax::CaptureId) -> Rgba {
    syntax_colors()[usize::from(id.0)]
}

// LazyLock runs its closure only on first deref, not at module init.
static SYNTAX_COLORS: LazyLock<Vec<Rgba>> = LazyLock::new(|| {
    crate::syntax::capture_names()
        .iter()
        .map(|name| resolve_syntax_color(name))
        .collect()
});

#[cfg(test)]
mod syntax_palette_tests {
    use super::*;
    use crate::syntax::{self, CaptureId};

    fn rgba_u8(c: Rgba) -> (u8, u8, u8) {
        (
            (c.r * 255.).round() as u8,
            (c.g * 255.).round() as u8,
            (c.b * 255.).round() as u8,
        )
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
    fn fallback_drops_dotted_segments_then_text() {
        assert_eq!(
            rgba_u8(resolve_syntax_color("function.method.call")),
            rgba_u8(resolve_syntax_color("function"))
        );
        assert_eq!(
            rgba_u8(resolve_syntax_color("no.such.capture")),
            rgba_u8(text())
        );
        assert_eq!(rgba_u8(resolve_syntax_color("unknown")), rgba_u8(text()));
    }

    #[test]
    fn every_capture_name_has_an_explicit_palette_entry() {
        let names = syntax::capture_names();
        assert!(
            !names.is_empty(),
            "queries should emit at least one capture name"
        );
        for name in names {
            assert!(
                SYNTAX_PALETTE.iter().any(|(n, _)| *n == name.as_str()),
                "missing deliberate palette entry for {name:?} (would only reach color via fallback)"
            );
        }
        assert!(
            SYNTAX_PALETTE.iter().any(|(n, _)| *n == "delimiter"),
            "bare delimiter must be explicit; drop-last-segment never reaches punctuation"
        );
    }

    #[test]
    fn resolved_vector_matches_capture_ids() {
        let names = syntax::capture_names();
        let colors = syntax_colors();
        assert_eq!(colors.len(), names.len());
        for (i, name) in names.iter().enumerate() {
            let id = CaptureId(i as u16);
            assert_eq!(
                rgba_u8(syntax_color(id)),
                rgba_u8(resolve_syntax_color(name)),
                "{name}"
            );
        }
    }

    #[test]
    fn palette_colors_meet_contrast_on_diff_backgrounds() {
        let bgs = [add_bg(), del_bg(), mod_bg(), mod_chg()];
        let bg_names = ["add_bg", "del_bg", "mod_bg", "mod_chg"];
        for &(name, hex) in SYNTAX_PALETTE {
            let fg = rgb(hex);
            for (bg, bg_name) in bgs.iter().zip(bg_names) {
                let ratio = contrast_ratio(fg, *bg);
                assert!(
                    ratio >= SYNTAX_MIN_CONTRAST,
                    "{name} on {bg_name}: contrast {ratio:.2} < {SYNTAX_MIN_CONTRAST}"
                );
            }
        }
    }
}

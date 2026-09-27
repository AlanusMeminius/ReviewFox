//! Visual tokens aligned with BeadsViewer + accepted prototype A.

use gpui::{BoxShadow, Hsla, Pixels, Rgba, hsla, point, px, rgb};
#[cfg(target_os = "windows")]
use gpui::rgba;

pub const SIDEBAR_WIDTH: Pixels = gpui::px(188.);
pub const FILES_WIDTH: Pixels = gpui::px(280.);
pub const DIFF_TREE_WIDTH: Pixels = gpui::px(200.);
/// The window titlebar band, and the only chrome row left: it carries the caption
/// buttons, so it is sized to clear them and nothing more. Every pixel it gives up
/// is one the islands below it gain.
pub const TITLEBAR_HEIGHT: Pixels = gpui::px(32.);
pub const TOGGLE_SIZE: Pixels = gpui::px(26.);
/// Prototype A fonts; OS falls back if not installed.
pub const UI_FONT: &str = "IBM Plex Sans";
pub const MONO_FONT: &str = "IBM Plex Mono";

/// Diff line numbers. System monospace, so a missing Plex install cannot
/// fall back to a proportional font and wrap the digits.
pub fn line_number_font() -> &'static str {
    system_mono_font()
}

fn system_mono_font() -> &'static str {
    if cfg!(windows) {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        MONO_FONT
    }
}

static CODE_FONT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Pick the Diff code font once at startup: Plex Mono when installed, else the
/// system monospace. The OS fallback for a missing Plex is proportional, which
/// breaks column alignment and tab stops.
pub fn init_code_font(installed: &[String]) {
    let pick = if installed.iter().any(|name| name == MONO_FONT) {
        MONO_FONT
    } else {
        system_mono_font()
    };
    let _ = CODE_FONT.set(pick);
}

/// Diff code text font; see [`init_code_font`].
pub fn code_font() -> &'static str {
    CODE_FONT.get().copied().unwrap_or(MONO_FONT)
}

/// Inset of floating capsules (Changes / Commit / MR detail) from the stage edges.
pub const CHANGES_INSET: f32 = 12.;
/// Inset from the titlebar only. Tighter than [`CHANGES_INSET`] because the
/// titlebar band is frost as well, so a full gap there reads as more titlebar
/// rather than as separation — the two add up to the height the user perceives as
/// chrome. Still non-zero, so the island reads as floating on the desk instead of
/// growing out of the titlebar.
pub const CHANGES_TOP_INSET: f32 = 4.;
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
/// Chrome pill fill (branch / MR toggles) — white on the frosted desk.
pub fn capsule() -> Rgba {
    white()
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

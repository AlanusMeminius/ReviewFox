//! Visual tokens aligned with BeadsViewer + accepted prototype A.

use gpui::{BoxShadow, Hsla, Pixels, Rgba, hsla, point, px, rgb};

pub const SIDEBAR_WIDTH: Pixels = gpui::px(220.);
pub const FILES_WIDTH: Pixels = gpui::px(280.);
pub const DIFF_TREE_WIDTH: Pixels = gpui::px(200.);
pub const CHROME_HEIGHT: Pixels = gpui::px(36.);
pub const TOGGLE_SIZE: Pixels = gpui::px(26.);
pub const GUTTER_WIDTH: Pixels = gpui::px(64.);
pub const ROW_HEIGHT: Pixels = gpui::px(20.);

/// Inset of the floating Changes capsule from the stage edges.
pub const CHANGES_INSET: f32 = 8.;
/// Gap between commits/scrollbar and the capsule — same as the right-edge inset.
pub const CHANGES_SHADOW_GAP: f32 = CHANGES_INSET;
/// Corner radius of the floating Changes capsule.
pub const CHANGES_RADIUS: f32 = 12.;

/// Right inset for the commits column: capsule width + stage inset + matching left gap.
pub fn changes_float_clearance(files_width: f32) -> f32 {
    files_width + CHANGES_INSET + CHANGES_SHADOW_GAP
}

/// Soft all-around cast — edge via shadow only (no hard border). See prototype/capsule-changes.html.
pub fn changes_capsule_shadow() -> Vec<BoxShadow> {
    let ink = |a: f32| -> Hsla { hsla(220. / 360., 0.38, 0.14, a) };
    vec![
        BoxShadow {
            color: ink(0.08),
            offset: point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: ink(0.12),
            offset: point(px(0.), px(12.)),
            blur_radius: px(32.),
            spread_radius: px(0.),
        },
    ]
}

/// Prototype A fonts; OS falls back if not installed.
pub const UI_FONT: &str = "IBM Plex Sans";
pub const MONO_FONT: &str = "IBM Plex Mono";


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
pub fn sidebar() -> Rgba {
    // ponytail: opaque stand-in for BeadsViewer frosted rgba; blur comes from window
    rgb(0xf4f5f7)
}
pub fn hover() -> Rgba {
    rgb(0xe9ebef)
}
pub fn capsule() -> Rgba {
    rgb(0xe5e7eb)
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
pub fn add_bg() -> Rgba {
    rgb(0xe8f7ee)
}
pub fn del_bg() -> Rgba {
    rgb(0xfdeceb)
}
pub fn gap_bg() -> Rgba {
    // ponytail: flat stand-in for prototype diagonal hatch
    rgb(0xe8eaef)
}

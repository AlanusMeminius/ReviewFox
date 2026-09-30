//! Visual tokens aligned with BeadsViewer + accepted prototype A.

#[cfg(target_os = "windows")]
use gpui::rgba;
use gpui::{BoxShadow, Hsla, Pixels, Rgba, hsla, point, px, rgb};

use super::code_theme::{self, SoftwareThemeMode};

/// Semantic application colors. New screens can migrate one visible slice at a
/// time while both built-in appearances keep the same set of roles.
#[derive(Clone, Copy)]
pub struct SoftwarePalette {
    pub surface: SurfaceColors,
    pub text: TextColors,
    pub sidebar_row: SidebarRowColors,
}

#[derive(Clone, Copy)]
pub struct SurfaceColors {
    pub window_backing: Rgba,
    /// Stable backing over the platform's frosted material. A small amount of
    /// the material remains visible without letting the wallpaper set contrast.
    pub sidebar_backing: Rgba,
}

#[derive(Clone, Copy)]
pub struct TextColors {
    pub primary: Rgba,
    pub secondary: Rgba,
    pub section: Rgba,
}

#[derive(Clone, Copy)]
pub struct SidebarRowColors {
    pub idle_indicator: Rgba,
    pub hover_indicator: Rgba,
    pub pressed_indicator: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub selected: Rgba,
    pub selected_hover: Rgba,
    pub selected_pressed: Rgba,
    pub selection_indicator: Rgba,
}

/// Pure resolver for the built-in Software Theme palettes.
pub fn resolve_software_palette(mode: SoftwareThemeMode) -> SoftwarePalette {
    match mode {
        SoftwareThemeMode::Light => SoftwarePalette {
            surface: SurfaceColors {
                window_backing: window_backing(mode),
                sidebar_backing: gpui::rgba(0xf4f5f7e6),
            },
            text: TextColors {
                primary: rgb(0x172033),
                secondary: rgb(0x596579),
                section: rgb(0x4b5669),
            },
            sidebar_row: SidebarRowColors {
                idle_indicator: gpui::rgba(0x00000000),
                hover_indicator: rgb(0x596579),
                pressed_indicator: rgb(0x2457d6),
                hover: rgb(0xe9ebef),
                pressed: rgb(0xdde3ec),
                selected: rgb(0xdce8f8),
                selected_hover: rgb(0xd1e1f7),
                selected_pressed: rgb(0xc3d9f4),
                selection_indicator: rgb(0x2457d6),
            },
        },
        SoftwareThemeMode::Dark => SoftwarePalette {
            surface: SurfaceColors {
                window_backing: window_backing(mode),
                sidebar_backing: gpui::rgba(0x1d202780),
            },
            text: TextColors {
                primary: rgb(0xd5dae3),
                secondary: rgb(0xaeb7c5),
                section: rgb(0xe2e6ed),
            },
            sidebar_row: SidebarRowColors {
                idle_indicator: gpui::rgba(0x00000000),
                hover_indicator: rgb(0xaeb7c5),
                pressed_indicator: rgb(0x89c7f7),
                hover: rgb(0x343b47),
                pressed: rgb(0x414a59),
                selected: rgb(0x344a65),
                selected_hover: rgb(0x3b5574),
                selected_pressed: rgb(0x435f81),
                selection_indicator: rgb(0x89c7f7),
            },
        },
    }
}

pub fn software_palette() -> SoftwarePalette {
    resolve_software_palette(if code_theme::is_dark() {
        SoftwareThemeMode::Dark
    } else {
        SoftwareThemeMode::Light
    })
}

#[cfg(target_os = "macos")]
fn window_backing(mode: SoftwareThemeMode) -> Rgba {
    // NSVisualEffectView sits under Metal. Light needs no extra root tint;
    // dark tint controls the otherwise system-light material.
    match mode {
        SoftwareThemeMode::Light => CLEAR,
        SoftwareThemeMode::Dark => gpui::rgba(0x14161db3),
    }
}

#[cfg(target_os = "windows")]
fn window_backing(mode: SoftwareThemeMode) -> Rgba {
    // Windows acrylic has a clear native tint; the root supplies its tone.
    match mode {
        SoftwareThemeMode::Light => rgba(0xf4f5f7e6),
        SoftwareThemeMode::Dark => rgba(0x1d2027e0),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn window_backing(mode: SoftwareThemeMode) -> Rgba {
    // No compositor backdrop is requested on other platforms.
    match mode {
        SoftwareThemeMode::Light => rgb(0xf4f5f7),
        SoftwareThemeMode::Dark => rgb(0x1d2027),
    }
}

pub const SIDEBAR_WIDTH: Pixels = gpui::px(188.);
/// Workspace sidebar row (repos + Settings).
pub const SIDEBAR_ROW_HEIGHT: f32 = 32.;
/// Vertical gap between rows, so neighbouring selected / hover capsules never touch.
pub const SIDEBAR_ROW_GAP: f32 = 4.;
/// Capsule inset from the sidebar's left and right edges.
pub const SIDEBAR_ROW_INSET: f32 = 8.;
/// Inner pad inside the capsule so the icon's left edge sits at 14 (inset + this).
pub const SIDEBAR_ROW_PAD_X: f32 = 6.;
/// Selected row's blue edge grows with interaction; matching pad adjustments
/// keep the icon and label fixed at their regular positions.
pub const SIDEBAR_ROW_INDICATOR_IDLE_WIDTH: f32 = 2.;
pub const SIDEBAR_ROW_INDICATOR_HOVER_WIDTH: f32 = 4.;
pub const SIDEBAR_ROW_INDICATOR_PRESSED_WIDTH: f32 = 6.;
pub const SIDEBAR_ROW_RADIUS: f32 = 8.;
/// Icon is 16 wide starting at 14; this gap puts the label at 34.
pub const SIDEBAR_ICON_LABEL_GAP: f32 = 4.;
pub const SIDEBAR_SECTION_HEIGHT: f32 = 24.;
/// Extra top margin on every section header except the first.
pub const SIDEBAR_SECTION_GAP: f32 = 12.;
pub const SIDEBAR_SCROLL_PAD_TOP: f32 = 16.;
pub const SIDEBAR_SETTINGS_PAD_BOTTOM: f32 = 8.;
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

/// Left inset of a sidebar-adjacent island, measured inside the stage.
/// The titlebar pills use the same value, so the island stays under the kind track.
///
/// Rail open: [`CHANGES_SHADOW_GAP`] is the seam outside the stage, so the inset
/// is 0. Rail collapsed: the parked handle occupies that gutter inside the stage,
/// so the inset is [`CHANGES_INSET`].
pub fn left_island_inset(rail_open: bool) -> f32 {
    if rail_open { 0. } else { CHANGES_INSET }
}

/// Corner radius of floating capsules.
pub const CHANGES_RADIUS: f32 = 12.;

/// Default width of the right-hand Diff comment island.
pub const COMMENT_ISLAND_WIDTH: f32 = 268.;
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

/// Floating Diff find bar (prototype B): shadow-only edge. Painted as a
/// window-root overlay *after* DualPane so the drop shadow sits on the code.
pub fn find_bar_shadow() -> Vec<BoxShadow> {
    let ink = |a: f32| -> Hsla { hsla(220. / 360., 0.38, 0.14, a) };
    vec![
        BoxShadow {
            color: ink(0.08),
            offset: point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: ink(0.14),
            offset: point(px(0.), px(8.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
        },
    ]
}

/// Translucent fill for the floating find bar / draft dock.
pub fn find_bar_bg() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        gpui::rgba(0x272b33f5)
    } else {
        Rgba {
            r: 1.,
            g: 1.,
            b: 1.,
            a: 0.96,
        }
    }
}

/// Outer capsule radius. Field radius is [`FIND_FIELD_RADIUS`] so corners stay
/// concentric with the pad ([`FIND_BAR_PAD`]) at 45°.
pub const FIND_BAR_RADIUS: f32 = 8.;
/// Equal inset of the find bar from the Diff island on top / left / right.
pub const FIND_BAR_INSET: f32 = 6.;
/// Equal pad inside the capsule on top / left / right (and bottom).
pub const FIND_BAR_PAD: f32 = 4.;
/// Inner input radius = outer − pad (parallel arcs).
pub const FIND_FIELD_RADIUS: f32 = FIND_BAR_RADIUS - FIND_BAR_PAD;
pub const FIND_FIELD_HEIGHT: f32 = 28.;
pub const FIND_BAR_HEIGHT: f32 = FIND_FIELD_HEIGHT + 2. * FIND_BAR_PAD;
/// Body top pad when find is open: clears the capsule so line 1 stays reachable.
pub const FIND_CONTENT_PAD: f32 = FIND_BAR_INSET + FIND_BAR_HEIGHT;

// The bottom draft dock wears the find bar's chrome (inset / radius / pad /
// shadow / translucent fill); only its height differs, because it stacks a
// meta row over a taller body field.

/// Meta row above the draft field: `DraftComment · postimage L3–5` plus the key hints.
pub const DRAFT_META_HEIGHT: f32 = 22.;
/// Gap between the dock's meta row and its field.
pub const DRAFT_DOCK_GAP: f32 = 6.;
/// Draft body field — taller than [`FIND_FIELD_HEIGHT`]: comments are prose.
pub const DRAFT_FIELD_HEIGHT: f32 = 52.;
pub const DRAFT_DOCK_HEIGHT: f32 =
    DRAFT_META_HEIGHT + DRAFT_DOCK_GAP + DRAFT_FIELD_HEIGHT + 2. * FIND_BAR_PAD;
/// Body bottom pad while drafting; mirror of [`FIND_CONTENT_PAD`].
#[allow(dead_code)] // Reserved when we add find-style bottom scroll clearance.
pub const DRAFT_CONTENT_PAD: f32 = FIND_BAR_INSET + DRAFT_DOCK_HEIGHT;

/// Draft body grows with visual lines (22px row + pad), never below [`DRAFT_FIELD_HEIGHT`].
pub fn draft_field_height(line_count: usize) -> f32 {
    let rows = line_count.max(1) as f32;
    (rows * 22. + 8.).max(DRAFT_FIELD_HEIGHT)
}

pub fn draft_dock_height(line_count: usize) -> f32 {
    DRAFT_META_HEIGHT + DRAFT_DOCK_GAP + draft_field_height(line_count) + 2. * FIND_BAR_PAD
}

#[allow(dead_code)]
pub fn draft_content_pad(line_count: usize) -> f32 {
    FIND_BAR_INSET + draft_dock_height(line_count)
}

#[cfg(target_os = "macos")]
pub const TRAFFIC_LIGHT_TOP_INSET: f32 = 11.;
#[cfg(target_os = "macos")]
pub const TRAFFIC_LIGHT_LEFT_INSET: f32 = 12.;
#[cfg(target_os = "macos")]
pub const TRAFFIC_LIGHTS_WIDTH: f32 = 76.;

pub fn text() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0xabb2bf)
    } else {
        rgb(0x172033)
    }
}
pub fn muted() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x8f96a3)
    } else {
        rgb(0x596579)
    }
}
/// Splitter stadium chrome: [`muted`] at the given alpha (idle / hover / drag).
pub fn splitter_capsule(alpha: f32) -> Rgba {
    Rgba {
        a: alpha,
        ..muted()
    }
}
pub fn faint() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x636d83)
    } else {
        rgb(0x98a2b3)
    }
}
pub fn line() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x3b4048)
    } else {
        rgb(0xdfe3ea)
    }
}
pub fn white() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x30343d)
    } else {
        rgb(0xffffff)
    }
}
/// Paints nothing; lets whatever is behind the element show through.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const CLEAR: Rgba = Rgba {
    r: 0.,
    g: 0.,
    b: 0.,
    a: 0.,
};

/// The window-wide frosted tint. Exactly one element per window paints it: the
/// root. The workspace sidebar adds a bounded backing for text; other columns
/// on the desk remain clear so their tint does not compound over this fill.
pub fn frost() -> Rgba {
    software_palette().surface.window_backing
}

/// Columns that sit directly on the frosted desk, such as the Diff file tree
/// and stage between islands. The workspace sidebar uses the semantic
/// `surface.sidebar_backing` role because its labels need a stable backdrop.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn sidebar() -> Rgba {
    // Columns on the frosted desk stay clear; in dark the desk is already
    // tinted by [`frost`].
    CLEAR
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn sidebar() -> Rgba {
    rgb(0xf4f5f7)
}
pub fn hover() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x3b4048)
    } else {
        rgb(0xe9ebef)
    }
}
/// Chrome pill fill (branch / value / unselected kind toggles) — white on the frosted desk.
pub fn capsule() -> Rgba {
    white()
}
/// Hover fill on capsule chrome (prototype `--capsule-bg-hover`).
pub fn capsule_track_hover() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x454b55)
    } else {
        rgb(0xe4e7ed)
    }
}
/// Sidebar active row — same blue as the app icon; chroma survives vibrancy.
pub fn sidebar_selected() -> Rgba {
    accent()
}
/// Primary label on [`sidebar_selected`].
pub fn on_sidebar_selected() -> Rgba {
    // Primary label on the accent fill: true white in both themes.
    rgb(0xffffff)
}
/// Workspace sidebar selected / pressed row fill. Translucent so macOS vibrancy
/// (and Windows acrylic) shows through.
pub fn sidebar_row_selected() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.07,
    }
}
/// Workspace sidebar hover fill on a non-selected row.
pub fn sidebar_row_hover() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.04,
    }
}
pub fn range() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x30384a)
    } else {
        rgb(0xf1f5ff)
    }
}
pub fn accent() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x61afef)
    } else {
        rgb(0x2457d6)
    }
}

// Settings tokens: Zed One Light roles, tuned to the palette above. [`line`]
// plays One Light's `border`.

/// Row dividers and card outlines — a step lighter than [`line`].
pub fn border_variant() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x333842)
    } else {
        rgb(0xe8ebf0)
    }
}
/// Focused input / keyboard-focused control border; a softened [`accent`].
pub fn border_focused() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x4c739e)
    } else {
        rgb(0x7c9be6)
    }
}
/// Pressed controls and selected rows; one step past [`hover`].
pub fn element_active() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x454b55)
    } else {
        rgb(0xdfe3e9)
    }
}
pub fn success() -> Rgba {
    rgb(0x2f8f55)
}
pub fn success_background() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x1e3a2a)
    } else {
        rgb(0xe8f7ee)
    }
}
pub fn success_border() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x2f5c40)
    } else {
        rgb(0xc3e5cf)
    }
}
pub fn error() -> Rgba {
    rgb(0xcf4a3c)
}
pub fn error_background() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x3d2422)
    } else {
        rgb(0xfdeceb)
    }
}
pub fn error_border() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x5c3532)
    } else {
        rgb(0xf5c8c2)
    }
}
/// Settings nav row under the pointer: a half-step toward the white
/// [`capsule`] that marks the selected row, so hover never reads as selection.
pub fn settings_nav_hover() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        Rgba {
            a: 0.12,
            ..rgb(0xffffff)
        }
    } else {
        Rgba { a: 0.55, ..white() }
    }
}

#[cfg(test)]
mod inset_tests {
    use super::*;

    #[test]
    fn left_island_inset_follows_the_rail() {
        assert_eq!(left_island_inset(true), 0.);
        assert_eq!(left_island_inset(false), CHANGES_INSET);
    }
}

#[cfg(test)]
mod software_palette_tests {
    use super::*;

    fn composite(foreground: Rgba, background: Rgba) -> Rgba {
        let a = foreground.a;
        Rgba {
            r: foreground.r * a + background.r * (1. - a),
            g: foreground.g * a + background.g * (1. - a),
            b: foreground.b * a + background.b * (1. - a),
            a: 1.,
        }
    }

    fn luminance(color: Rgba) -> f32 {
        let linear = |channel: f32| {
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }

    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let (light, dark) = if luminance(a) >= luminance(b) {
            (luminance(a), luminance(b))
        } else {
            (luminance(b), luminance(a))
        };
        (light + 0.05) / (dark + 0.05)
    }

    #[test]
    fn sidebar_text_survives_light_and_dark_wallpapers_and_row_states() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let palette = resolve_software_palette(mode);
            for wallpaper in [rgb(0x000000), rgb(0xffffff)] {
                let window = composite(palette.surface.window_backing, wallpaper);
                let backing = composite(palette.surface.sidebar_backing, window);
                let states = [
                    ("idle", backing),
                    ("hover", palette.sidebar_row.hover),
                    ("pressed", palette.sidebar_row.pressed),
                    ("selected", palette.sidebar_row.selected),
                    ("selected hover", palette.sidebar_row.selected_hover),
                    ("selected pressed", palette.sidebar_row.selected_pressed),
                ];
                assert!(
                    contrast(palette.text.section, backing) >= 4.5,
                    "{mode:?} section label on {wallpaper:?}"
                );
                for (name, surface) in states {
                    assert!(
                        contrast(palette.text.primary, surface) >= 4.5,
                        "{mode:?} primary text on {name}: {}",
                        contrast(palette.text.primary, surface)
                    );
                    assert!(
                        contrast(palette.text.secondary, surface) >= 3.,
                        "{mode:?} icon on {name}: {}",
                        contrast(palette.text.secondary, surface)
                    );
                }
                for surface in [
                    palette.sidebar_row.selected,
                    palette.sidebar_row.selected_hover,
                    palette.sidebar_row.selected_pressed,
                ] {
                    assert!(
                        contrast(palette.sidebar_row.selection_indicator, surface) >= 3.,
                        "{mode:?} selected row indicator: {}",
                        contrast(palette.sidebar_row.selection_indicator, surface)
                    );
                }
                for (name, indicator, surface) in [
                    (
                        "hover",
                        palette.sidebar_row.hover_indicator,
                        palette.sidebar_row.hover,
                    ),
                    (
                        "pressed",
                        palette.sidebar_row.pressed_indicator,
                        palette.sidebar_row.pressed,
                    ),
                ] {
                    assert!(
                        contrast(indicator, surface) >= 3.,
                        "{mode:?} {name} row indicator: {}",
                        contrast(indicator, surface)
                    );
                }
            }
        }
    }

    #[test]
    fn sidebar_row_states_have_distinct_fills() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let rows = resolve_software_palette(mode).sidebar_row;
            let colors = [
                rows.hover,
                rows.pressed,
                rows.selected,
                rows.selected_hover,
                rows.selected_pressed,
            ];
            for (index, color) in colors.iter().enumerate() {
                assert!(colors[index + 1..].iter().all(|other| color != other));
            }
        }
    }

    #[test]
    fn selected_row_indicator_grows_on_hover_and_press_without_moving_content() {
        let widths = [
            SIDEBAR_ROW_INDICATOR_IDLE_WIDTH,
            SIDEBAR_ROW_INDICATOR_HOVER_WIDTH,
            SIDEBAR_ROW_INDICATOR_PRESSED_WIDTH,
        ];
        assert_eq!(widths, [2., 4., 6.]);
        for pair in widths.windows(2) {
            assert!(pair[1] - pair[0] >= 2.);
        }
        for width in widths {
            assert!(width <= SIDEBAR_ROW_PAD_X);
            assert_eq!(width + (SIDEBAR_ROW_PAD_X - width), SIDEBAR_ROW_PAD_X);
        }
    }
}

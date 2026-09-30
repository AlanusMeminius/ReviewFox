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
    pub field: FieldColors,
    pub scrollbar: ScrollbarColors,
    pub sidebar_row: SidebarRowColors,
    pub tree: TreeColors,
    pub settings: SettingsColors,
    pub control: ControlColors,
    pub tooltip: TooltipColors,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub window_control: WindowControlColors,
    pub feedback: FeedbackColors,
    pub metadata: MetadataColors,
    pub markdown: MarkdownColors,
}

#[derive(Clone, Copy)]
pub struct StatusColors {
    pub foreground: Rgba,
    pub background: Rgba,
    pub border: Rgba,
}

#[derive(Clone, Copy)]
pub struct FeedbackColors {
    pub success: StatusColors,
    pub error: StatusColors,
    pub warning: StatusColors,
    pub info: StatusColors,
    pub neutral: StatusColors,
}

#[derive(Clone, Copy)]
pub struct MetadataColors {
    pub surface: Rgba,
    pub label: Rgba,
    pub text: Rgba,
    pub copy_hover: Rgba,
    pub copy_pressed: Rgba,
    pub row_hover: Rgba,
    pub row_pressed: Rgba,
    pub range: Rgba,
    pub range_hover: Rgba,
    pub range_pressed: Rgba,
    pub row_indicator: Rgba,
    pub range_indicator: Rgba,
    pub link: Rgba,
}

#[derive(Clone, Copy)]
pub struct MarkdownColors {
    pub body: Rgba,
    pub secondary: Rgba,
    pub code_surface: Rgba,
    pub code_border: Rgba,
    pub inline_code_surface: Rgba,
    pub quote_border: Rgba,
    pub table_border: Rgba,
    pub table_header: Rgba,
    pub link: Rgba,
    pub selection: Rgba,
    pub selection_outline: Rgba,
}

#[derive(Clone, Copy)]
pub struct SettingsColors {
    pub nav_backing: Rgba,
    pub island: Rgba,
    pub card: Rgba,
    pub popover: Rgba,
    pub border: Rgba,
    pub divider: Rgba,
    pub focus_border: Rgba,
    pub text_disabled: Rgba,
    pub nav_row: SettingsRowColors,
    pub option_row: SettingsRowColors,
}

#[derive(Clone, Copy)]
pub struct SettingsRowColors {
    pub hover: Rgba,
    pub pressed: Rgba,
    pub selected: Rgba,
    pub selected_hover: Rgba,
    pub selected_pressed: Rgba,
    pub indicator: Rgba,
}

#[derive(Clone, Copy)]
pub struct ControlColors {
    pub idle: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub selected: Rgba,
    pub selected_hover: Rgba,
    pub selected_pressed: Rgba,
    pub outline: Rgba,
    pub disabled_surface: Rgba,
    pub disabled_icon: Rgba,
    pub disabled_outline: Rgba,
    pub hint: Rgba,
}

#[derive(Clone, Copy)]
pub struct TooltipColors {
    pub surface: Rgba,
    pub text: Rgba,
    pub shortcut: Rgba,
}

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct WindowControlColors {
    pub idle: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub icon: Rgba,
    pub close_hover: Rgba,
    pub close_pressed: Rgba,
    pub close_icon: Rgba,
}

#[derive(Clone, Copy)]
pub struct SurfaceColors {
    pub window_backing: Rgba,
    /// Stable backing over the platform's frosted material. A small amount of
    /// the material remains visible without letting the wallpaper set contrast.
    pub sidebar_backing: Rgba,
    pub floating_overlay: Rgba,
    pub shadow_ink: Hsla,
}

#[derive(Clone, Copy)]
pub struct TextColors {
    pub primary: Rgba,
    pub secondary: Rgba,
    pub section: Rgba,
    pub placeholder: Rgba,
}

#[derive(Clone, Copy)]
pub struct FieldColors {
    pub surface: Rgba,
    pub border: Rgba,
    pub focused_border: Rgba,
    pub caret: Rgba,
    pub selection: Rgba,
    pub selection_outline: Rgba,
}

#[derive(Clone, Copy)]
pub struct ScrollbarColors {
    pub track: Rgba,
    pub track_outline: Rgba,
    pub idle: Rgba,
    pub hover: Rgba,
    pub drag: Rgba,
}

impl ScrollbarColors {
    pub fn thumb(self, hovered: bool, dragging: bool) -> Rgba {
        if dragging {
            self.drag
        } else if hovered {
            self.hover
        } else {
            self.idle
        }
    }
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

#[derive(Clone, Copy)]
pub struct TreeColors {
    pub island: TreeRowColors,
    pub desk: TreeRowColors,
    pub directory_text: Rgba,
    pub added: Rgba,
    pub deleted: Rgba,
    pub modified: Rgba,
}

#[derive(Clone, Copy)]
pub struct TreeRowColors {
    pub backing: Rgba,
    pub idle_indicator: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub selected: Rgba,
    pub selected_hover: Rgba,
    pub selected_pressed: Rgba,
    pub hover_indicator: Rgba,
    pub pressed_indicator: Rgba,
    pub selected_indicator: Rgba,
    pub selected_text: Rgba,
}

/// Pure resolver for the built-in Software Theme palettes.
pub fn resolve_software_palette(mode: SoftwareThemeMode) -> SoftwarePalette {
    match mode {
        SoftwareThemeMode::Light => SoftwarePalette {
            feedback: FeedbackColors {
                success: StatusColors {
                    foreground: rgb(0x0e6137),
                    background: rgb(0xe8f7ee),
                    border: rgb(0x327e4e),
                },
                error: StatusColors {
                    foreground: rgb(0xa3261d),
                    background: rgb(0xfdeceb),
                    border: rgb(0xa3261d),
                },
                warning: StatusColors {
                    foreground: rgb(0x754a00),
                    background: rgb(0xfff2cf),
                    border: rgb(0x895a00),
                },
                info: StatusColors {
                    foreground: rgb(0x1d4ed8),
                    background: rgb(0xdbeafe),
                    border: rgb(0x315eaf),
                },
                neutral: StatusColors {
                    foreground: rgb(0x4b5669),
                    background: rgb(0xf1f3f6),
                    border: rgb(0x596579),
                },
            },
            metadata: MetadataColors {
                surface: rgb(0xffffff),
                label: rgb(0x596579),
                text: rgb(0x172033),
                copy_hover: rgb(0xe9ebef),
                copy_pressed: rgb(0xdde3ec),
                row_hover: rgb(0xe9ebef),
                row_pressed: rgb(0xdde3ec),
                range: rgb(0xf1f5ff),
                range_hover: rgb(0xe4edff),
                range_pressed: rgb(0xd3e2ff),
                row_indicator: rgb(0x596579),
                range_indicator: rgb(0x2457d6),
                link: rgb(0x2457d6),
            },
            markdown: MarkdownColors {
                body: rgb(0x172033),
                secondary: rgb(0x596579),
                code_surface: rgb(0xe9ebef),
                code_border: rgb(0x7c8798),
                inline_code_surface: rgb(0xe9ebef),
                quote_border: rgb(0x7c8798),
                table_border: rgb(0x7c8798),
                table_header: rgb(0xe9ebef),
                link: rgb(0x2457d6),
                selection: gpui::rgba(0xffffff1f),
                selection_outline: rgb(0x2457d6),
            },
            control: ControlColors {
                idle: rgb(0xf4f5f7),
                hover: rgb(0xe9ebef),
                pressed: rgb(0xdde3ec),
                selected: rgb(0xdce8f8),
                selected_hover: rgb(0xd1e1f7),
                selected_pressed: rgb(0xc3d9f4),
                outline: rgb(0x7c8798),
                disabled_surface: rgb(0xd0d7e2),
                disabled_icon: rgb(0x657184),
                disabled_outline: rgb(0x536073),
                hint: rgb(0x4b5669),
            },
            tooltip: TooltipColors {
                surface: rgb(0x273142),
                text: rgb(0xffffff),
                shortcut: rgb(0xc5cfde),
            },
            window_control: WindowControlColors {
                idle: rgb(0xf4f5f7),
                hover: rgb(0xdde3ec),
                pressed: rgb(0xc3d9f4),
                icon: rgb(0x4b5669),
                close_hover: rgb(0xb42318),
                close_pressed: rgb(0x8d1c14),
                close_icon: rgb(0xffffff),
            },
            settings: SettingsColors {
                nav_backing: gpui::rgba(0xf4f5f7f5),
                island: rgb(0xffffff),
                card: rgb(0xf4f5f7),
                popover: rgb(0xffffff),
                border: rgb(0x7c8798),
                divider: rgb(0xc3c9d3),
                focus_border: rgb(0x2457d6),
                text_disabled: rgb(0x596579),
                nav_row: SettingsRowColors {
                    hover: rgb(0xe9ebef),
                    pressed: rgb(0xdde3ec),
                    selected: rgb(0xdce8f8),
                    selected_hover: rgb(0xd1e1f7),
                    selected_pressed: rgb(0xc3d9f4),
                    indicator: rgb(0x596579),
                },
                option_row: SettingsRowColors {
                    hover: rgb(0xe9ebef),
                    pressed: rgb(0xdde3ec),
                    selected: rgb(0xdce8f8),
                    selected_hover: rgb(0xd1e1f7),
                    selected_pressed: rgb(0xc3d9f4),
                    indicator: rgb(0x2457d6),
                },
            },
            surface: SurfaceColors {
                window_backing: window_backing(mode),
                sidebar_backing: gpui::rgba(0xf4f5f7e6),
                floating_overlay: gpui::rgba(0xfffffff5),
                shadow_ink: hsla(220. / 360., 0.38, 0.14, 1.),
            },
            text: TextColors {
                primary: rgb(0x172033),
                secondary: rgb(0x596579),
                section: rgb(0x4b5669),
                placeholder: rgb(0x4b5669),
            },
            field: FieldColors {
                surface: rgb(0xffffff),
                border: rgb(0x7c8798),
                focused_border: rgb(0x2457d6),
                caret: rgb(0x2457d6),
                selection: gpui::rgba(0x2457d633),
                selection_outline: rgb(0x2457d6),
            },
            scrollbar: ScrollbarColors {
                track: rgb(0xffffff),
                track_outline: rgb(0x3b4048),
                idle: rgb(0x858c98),
                hover: rgb(0x647b9e),
                drag: rgb(0x3f78bc),
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
            tree: TreeColors {
                directory_text: rgb(0x4b5669),
                island: TreeRowColors {
                    backing: rgb(0xffffff),
                    idle_indicator: gpui::rgba(0x00000000),
                    hover: rgb(0xe9ebef),
                    pressed: rgb(0xdde3ec),
                    selected: rgb(0x2457d6),
                    selected_hover: rgb(0x1e4ebf),
                    selected_pressed: rgb(0x183f9e),
                    hover_indicator: rgb(0x596579),
                    pressed_indicator: rgb(0x2457d6),
                    selected_indicator: rgb(0xffffff),
                    selected_text: rgb(0xffffff),
                },
                desk: TreeRowColors {
                    backing: gpui::rgba(0xf4f5f7e6),
                    idle_indicator: gpui::rgba(0x00000000),
                    hover: rgb(0xe9ebef),
                    pressed: rgb(0xdde3ec),
                    selected: rgb(0xdce8f8),
                    selected_hover: rgb(0xd1e1f7),
                    selected_pressed: rgb(0xc3d9f4),
                    hover_indicator: rgb(0x596579),
                    pressed_indicator: rgb(0x2457d6),
                    selected_indicator: rgb(0x2457d6),
                    selected_text: rgb(0x172033),
                },
                added: rgb(0x0e6137),
                deleted: rgb(0xb42318),
                modified: rgb(0x754a00),
            },
        },
        SoftwareThemeMode::Dark => SoftwarePalette {
            feedback: FeedbackColors {
                success: StatusColors {
                    foreground: rgb(0xb2eabc),
                    background: rgb(0x1e3a2a),
                    border: rgb(0x7acb98),
                },
                error: StatusColors {
                    foreground: rgb(0xffb6ad),
                    background: rgb(0x3d2422),
                    border: rgb(0xe17c75),
                },
                warning: StatusColors {
                    foreground: rgb(0xffdb95),
                    background: rgb(0x423314),
                    border: rgb(0xd7ac60),
                },
                info: StatusColors {
                    foreground: rgb(0xa8cdfc),
                    background: rgb(0x263b55),
                    border: rgb(0x8cb7ed),
                },
                neutral: StatusColors {
                    foreground: rgb(0xd5dae3),
                    background: rgb(0x343b47),
                    border: rgb(0xaeb7c5),
                },
            },
            metadata: MetadataColors {
                surface: rgb(0x30343d),
                label: rgb(0xc5cfde),
                text: rgb(0xd5dae3),
                copy_hover: rgb(0x414a59),
                copy_pressed: rgb(0x4a5669),
                row_hover: rgb(0x414a59),
                row_pressed: rgb(0x4a5669),
                range: rgb(0x30384a),
                range_hover: rgb(0x39465d),
                range_pressed: rgb(0x435570),
                row_indicator: rgb(0xc5cfde),
                range_indicator: rgb(0x89c7f7),
                link: rgb(0x89c7f7),
            },
            markdown: MarkdownColors {
                body: rgb(0xd5dae3),
                secondary: rgb(0xaeb7c5),
                code_surface: rgb(0x343b47),
                code_border: rgb(0x8993a3),
                inline_code_surface: rgb(0x343b47),
                quote_border: rgb(0x8993a3),
                table_border: rgb(0x8993a3),
                table_header: rgb(0x343b47),
                link: rgb(0x89c7f7),
                selection: gpui::rgba(0x0000001f),
                selection_outline: rgb(0x89c7f7),
            },
            control: ControlColors {
                idle: rgb(0x1d2027),
                hover: rgb(0x414a59),
                pressed: rgb(0x4a5669),
                selected: rgb(0x344a65),
                selected_hover: rgb(0x3b5574),
                selected_pressed: rgb(0x435f81),
                outline: rgb(0x8993a3),
                disabled_surface: rgb(0x343b47),
                disabled_icon: rgb(0x96a1b0),
                disabled_outline: rgb(0xb8c4d4),
                hint: rgb(0xd5dae3),
            },
            tooltip: TooltipColors {
                surface: rgb(0x39475b),
                text: rgb(0xffffff),
                shortcut: rgb(0xd5dae3),
            },
            window_control: WindowControlColors {
                idle: rgb(0x1d2027),
                hover: rgb(0x414a59),
                pressed: rgb(0x4a5669),
                icon: rgb(0xc5cfde),
                close_hover: rgb(0xb42318),
                close_pressed: rgb(0x8d1c14),
                close_icon: rgb(0xffffff),
            },
            settings: SettingsColors {
                nav_backing: gpui::rgba(0x1d2027e6),
                island: rgb(0x30343d),
                card: rgb(0x343b47),
                popover: rgb(0x30343d),
                border: rgb(0x8993a3),
                divider: rgb(0x596579),
                focus_border: rgb(0x89c7f7),
                text_disabled: rgb(0xaeb7c5),
                nav_row: SettingsRowColors {
                    hover: rgb(0x343b47),
                    pressed: rgb(0x3e4654),
                    selected: rgb(0x344a65),
                    selected_hover: rgb(0x3b5574),
                    selected_pressed: rgb(0x435f81),
                    indicator: rgb(0xaeb7c5),
                },
                option_row: SettingsRowColors {
                    hover: rgb(0x414a59),
                    pressed: rgb(0x4a5669),
                    selected: rgb(0x344a65),
                    selected_hover: rgb(0x3b5574),
                    selected_pressed: rgb(0x435f81),
                    indicator: rgb(0x89c7f7),
                },
            },
            surface: SurfaceColors {
                window_backing: window_backing(mode),
                sidebar_backing: gpui::rgba(0x1d202780),
                floating_overlay: gpui::rgba(0x272b33f5),
                shadow_ink: hsla(220. / 360., 0.38, 0.03, 1.),
            },
            text: TextColors {
                primary: rgb(0xd5dae3),
                secondary: rgb(0xaeb7c5),
                section: rgb(0xe2e6ed),
                placeholder: rgb(0xc5cfde),
            },
            field: FieldColors {
                surface: rgb(0x30343d),
                border: rgb(0x8993a3),
                focused_border: rgb(0x89c7f7),
                caret: rgb(0x89c7f7),
                selection: gpui::rgba(0x89c7f72a),
                selection_outline: rgb(0x89c7f7),
            },
            scrollbar: ScrollbarColors {
                track: rgb(0x1d2027),
                track_outline: rgb(0xc5cfde),
                idle: rgb(0x858c98),
                hover: rgb(0x6d85a5),
                drag: rgb(0x4c80bb),
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
            tree: TreeColors {
                directory_text: rgb(0xc5cfde),
                island: TreeRowColors {
                    backing: rgb(0x30343d),
                    idle_indicator: gpui::rgba(0x00000000),
                    hover: rgb(0x414a59),
                    pressed: rgb(0x4a5669),
                    selected: rgb(0x2457a8),
                    selected_hover: rgb(0x2b64bf),
                    selected_pressed: rgb(0x1c4f9a),
                    hover_indicator: rgb(0xc5cfde),
                    pressed_indicator: rgb(0x89c7f7),
                    selected_indicator: rgb(0xffffff),
                    selected_text: rgb(0xffffff),
                },
                desk: TreeRowColors {
                    backing: gpui::rgba(0x1d202780),
                    idle_indicator: gpui::rgba(0x00000000),
                    hover: rgb(0x343b47),
                    pressed: rgb(0x414a59),
                    selected: rgb(0x344a65),
                    selected_hover: rgb(0x3b5574),
                    selected_pressed: rgb(0x435f81),
                    hover_indicator: rgb(0xc5cfde),
                    pressed_indicator: rgb(0x89c7f7),
                    selected_indicator: rgb(0x89c7f7),
                    selected_text: rgb(0xd5dae3),
                },
                added: rgb(0xb2eabc),
                deleted: rgb(0xffd2cb),
                modified: rgb(0xffdb95),
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
    let shadow_ink = software_palette().surface.shadow_ink;
    let ink = |a: f32| -> Hsla { Hsla { a, ..shadow_ink } };
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
    software_palette().surface.floating_overlay
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
/// root. The workspace sidebar and Diff ChangedPath tree add bounded backings
/// for text; other desk columns remain clear so their tint does not compound.
pub fn frost() -> Rgba {
    software_palette().surface.window_backing
}

/// Columns that sit directly on the frosted desk, such as the stage between
/// islands. The workspace sidebar and Diff ChangedPath tree have semantic
/// backings because their labels need a stable backdrop.
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
#[allow(dead_code)] // Compatibility role until remaining callers are migrated in ticket 09.
pub fn sidebar_row_selected() -> Rgba {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 0.07,
    }
}
/// Workspace sidebar hover fill on a non-selected row.
#[allow(dead_code)] // Compatibility role until remaining callers are migrated in ticket 09.
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
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
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
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
pub fn success() -> Rgba {
    rgb(0x2f8f55)
}
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
pub fn success_background() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x1e3a2a)
    } else {
        rgb(0xe8f7ee)
    }
}
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
pub fn success_border() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x2f5c40)
    } else {
        rgb(0xc3e5cf)
    }
}
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
pub fn error() -> Rgba {
    rgb(0xcf4a3c)
}
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
pub fn error_background() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x3d2422)
    } else {
        rgb(0xfdeceb)
    }
}
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
pub fn error_border() -> Rgba {
    if crate::ui::code_theme::is_dark() {
        rgb(0x5c3532)
    } else {
        rgb(0xf5c8c2)
    }
}
/// Settings nav row under the pointer: a half-step toward the white
/// [`capsule`] that marks the selected row, so hover never reads as selection.
#[allow(dead_code)] // Compatibility helper until ticket 09 retires old color accessors.
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

    #[test]
    fn find_and_draft_fields_remain_readable_over_either_code_paper() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let palette = resolve_software_palette(mode);
            let field = palette.field;
            let selected_surface = composite(field.selection, field.surface);
            for (name, text) in [
                ("placeholder", palette.text.placeholder),
                ("entered text", palette.text.primary),
            ] {
                assert!(
                    contrast(text, field.surface) >= 4.5,
                    "{mode:?} {name} on input: {}",
                    contrast(text, field.surface)
                );
                assert!(
                    contrast(text, selected_surface) >= 4.5,
                    "{mode:?} {name} on selection: {}",
                    contrast(text, selected_surface)
                );
            }
            for (name, mark) in [
                ("idle border", field.border),
                ("focused border", field.focused_border),
                ("caret", field.caret),
                ("selection outline", field.selection_outline),
            ] {
                assert!(
                    contrast(mark, field.surface) >= 3.,
                    "{mode:?} {name} on input: {}",
                    contrast(mark, field.surface)
                );
            }
            assert!(
                contrast(field.selection_outline, selected_surface) >= 3.,
                "{mode:?} selection outline on selection: {}",
                contrast(field.selection_outline, selected_surface)
            );
            for code_paper in [rgb(0x000000), rgb(0xffffff)] {
                let overlay = composite(palette.surface.floating_overlay, code_paper);
                assert!(contrast(palette.text.primary, overlay) >= 4.5);
                assert!(contrast(palette.text.secondary, overlay) >= 4.5);
                assert!(contrast(field.border, overlay) >= 3.);
            }
        }
    }

    #[test]
    fn scrollbar_track_and_thumb_stay_visible_on_every_builtin_diff_band() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let colors = resolve_software_palette(mode).scrollbar;
            assert_eq!(colors.thumb(false, false), colors.idle);
            assert_eq!(colors.thumb(true, false), colors.hover);
            assert_eq!(colors.thumb(true, true), colors.drag);
            assert_eq!(colors.thumb(false, true), colors.drag);
            assert!(contrast(colors.track, colors.track_outline) >= 3.);
            for (name, thumb) in [
                ("idle", colors.idle),
                ("hover", colors.hover),
                ("drag", colors.drag),
            ] {
                assert!(
                    contrast(thumb, colors.track) >= 3.,
                    "{mode:?} {name} thumb on track: {}",
                    contrast(thumb, colors.track)
                );
            }
            for code_theme in crate::ui::code_theme::builtin_catalog() {
                for band in [
                    code_theme.slots.paper,
                    code_theme.slots.added_band,
                    code_theme.slots.deleted_band,
                    code_theme.slots.replaced_band,
                ] {
                    assert!(
                        contrast(colors.track, band).max(contrast(colors.track_outline, band))
                            >= 3.,
                        "{mode:?} track boundary on {} band {band:?}",
                        code_theme.id
                    );
                }
            }
        }
    }

    #[test]
    fn changed_path_tree_roles_are_readable_on_island_and_composed_desk() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let palette = resolve_software_palette(mode);
            for wallpaper in [rgb(0x000000), rgb(0xffffff)] {
                let window = composite(palette.surface.window_backing, wallpaper);
                let surfaces = [
                    ("island", palette.tree.island, palette.tree.island.backing),
                    (
                        "desk",
                        palette.tree.desk,
                        composite(palette.tree.desk.backing, window),
                    ),
                ];
                for (name, rows, idle) in surfaces {
                    for (state, surface) in [
                        ("idle", idle),
                        ("hover", rows.hover),
                        ("pressed", rows.pressed),
                    ] {
                        for (text_name, text) in [
                            ("name", palette.text.primary),
                            ("directory", palette.tree.directory_text),
                            ("added", palette.tree.added),
                            ("deleted", palette.tree.deleted),
                            ("modified", palette.tree.modified),
                        ] {
                            assert!(
                                contrast(text, surface) >= 4.5,
                                "{mode:?} {name} {state} {text_name}: {}",
                                contrast(text, surface)
                            );
                        }
                    }
                    for (state, fill, mark) in [
                        ("hover", rows.hover, rows.hover_indicator),
                        ("pressed", rows.pressed, rows.pressed_indicator),
                        ("selected", rows.selected, rows.selected_indicator),
                        (
                            "selected hover",
                            rows.selected_hover,
                            rows.selected_indicator,
                        ),
                        (
                            "selected pressed",
                            rows.selected_pressed,
                            rows.selected_indicator,
                        ),
                    ] {
                        assert!(
                            contrast(mark, fill) >= 3.,
                            "{mode:?} {name} {state} indicator: {}",
                            contrast(mark, fill)
                        );
                        if state.starts_with("selected") {
                            assert!(
                                contrast(rows.selected_text, fill) >= 4.5,
                                "{mode:?} {name} {state} name: {}",
                                contrast(rows.selected_text, fill)
                            );
                            if name == "desk" {
                                for status in [
                                    palette.tree.added,
                                    palette.tree.deleted,
                                    palette.tree.modified,
                                ] {
                                    assert!(
                                        contrast(status, fill) >= 4.5,
                                        "{mode:?} desk {state} status {status:?}: {}",
                                        contrast(status, fill)
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn settings_and_shared_controls_are_readable_on_their_actual_surfaces() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let palette = resolve_software_palette(mode);
            let settings = palette.settings;
            for wallpaper in [rgb(0x000000), rgb(0xffffff)] {
                let root = composite(palette.surface.window_backing, wallpaper);
                let nav = composite(settings.nav_backing, root);
                for (name, surface) in [
                    ("nav", nav),
                    ("island", settings.island),
                    ("card", settings.card),
                    ("popover", settings.popover),
                    ("control idle", palette.control.idle),
                    ("control hover", palette.control.hover),
                    ("control pressed", palette.control.pressed),
                    ("control selected", palette.control.selected),
                    ("control selected hover", palette.control.selected_hover),
                    ("control selected pressed", palette.control.selected_pressed),
                ] {
                    for (text_name, text) in [
                        ("primary", palette.text.primary),
                        ("secondary", palette.text.secondary),
                    ] {
                        let target = if name.starts_with("control ") && text_name == "secondary" {
                            3.
                        } else {
                            4.5
                        };
                        assert!(
                            contrast(text, surface) >= target,
                            "{mode:?} {text_name} on {name}: {}",
                            contrast(text, surface)
                        );
                    }
                    if name.starts_with("control ") {
                        assert!(
                            contrast(palette.control.hint, surface) >= 4.5,
                            "{mode:?} control hint on {name}"
                        );
                    }
                }
                for (name, rows, idle) in [
                    ("nav row", settings.nav_row, nav),
                    ("option row", settings.option_row, settings.popover),
                ] {
                    for (state, fill) in [
                        ("idle", idle),
                        ("hover", rows.hover),
                        ("pressed", rows.pressed),
                        ("selected", rows.selected),
                        ("selected hover", rows.selected_hover),
                        ("selected pressed", rows.selected_pressed),
                    ] {
                        assert!(
                            contrast(palette.text.primary, fill) >= 4.5,
                            "{mode:?} {name} {state} primary text"
                        );
                        let secondary_target =
                            if name == "nav row" && !state.starts_with("selected") {
                                4.5
                            } else {
                                3.
                            };
                        assert!(
                            contrast(palette.text.secondary, fill) >= secondary_target,
                            "{mode:?} {name} {state} secondary icon/text"
                        );
                    }
                    for fill in [rows.selected, rows.selected_hover, rows.selected_pressed] {
                        assert!(
                            contrast(rows.indicator, fill) >= 3.,
                            "{mode:?} {name} selected indicator"
                        );
                    }
                    if name == "nav row" {
                        assert!(contrast(settings.focus_border, rows.selected) >= 3.);
                        assert!(contrast(settings.focus_border, rows.selected_hover) >= 3.);
                        assert!(contrast(settings.focus_border, rows.selected_pressed) >= 3.);
                    }
                    assert!(contrast(palette.text.secondary, rows.hover) >= 3.);
                    assert!(contrast(rows.indicator, rows.pressed) >= 3.);
                }
                for surface in [settings.island, settings.card, settings.popover] {
                    assert!(contrast(settings.border, surface) >= 3.);
                    assert!(contrast(settings.focus_border, surface) >= 3.);
                    assert!(contrast(palette.control.outline, surface) >= 3.);
                    assert!(contrast(settings.text_disabled, surface) >= 4.5);
                }
                assert!(
                    contrast(
                        palette.control.disabled_icon,
                        palette.control.disabled_surface
                    ) >= 3.
                );
                assert!(
                    contrast(
                        palette.control.disabled_outline,
                        palette.control.disabled_surface
                    ) >= 3.,
                    "{mode:?} disabled outline on disabled control surface"
                );
                assert!(
                    contrast(palette.control.disabled_surface, palette.control.idle) >= 1.2,
                    "{mode:?} disabled control backing must be distinguishable from enabled idle"
                );
            }
            let tooltip = palette.tooltip;
            assert!(contrast(tooltip.text, tooltip.surface) >= 4.5);
            assert!(contrast(tooltip.shortcut, tooltip.surface) >= 4.5);
            let caption = palette.window_control;
            for surface in [caption.idle, caption.hover, caption.pressed] {
                assert!(contrast(caption.icon, surface) >= 3.);
            }
            for surface in [caption.close_hover, caption.close_pressed] {
                assert!(contrast(caption.close_icon, surface) >= 3.);
            }
        }
    }

    #[test]
    fn feedback_metadata_and_markdown_meet_contrast_on_content_surfaces() {
        for mode in [SoftwareThemeMode::Light, SoftwareThemeMode::Dark] {
            let palette = resolve_software_palette(mode);
            let content = palette.metadata;
            let markdown = palette.markdown;
            for (tone, colors) in [
                ("success", palette.feedback.success),
                ("error", palette.feedback.error),
                ("warning", palette.feedback.warning),
                ("info", palette.feedback.info),
                ("neutral", palette.feedback.neutral),
            ] {
                for (surface, bg) in [
                    ("badge", colors.background),
                    ("metadata island", content.surface),
                    ("settings card", palette.settings.card),
                ] {
                    assert!(
                        contrast(colors.foreground, bg) >= 4.5,
                        "{mode:?} {tone} foreground on {surface}: {}",
                        contrast(colors.foreground, bg)
                    );
                }
                assert!(
                    contrast(colors.border, colors.background) >= 3.,
                    "{mode:?} {tone} badge border"
                );
            }
            for (surface, bg) in [
                ("idle", content.surface),
                ("hover", content.row_hover),
                ("pressed", content.row_pressed),
                ("range", content.range),
                ("range hover", content.range_hover),
                ("range pressed", content.range_pressed),
                ("copy hover", content.copy_hover),
                ("copy pressed", content.copy_pressed),
            ] {
                for (name, fg) in [("text", content.text), ("label", content.label)] {
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "{mode:?} metadata {name} on {surface}: {}",
                        contrast(fg, bg)
                    );
                }
            }
            assert!(contrast(content.link, content.surface) >= 4.5);
            for (name, indicator, backgrounds) in [
                (
                    "commit row",
                    content.row_indicator,
                    [content.row_hover, content.row_pressed],
                ),
                (
                    "selected commit row",
                    content.range_indicator,
                    [content.range_hover, content.range_pressed],
                ),
            ] {
                for bg in backgrounds {
                    assert!(
                        contrast(indicator, bg) >= 3.,
                        "{mode:?} {name} interaction indicator: {}",
                        contrast(indicator, bg)
                    );
                }
            }
            assert!(contrast(content.range_indicator, content.range) >= 3.);
            for (state, bg) in [
                ("hover", content.copy_hover),
                ("pressed", content.copy_pressed),
            ] {
                assert!(
                    contrast(palette.feedback.neutral.foreground, bg) >= 4.5,
                    "{mode:?} copy capsule {state} text"
                );
                assert!(
                    contrast(palette.feedback.neutral.border, bg) >= 3.,
                    "{mode:?} copy capsule {state} indicator"
                );
            }
            for (surface, bg) in [
                ("body", content.surface),
                ("code block", markdown.code_surface),
                ("inline code", markdown.inline_code_surface),
                ("table heading", markdown.table_header),
            ] {
                for (name, fg) in [
                    ("body", markdown.body),
                    ("secondary", markdown.secondary),
                    ("link", markdown.link),
                ] {
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "{mode:?} Markdown {name} on {surface}: {}",
                        contrast(fg, bg)
                    );
                    let selected = composite(markdown.selection, bg);
                    assert!(
                        contrast(fg, selected) >= 4.5,
                        "{mode:?} selected Markdown {name} on {surface}: {}",
                        contrast(fg, selected)
                    );
                }
                assert!(
                    contrast(
                        markdown.selection_outline,
                        composite(markdown.selection, bg)
                    ) >= 3.,
                    "{mode:?} Markdown selection mark on {surface}"
                );
            }
            for (name, mark, bg) in [
                ("code border", markdown.code_border, markdown.code_surface),
                ("quote border", markdown.quote_border, content.surface),
                ("table border", markdown.table_border, content.surface),
                (
                    "table border on heading",
                    markdown.table_border,
                    markdown.table_header,
                ),
            ] {
                assert!(
                    contrast(mark, bg) >= 3.,
                    "{mode:?} Markdown {name}: {}",
                    contrast(mark, bg)
                );
            }
        }
    }
}

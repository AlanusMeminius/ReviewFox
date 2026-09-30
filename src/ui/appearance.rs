//! UI Font, Code Font, and the Code Theme Pairing: what `settings.json` stores,
//! resolved into the [`Appearance`] Global. Rendering reads the Global;
//! [`update`] writes the file and replaces it, which re-renders every window.

use std::ops::RangeInclusive;
use std::sync::Arc;

use gpui::{App, Global, Pixels, SharedString, Styled, px};

use super::code_theme::{self, CodeThemePairing};
use crate::domain::DiffFontSize;
use crate::settings_store::{self, SettingsFile};

/// A font family as rendering uses it, plus what the user stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    /// The family to render with: the stored one when installed, else Default.
    pub name: SharedString,
    /// What `settings.json` holds; `None` is Default.
    pub stored: Option<String>,
    /// `stored` is set but not installed, so `name` fell back to Default.
    pub not_installed: bool,
}

/// Resolved UI Font, Code Font, and stored Code Theme Pairing.
///
/// Each Code Theme id is `None` when unset (One Light). An unknown id is kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub software_theme: code_theme::SoftwareThemeMode,
    pub ui_font: Family,
    pub ui_font_size: u32,
    pub code_font: Family,
    pub code_font_size: u32,
    pub code_theme_light: Option<String>,
    pub code_theme_dark: Option<String>,
}

impl Global for Appearance {}

/// Prototype A's UI face. Default even when not installed (the OS falls back
/// to a proportional face, which is fine for UI text).
pub const DEFAULT_UI_FONT: &str = "IBM Plex Sans";
/// Code Font Default when installed.
const PLEX_MONO: &str = "IBM Plex Mono";
/// UI Font size default and range. The Code Font's are `DiffFontSize`'s.
pub const UI_FONT_SIZE_DEFAULT: u32 = 13;
pub const UI_FONT_SIZE_MIN: u32 = 11;
pub const UI_FONT_SIZE_MAX: u32 = 15;

/// UI Font or Code Font, for code that treats both alike (the Settings page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontRole {
    Ui,
    Code,
}

impl FontRole {
    pub fn family(self, appearance: &Appearance) -> &Family {
        match self {
            FontRole::Ui => &appearance.ui_font,
            FontRole::Code => &appearance.code_font,
        }
    }

    pub fn size(self, appearance: &Appearance) -> u32 {
        match self {
            FontRole::Ui => appearance.ui_font_size,
            FontRole::Code => appearance.code_font_size,
        }
    }

    pub fn default_size(self) -> u32 {
        match self {
            FontRole::Ui => UI_FONT_SIZE_DEFAULT,
            FontRole::Code => DiffFontSize::DEFAULT,
        }
    }

    pub fn size_range(self) -> RangeInclusive<u32> {
        match self {
            FontRole::Ui => UI_FONT_SIZE_MIN..=UI_FONT_SIZE_MAX,
            FontRole::Code => DiffFontSize::MIN..=DiffFontSize::MAX,
        }
    }

    /// Store a chosen size; `None` (Reset) and the default both remove the
    /// field, so a later default change still reaches the user.
    pub fn set_size(self, file: &mut SettingsFile, size: Option<u32>) {
        let stored = size
            .filter(|&size| size != self.default_size())
            .map(|size| size as f32);
        match self {
            FontRole::Ui => file.ui_font_size = stored,
            FontRole::Code => file.code_font_size = stored,
        }
    }

    /// The family Default resolves to on this machine.
    pub fn default_family(self, installed: &[SharedString]) -> &'static str {
        match self {
            FontRole::Ui => DEFAULT_UI_FONT,
            FontRole::Code => default_code_font(installed),
        }
    }

    /// Store a chosen family; like [`Self::set_size`], `None` (Reset) and the
    /// family Default resolves to both remove the field.
    pub fn set_family(
        self,
        file: &mut SettingsFile,
        family: Option<&str>,
        installed: &[SharedString],
    ) {
        let stored = family
            .filter(|&family| family != self.default_family(installed))
            .map(str::to_string);
        match self {
            FontRole::Ui => file.ui_font_family = stored,
            FontRole::Code => file.code_font_family = stored,
        }
    }
}

/// Code Font Default: Plex Mono when installed, else the system monospace. The
/// OS fallback for a missing Plex is proportional, which breaks column
/// alignment and tab stops.
fn default_code_font(installed: &[SharedString]) -> &'static str {
    if installed.iter().any(|name| name.as_ref() == PLEX_MONO) {
        PLEX_MONO
    } else if cfg!(windows) {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        PLEX_MONO
    }
}

/// Installed font names (sorted, deduplicated), read once at startup: what
/// [`update`] resolves against and what the font picker lists (shared, so
/// opening the picker does not copy it).
struct InstalledFonts(Arc<[SharedString]>);

impl Global for InstalledFonts {}

/// Resolve `settings.json` into the Global and re-render every window whenever
/// it changes. Call before opening any window.
pub fn init(cx: &mut App) {
    let installed: Arc<[SharedString]> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .map(SharedString::from)
        .collect();
    let appearance = resolve(&settings_store::load_file(), &installed);
    remember(&appearance);
    cx.set_global(appearance);
    cx.set_global(InstalledFonts(installed));
    // Views cached with `AnyView::cached` re-render only on a refresh.
    cx.observe_global::<Appearance>(|cx| cx.refresh_windows())
        .detach();
}

/// Apply `edit` to `settings.json`, save it, and replace the Global when the
/// resolved result changed. Observers of [`Appearance`] hear about it. A
/// refused save still applies live, for this session only.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut SettingsFile)) {
    let mut file = settings_store::load_file();
    edit(&mut file);
    settings_store::save_file(&file).ok();
    let next = resolve(&file, &cx.global::<InstalledFonts>().0);
    // Publish before the Global swap so a refresh paints the new light choice.
    remember(&next);
    if &next != cx.global::<Appearance>() {
        cx.set_global(next);
    }
}

/// Set the Software Theme and persist it for the whole application.
pub fn set_software_theme(cx: &mut App, mode: code_theme::SoftwareThemeMode) {
    update(cx, |file| {
        file.software_theme = Some(
            match mode {
                code_theme::SoftwareThemeMode::Light => "light",
                code_theme::SoftwareThemeMode::Dark => "dark",
            }
            .to_string(),
        );
    });
}

/// Code Theme for the light Software Theme. The id `one-light` clears the field.
pub fn set_code_theme_light(cx: &mut App, id: &str) {
    update(cx, |file| {
        file.code_theme_light = settings_store::code_theme_choice(Some(id.to_string()));
    });
}

fn remember(appearance: &Appearance) {
    code_theme::remember_pairing(CodeThemePairing {
        light: appearance.code_theme_light.clone(),
        dark: appearance.code_theme_dark.clone(),
    });
    code_theme::remember_mode(appearance.software_theme);
}

/// The installed font names, cached since startup.
pub fn installed_fonts(cx: &App) -> Arc<[SharedString]> {
    cx.global::<InstalledFonts>().0.clone()
}

/// Store a picked family (`None`: Reset) through [`update`]; see
/// [`FontRole::set_family`].
pub fn set_family(cx: &mut App, role: FontRole, family: Option<&str>) {
    let installed = installed_fonts(cx);
    update(cx, |file| role.set_family(file, family, &installed));
}

/// UI Font family to render with.
pub fn ui_font(cx: &App) -> SharedString {
    cx.global::<Appearance>().ui_font.name.clone()
}

/// Code Font family to render with (Diff text, line numbers, mono chrome).
pub fn code_font(cx: &App) -> SharedString {
    cx.global::<Appearance>().code_font.name.clone()
}

/// A UI text size as designed at the default UI Font size (13), for the
/// current `ui_font_size`: the setting is the body size and every other UI
/// text size keeps its offset from it.
pub fn ui_text_px(design: f32, ui_font_size: u32) -> f32 {
    design + (ui_font_size as f32 - UI_FONT_SIZE_DEFAULT as f32)
}

/// [`ui_text_px`] at the current UI Font size.
pub fn ui_text(cx: &App, design: f32) -> Pixels {
    px(ui_text_px(design, cx.global::<Appearance>().ui_font_size))
}

/// Downward shift that centres glyph ink in its line box: gpui centres
/// ascent + descent, but ink only spans `ink_below` below the baseline to
/// `ink_above` above it (both distances ≥ 0).
pub(crate) fn ink_center_nudge(ascent: f32, descent: f32, ink_above: f32, ink_below: f32) -> f32 {
    (ink_above - ink_below + descent.abs() - ascent) / 2.
}

/// [`ink_center_nudge`] for Latin caps / figures (`ink` = baseline..cap height).
fn cap_center_nudge(ascent: f32, descent: f32, cap_height: f32) -> f32 {
    ink_center_nudge(ascent, descent, cap_height, 0.)
}

/// [`cap_center_nudge`] at `size` for `family`, from the metrics of the face
/// actually drawn (the fallback when the family is not installed).
pub fn cap_nudge(cx: &App, family: SharedString, size: Pixels) -> Pixels {
    let text = cx.text_system();
    let id = text.resolve_font(&gpui::font(family));
    px(cap_center_nudge(
        text.ascent(id, size).into(),
        text.descent(id, size).into(),
        text.cap_height(id, size).into(),
    ))
}

/// Ink above / below the baseline from a glyph's typographic box (font y-up,
/// baseline at 0). Used with [`ink_center_nudge`].
pub(crate) fn ink_extents_from_bounds(bounds: gpui::Bounds<Pixels>) -> (f32, f32) {
    let bottom: f32 = bounds.origin.y.into();
    let top: f32 = (bounds.origin.y + bounds.size.height).into();
    (top.max(0.), (-bottom).max(0.))
}

/// Sizes UI Font text through [`ui_text`]. Every UI text size goes through
/// this, in design px: 12 for gpui's `text_xs`, 14 for `text_sm`, 16 for the
/// unsized default (1rem, set at each window root). Code Font chrome/meta text
/// keeps gpui's rem helpers and does not scale, so it sets its own size even
/// inside UI text; icon glyphs and layout sizes never scale.
pub trait UiTextSize: Styled + Sized {
    fn ui_text_size(self, design: f32, cx: &App) -> Self {
        self.text_size(ui_text(cx, design))
    }

    /// Text beside an icon: shifts only this element so caps centre on the icon.
    /// Put it on the text element, never on a container with hover/background/border.
    fn ui_label_size(self, design: f32, cx: &App) -> Self {
        let size = ui_text(cx, design);
        self.text_size(size)
            .relative()
            .top(cap_nudge(cx, ui_font(cx), size))
    }

    /// Code Font meta text at an unscaled `size` (not [`ui_text`]).
    fn code_label_size(self, size: Pixels, family: SharedString, cx: &App) -> Self {
        self.text_size(size)
            .relative()
            .top(cap_nudge(cx, family, size))
    }
}

impl<E: Styled> UiTextSize for E {}

pub fn resolve(file: &SettingsFile, installed: &[SharedString]) -> Appearance {
    Appearance {
        software_theme: match file.software_theme.as_deref() {
            Some("dark") => code_theme::SoftwareThemeMode::Dark,
            _ => code_theme::SoftwareThemeMode::Light,
        },
        ui_font: resolve_family(
            file.ui_font_family.as_deref(),
            FontRole::Ui.default_family(installed),
            installed,
        ),
        ui_font_size: round_size(file.ui_font_size).map_or(UI_FONT_SIZE_DEFAULT, |px| {
            px.clamp(UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX)
        }),
        code_font: resolve_family(
            file.code_font_family.as_deref(),
            FontRole::Code.default_family(installed),
            installed,
        ),
        // `DiffFontSize::new` owns the Code Font size range.
        code_font_size: DiffFontSize::new(
            round_size(file.code_font_size).unwrap_or(DiffFontSize::DEFAULT),
        )
        .base(),
        code_theme_light: settings_store::code_theme_choice(file.code_theme_light.clone()),
        code_theme_dark: settings_store::code_theme_choice(file.code_theme_dark.clone()),
    }
}

/// Sizes step by 1, so fractions round; unset or not a number is `None`
/// (Default). The caller clamps.
fn round_size(stored: Option<f32>) -> Option<u32> {
    stored
        .filter(|v| v.is_finite())
        .map(|v| v.round().max(0.) as u32)
}

fn resolve_family(
    stored: Option<&str>,
    default: &'static str,
    installed: &[SharedString],
) -> Family {
    let found = stored.and_then(|s| installed.iter().find(|name| name.as_ref() == s));
    Family {
        name: found.map_or_else(|| default.into(), SharedString::clone),
        stored: stored.map(str::to_string),
        not_installed: stored.is_some() && found.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(names: &[&'static str]) -> Vec<SharedString> {
        names.iter().map(|&n| n.into()).collect()
    }

    #[test]
    fn cap_center_nudge_follows_the_face_metrics() {
        let at_12 = |units: f32| units / 2048. * 12.;
        // Helvetica: shallow ascent, deep descent, so caps sit high.
        let helvetica = cap_center_nudge(at_12(1577.), at_12(-471.), at_12(1469.));
        assert!((helvetica - 1.06).abs() < 0.01, "{helvetica}");
        // IBM Plex Sans (1000 upm): tall ascent, so caps sit low.
        let plex = cap_center_nudge(12.3, 3.3, 8.376);
        assert!((plex + 0.31).abs() < 0.01, "{plex}");
    }

    #[test]
    fn ink_center_nudge_shifts_cjk_like_ink_down_when_descent_is_unused() {
        // Em-like ink fills most of ascent; unused descent leaves optical mass high.
        // (10 - 0.5 + 3 - 11) / 2 = 0.75
        let nudge = ink_center_nudge(11., 3., 10., 0.5);
        assert!((nudge - 0.75).abs() < 0.01, "{nudge}");
        assert!(nudge > 0., "should shift down");
    }

    #[test]
    fn ink_extents_from_bounds_split_at_baseline() {
        // Cap-like: entirely above baseline.
        assert_eq!(
            ink_extents_from_bounds(gpui::Bounds {
                origin: gpui::point(px(0.), px(0.)),
                size: gpui::size(px(8.), px(9.)),
            }),
            (9., 0.)
        );
        // Descender: straddles baseline (font y-up).
        assert_eq!(
            ink_extents_from_bounds(gpui::Bounds {
                origin: gpui::point(px(0.), px(-3.)),
                size: gpui::size(px(6.), px(10.)),
            }),
            (7., 3.)
        );
    }

    #[test]
    fn unset_settings_resolve_to_defaults() {
        let with_plex = resolve(
            &SettingsFile::default(),
            &installed(&["IBM Plex Mono", "Arial"]),
        );
        assert_eq!(with_plex.ui_font.name, "IBM Plex Sans");
        assert_eq!(with_plex.ui_font.stored, None);
        assert!(!with_plex.ui_font.not_installed);
        assert_eq!(with_plex.code_font.name, "IBM Plex Mono");
        assert_eq!(with_plex.ui_font_size, 13);
        assert_eq!(with_plex.code_font_size, 13);

        // Without Plex Mono the Code Font default is the system monospace.
        let without_plex = resolve(&SettingsFile::default(), &installed(&["Arial"]));
        let system_mono = if cfg!(windows) {
            "Consolas"
        } else if cfg!(target_os = "macos") {
            "Menlo"
        } else {
            "IBM Plex Mono"
        };
        assert_eq!(without_plex.code_font.name, system_mono);
    }

    #[test]
    fn installed_stored_family_is_used() {
        let file = SettingsFile {
            ui_font_family: Some("Inter".into()),
            code_font_family: Some("JetBrains Mono".into()),
            ..Default::default()
        };
        let a = resolve(
            &file,
            &installed(&["Inter", "JetBrains Mono", "IBM Plex Mono"]),
        );
        assert_eq!(a.ui_font.name, "Inter");
        assert_eq!(a.ui_font.stored.as_deref(), Some("Inter"));
        assert!(!a.ui_font.not_installed);
        assert_eq!(a.code_font.name, "JetBrains Mono");
        assert!(!a.code_font.not_installed);
    }

    #[test]
    fn uninstalled_stored_family_renders_default_but_is_kept_and_flagged() {
        let file = SettingsFile {
            ui_font_family: Some("Gone Sans".into()),
            code_font_family: Some("Gone Mono".into()),
            ..Default::default()
        };
        let a = resolve(&file, &installed(&["IBM Plex Mono"]));
        assert_eq!(a.ui_font.name, "IBM Plex Sans");
        assert_eq!(a.ui_font.stored.as_deref(), Some("Gone Sans"));
        assert!(a.ui_font.not_installed);
        assert_eq!(a.code_font.name, "IBM Plex Mono");
        assert_eq!(a.code_font.stored.as_deref(), Some("Gone Mono"));
        assert!(a.code_font.not_installed);
    }

    #[test]
    fn ui_text_keeps_its_offset_from_the_body_size() {
        // Default: every design size renders as designed.
        assert_eq!(ui_text_px(12., 13), 12.);
        assert_eq!(ui_text_px(14., 13), 14.);
        // Spec example: at 14, former 12px text is 13px and 16px is 17px.
        assert_eq!(ui_text_px(12., 14), 13.);
        assert_eq!(ui_text_px(16., 14), 17.);
        assert_eq!(ui_text_px(13., 11), 11.);
        assert_eq!(ui_text_px(16., 15), 18.);
    }

    #[test]
    fn setting_a_size_to_its_default_or_resetting_removes_the_field() {
        let mut file = SettingsFile::default();
        FontRole::Ui.set_size(&mut file, Some(14));
        FontRole::Code.set_size(&mut file, Some(18));
        assert_eq!(
            (file.ui_font_size, file.code_font_size),
            (Some(14.), Some(18.))
        );

        FontRole::Ui.set_size(&mut file, Some(13));
        FontRole::Code.set_size(&mut file, None);
        assert_eq!((file.ui_font_size, file.code_font_size), (None, None));

        FontRole::Code.set_size(&mut file, Some(13));
        assert_eq!(file.code_font_size, None);
    }

    #[test]
    fn choosing_the_default_family_or_resetting_removes_the_field() {
        let fonts = installed(&["IBM Plex Mono", "Inter", "Consolas"]);
        let mut file = SettingsFile::default();
        FontRole::Ui.set_family(&mut file, Some("Inter"), &fonts);
        FontRole::Code.set_family(&mut file, Some("Consolas"), &fonts);
        assert_eq!(file.ui_font_family.as_deref(), Some("Inter"));
        assert_eq!(file.code_font_family.as_deref(), Some("Consolas"));

        // The resolved Default: Plex Sans for UI, Plex Mono (installed) for Code.
        FontRole::Ui.set_family(&mut file, Some("IBM Plex Sans"), &fonts);
        FontRole::Code.set_family(&mut file, Some("IBM Plex Mono"), &fonts);
        assert_eq!(
            (file.ui_font_family.clone(), file.code_font_family.clone()),
            (None, None)
        );

        FontRole::Ui.set_family(&mut file, Some("Inter"), &fonts);
        FontRole::Ui.set_family(&mut file, None, &fonts);
        assert_eq!(file.ui_font_family, None);
    }

    #[test]
    fn stored_sizes_are_rounded_and_clamped_to_their_ranges() {
        let sizes = |ui: f32, code: f32| {
            let file = SettingsFile {
                ui_font_size: Some(ui),
                code_font_size: Some(code),
                ..Default::default()
            };
            let a = resolve(&file, &[]);
            (a.ui_font_size, a.code_font_size)
        };
        assert_eq!(sizes(14., 18.), (14, 18));
        assert_eq!(sizes(14.4, 17.6), (14, 18));
        assert_eq!(sizes(2., 2.), (11, 10));
        assert_eq!(sizes(99., 99.), (15, 22));
        assert_eq!(sizes(f32::NAN, f32::NAN), (13, 13));
    }
}

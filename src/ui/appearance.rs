//! UI Font and Code Font: what `settings.json` stores, resolved against the
//! installed fonts and held in the [`Appearance`] Global. Rendering reads the
//! Global; [`update`] writes the file and replaces it, which re-renders every
//! window.

use gpui::{App, Global, SharedString};

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

/// Resolved UI Font and Code Font.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub ui_font: Family,
    pub ui_font_size: u32,
    pub code_font: Family,
    pub code_font_size: u32,
}

impl Global for Appearance {}

/// Prototype A's UI face. Default even when not installed (the OS falls back
/// to a proportional face, which is fine for UI text).
pub const DEFAULT_UI_FONT: &str = "IBM Plex Sans";
/// Code Font Default when installed.
const PLEX_MONO: &str = "IBM Plex Mono";
pub const DEFAULT_FONT_SIZE: u32 = 13;
/// UI Font size range; the Code Font size range is `DiffFontSize`'s.
pub const UI_FONT_SIZE_MIN: u32 = 11;
pub const UI_FONT_SIZE_MAX: u32 = 15;

/// Code Font Default: Plex Mono when installed, else the system monospace. The
/// OS fallback for a missing Plex is proportional, which breaks column
/// alignment and tab stops.
fn default_code_font(installed: &[String]) -> &'static str {
    if installed.iter().any(|name| name == PLEX_MONO) {
        PLEX_MONO
    } else if cfg!(windows) {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        PLEX_MONO
    }
}

/// Installed font names, read once at startup; what [`update`] resolves against.
#[allow(dead_code)] // Read only by `update`.
struct InstalledFonts(Vec<String>);

impl Global for InstalledFonts {}

/// Resolve `settings.json` into the Global and re-render every window whenever
/// it changes. Call before opening any window.
pub fn init(cx: &mut App) {
    let installed = cx.text_system().all_font_names();
    cx.set_global(resolve(&settings_store::load_file(), &installed));
    cx.set_global(InstalledFonts(installed));
    // Views cached with `AnyView::cached` re-render only on a refresh.
    cx.observe_global::<Appearance>(|cx| cx.refresh_windows()).detach();
}

/// Apply `edit` to `settings.json`, save it, and replace the Global when the
/// resolved result changed. Observers of [`Appearance`] hear about it.
#[allow(dead_code)] // No Settings page edits fonts yet.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut SettingsFile)) {
    let mut file = settings_store::load_file();
    edit(&mut file);
    settings_store::save_file(&file);
    let next = resolve(&file, &cx.global::<InstalledFonts>().0);
    if &next != cx.global::<Appearance>() {
        cx.set_global(next);
    }
}

/// UI Font family to render with.
pub fn ui_font(cx: &App) -> SharedString {
    cx.global::<Appearance>().ui_font.name.clone()
}

/// Code Font family to render with (Diff text, line numbers, mono chrome).
pub fn code_font(cx: &App) -> SharedString {
    cx.global::<Appearance>().code_font.name.clone()
}

pub fn resolve(file: &SettingsFile, installed: &[String]) -> Appearance {
    Appearance {
        ui_font: resolve_family(file.ui_font_family.as_deref(), DEFAULT_UI_FONT, installed),
        ui_font_size: resolve_size(file.ui_font_size, UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX),
        code_font: resolve_family(
            file.code_font_family.as_deref(),
            default_code_font(installed),
            installed,
        ),
        code_font_size: resolve_size(file.code_font_size, DiffFontSize::MIN, DiffFontSize::MAX),
    }
}

/// Unset or not a number is Default; out of range is clamped (step 1, so
/// fractions round).
fn resolve_size(stored: Option<f32>, min: u32, max: u32) -> u32 {
    match stored {
        Some(v) if v.is_finite() => (v.round().max(0.) as u32).clamp(min, max),
        _ => DEFAULT_FONT_SIZE,
    }
}

fn resolve_family(stored: Option<&str>, default: &'static str, installed: &[String]) -> Family {
    let found = stored.filter(|s| installed.iter().any(|name| name == s));
    Family {
        name: found.map_or_else(|| default.into(), |s| SharedString::from(s.to_string())),
        stored: stored.map(str::to_string),
        not_installed: stored.is_some() && found.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn unset_settings_resolve_to_defaults() {
        let with_plex = resolve(&SettingsFile::default(), &installed(&["IBM Plex Mono", "Arial"]));
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
        let a = resolve(&file, &installed(&["Inter", "JetBrains Mono", "IBM Plex Mono"]));
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

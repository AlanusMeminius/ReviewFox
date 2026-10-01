//! macOS Finder-style vibrancy under the GPUI Metal view.
//!
//! Window uses `Transparent` (not full-window `Blurred`); this view supplies
//! `NSVisualEffectMaterial::Sidebar` under the full content view of each window.
//! Its appearance follows Software Theme independently of the system appearance.

use gpui::Window;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::code_theme::{self, SoftwareThemeMode};

/// Owns one `NSVisualEffectView` inserted below the GPUI view.
pub struct ColumnVibrancy {
    view: Retained<NSVisualEffectView>,
    mode: SoftwareThemeMode,
}

impl ColumnVibrancy {
    /// Lazy-attach and sync vibrancy under the entire window content (main desk).
    pub fn ensure_synced_window(slot: &mut Option<Self>, window: &Window) {
        if slot.is_none() {
            *slot = Self::attach(window);
        }
        let size = window.viewport_size();
        if let Some(v) = slot.as_mut() {
            v.sync(f32::from(size.width), f32::from(size.height));
        }
    }

    fn attach(window: &Window) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        // Fully qualify: GPUI's Window also has `window_handle()` → AnyWindowHandle.
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };

        // SAFETY: gpui's AppKit handle is the live Metal NSView for this window.
        let gpui_view = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
        let content = unsafe { gpui_view.superview()? };

        let effect = NSVisualEffectView::initWithFrame(
            NSVisualEffectView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        );
        effect.setMaterial(NSVisualEffectMaterial::Sidebar);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::FollowsWindowActiveState);

        content.addSubview_positioned_relativeTo(
            &effect,
            NSWindowOrderingMode::Below,
            Some(gpui_view),
        );

        let mode = code_theme::active_mode();
        effect.setAppearance(Some(&appearance_for_mode(mode)));
        Some(Self { view: effect, mode })
    }

    /// Sync full-window material size and live Software Theme changes.
    fn sync(&mut self, width_px: f32, height_px: f32) {
        let mode = code_theme::active_mode();
        if self.mode != mode {
            self.view.setAppearance(Some(&appearance_for_mode(mode)));
            self.mode = mode;
        }
        if width_px <= 0.0 || height_px <= 0.0 {
            self.view.setHidden(true);
            self.view
                .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)));
            return;
        }
        // AppKit origin is bottom-left; y=0 spans the entire content view.
        self.view.setHidden(false);
        self.view.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width_px as f64, height_px as f64),
        ));
    }
}

impl Drop for ColumnVibrancy {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
    }
}

/// Resolve native material appearance from the manual app choice, never the OS.
fn appearance_for_mode(mode: SoftwareThemeMode) -> Retained<NSAppearance> {
    // SAFETY: AppKit exports these immutable appearance-name constants.
    let name = unsafe {
        match mode {
            SoftwareThemeMode::Light => NSAppearanceNameAqua,
            SoftwareThemeMode::Dark => NSAppearanceNameDarkAqua,
        }
    };
    NSAppearance::appearanceNamed(name).expect("macOS provides Aqua and Dark Aqua")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_material_appearance_follows_software_theme_in_both_directions() {
        for mode in [
            SoftwareThemeMode::Light,
            SoftwareThemeMode::Dark,
            SoftwareThemeMode::Light,
        ] {
            let expected = unsafe {
                match mode {
                    SoftwareThemeMode::Light => NSAppearanceNameAqua,
                    SoftwareThemeMode::Dark => NSAppearanceNameDarkAqua,
                }
            };
            assert_eq!(&*appearance_for_mode(mode).name(), expected);
        }
    }
}

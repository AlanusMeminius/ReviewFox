//! macOS Finder-style vibrancy under the GPUI Metal view.
//!
//! Window uses `Transparent` (not full-window `Blurred`); this view supplies
//! `NSVisualEffectMaterial::Sidebar` under a left column (Diff tree) or the
//! full content view (main window frosted desk).

use gpui::Window;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::splitter;

/// Owns one `NSVisualEffectView` inserted below the GPUI view.
pub struct ColumnVibrancy {
    view: Retained<NSVisualEffectView>,
}

impl ColumnVibrancy {
    /// Lazy-attach and sync a full-height left strip (Diff tree column).
    ///
    /// `column_width` is the pane width (`0` when collapsed). When expanded,
    /// includes [`splitter::HANDLE_WIDTH`] so the leading resize hit stays frosted.
    pub fn ensure_synced(slot: &mut Option<Self>, window: &Window, column_width: f32) {
        if slot.is_none() {
            *slot = Self::attach(window);
        }
        let width = if column_width <= 0. {
            0.
        } else {
            column_width + splitter::HANDLE_WIDTH
        };
        let height = f32::from(window.viewport_size().height);
        if let Some(v) = slot.as_ref() {
            v.sync(width, height);
        }
    }

    /// Lazy-attach and sync vibrancy under the entire window content (main desk).
    pub fn ensure_synced_window(slot: &mut Option<Self>, window: &Window) {
        if slot.is_none() {
            *slot = Self::attach(window);
        }
        let size = window.viewport_size();
        if let Some(v) = slot.as_ref() {
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

        Some(Self { view: effect })
    }

    /// Sync the left-column frame. `width_px == 0` hides the effect (collapsed).
    fn sync(&self, width_px: f32, height_px: f32) {
        if width_px <= 0.0 || height_px <= 0.0 {
            self.view.setHidden(true);
            self.view.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(0.0, 0.0),
            ));
            return;
        }
        // Full-height left strip: AppKit origin is bottom-left; y=0 spans the
        // content view when height == viewport height.
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

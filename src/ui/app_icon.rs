//! macOS application icon: `assets/icon.ico` decoded by NSImage and set as the
//! Dock / app-switcher icon at runtime.
//!
//! The raw ICO is a full-bleed square, while bundled apps get the system
//! squircle mask with ~10% padding. Runtime-set icons skip that mask, so we
//! draw the square inset into a rounded-rect clip to match the Dock's grid.
//!
//! ponytail: dev-binary only — a shipped `.app` should carry an `icns` in its
//! bundle instead.

use objc2::{AllocAnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSBezierPath, NSImage};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};

const ICON_ICO: &[u8] = include_bytes!("../../assets/icon.ico");

/// Dock icons render with ~10% padding per side to match the system squircle
/// grid; corner radius ~22.37% of the artwork (rounded-rect approximation).
const CANVAS: f64 = 256.0;
const INSET: f64 = CANVAS * 0.10;
const CORNER_RADIUS: f64 = (CANVAS - 2.0 * INSET) * 0.2237;

/// Draw `image` inset with rounded corners onto a transparent canvas.
fn dock_icon(image: &NSImage) -> Retained<NSImage> {
    let artwork = NSRect::new(
        NSPoint::new(INSET, INSET),
        NSSize::new(CANVAS - 2.0 * INSET, CANVAS - 2.0 * INSET),
    );
    let icon = NSImage::initWithSize(NSImage::alloc(), NSSize::new(CANVAS, CANVAS));
    icon.lockFocus();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        artwork,
        CORNER_RADIUS,
        CORNER_RADIUS,
    )
    .addClip();
    unsafe { image.drawInRect(artwork) };
    icon.unlockFocus();
    icon
}

/// Set the application icon shown in the Dock and the app switcher.
pub fn set_app_icon() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(ICON_ICO);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let icon = dock_icon(&image);
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&icon)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_asset_decodes_to_non_empty_image() {
        let data = NSData::with_bytes(ICON_ICO);
        let image = NSImage::initWithData(NSImage::alloc(), &data).expect("ICO decodes");
        assert!(image.size().width > 0.0);
    }
}

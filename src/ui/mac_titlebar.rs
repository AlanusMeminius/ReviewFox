//! Compatibility adapter for Zed's app-owned titlebar on GPUI 0.2.2.
//!
//! Upstream: zed-industries/zed#60620 (23bb2fc135a69492847c3aa68444a7d14cc282f6).
//! GPUI 0.2.2 lacks `app_owns_titlebar_drag` and macOS `start_window_move`.
//! Apply the same NSView selector only to our custom-titlebar windows; keep
//! NSWindow movable so macOS window tiling remains available.

use std::ffi::CString;

use gpui::Window;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{NSApplication, NSView};
use objc2_foundation::NSRect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn take_ownership(window: &Window) {
    let Some(view) = native_view(window) else {
        return;
    };
    let original = view.class();
    if original.name().to_bytes().starts_with(b"ReviewFoxAppOwned") {
        return;
    }
    let name = CString::new(format!(
        "ReviewFoxAppOwned{}",
        original.name().to_string_lossy()
    ))
    .expect("Objective-C class name has no embedded null");
    let class = AnyClass::get(&name).unwrap_or_else(|| {
        let mut builder = ClassBuilder::new(&name, original)
            .expect("titlebar subclass is registered on the main thread");
        // SAFETY: identical NSRect-returning selector/signature to Zed's
        // GPUIView implementation. No instance variables or other overrides.
        unsafe {
            builder.add_method(
                sel!(_opaqueRectForWindowMoveWhenInTitlebar),
                app_owned_bounds as extern "C" fn(*mut AnyObject, Sel) -> NSRect,
            );
        }
        builder.register()
    });
    // SAFETY: the subclass inherits this exact GPUI view class, adds no ivars,
    // and changes only AppKit's titlebar ownership query. GPUI's state and all
    // event/lifetime methods stay inherited. Other windows retain their class.
    unsafe { AnyObject::set_class(view, class) };
}

extern "C" fn app_owned_bounds(view: *mut AnyObject, _: Sel) -> NSRect {
    // SAFETY: called only on our NSView subclass; NSView.bounds returns NSRect.
    unsafe { msg_send![view, bounds] }
}

/// Same native operation as modern GPUI's macOS `start_window_move`.
pub(super) fn start_window_move(window: &Window) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(view) = native_view(window) else {
        return;
    };
    let Some(native_window) = view.window() else {
        return;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return;
    };
    native_window.performWindowDragWithEvent(&event);
}

fn native_view(window: &Window) -> Option<&NSView> {
    MainThreadMarker::new()?;
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: GPUI owns the view for the borrowed Window's lifetime. Callers
    // run on the main thread and never store or retain this reference.
    Some(unsafe { handle.ns_view.cast::<NSView>().as_ref() })
}

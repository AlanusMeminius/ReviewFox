//! Caption buttons for the Windows client-decorated windows.
//!
//! `titlebar.appears_transparent` makes gpui drop the native title bar on Windows, so minimize /
//! maximize / close have to be drawn and tagged with [`WindowControlArea`] for the OS to act on.

#[cfg(target_os = "windows")]
use gpui::{
    Div, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, Window,
    WindowControlArea, div, px,
};
#[cfg(not(target_os = "windows"))]
use gpui::{Div, Window};

#[cfg(target_os = "windows")]
use super::theme;

/// Minimize / maximize / close, laid out left to right; caller places them in the corner.
#[cfg(target_os = "windows")]
pub fn window_controls(window: &Window) -> Vec<gpui::Stateful<Div>> {
    let colors = theme::software_palette().window_control;
    let maximize_glyph = if window.is_maximized() {
        "\u{e923}"
    } else {
        "\u{e922}"
    };
    [
        ("window-minimize", "\u{e921}", WindowControlArea::Min),
        ("window-maximize", maximize_glyph, WindowControlArea::Max),
        ("window-close", "\u{e8bb}", WindowControlArea::Close),
    ]
    .into_iter()
    .map(|(id, glyph, area)| {
        div()
            .id(id)
            .w(px(36.))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .font_family("Segoe Fluent Icons")
            .text_size(px(10.))
            .bg(colors.idle)
            .text_color(colors.icon)
            .window_control_area(area)
            // Blocks the hit test from reaching the root's focus hitbox, whose mouse-down
            // listener calls `prevent_default` and makes Windows treat the click as consumed.
            .occlude()
            .hover(move |button| {
                if area == WindowControlArea::Close {
                    button.bg(colors.close_hover).text_color(colors.close_icon)
                } else {
                    button.bg(colors.hover)
                }
            })
            .active(move |button| {
                if area == WindowControlArea::Close {
                    button
                        .bg(colors.close_pressed)
                        .text_color(colors.close_icon)
                } else {
                    button.bg(colors.pressed)
                }
            })
            .child(glyph)
    })
    .collect()
}

/// Other platforms keep their native decorations; empty so call sites need no `cfg`.
#[cfg(not(target_os = "windows"))]
pub fn window_controls(_: &Window) -> Vec<gpui::Stateful<Div>> {
    Vec::new()
}

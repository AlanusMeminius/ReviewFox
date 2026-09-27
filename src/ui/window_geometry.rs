//! GPUI glue for window geometry restore / observe (ADR-0007).

use gpui::{App, Bounds, Context, Pixels, Size, Timer, Window, point, px, size};
use std::time::Duration;

use crate::window_geometry_store::{self, StoredBounds};

pub fn main_default_size() -> Size<Pixels> {
    size(px(1280.), px(820.))
}

pub fn diff_default_size() -> Size<Pixels> {
    size(px(1100.), px(720.))
}

const DEBOUNCE: Duration = Duration::from_millis(300);

pub fn stored_from_window(window: &Window) -> StoredBounds {
    let b = window.bounds();
    StoredBounds {
        x: f32::from(b.origin.x),
        y: f32::from(b.origin.y),
        width: f32::from(b.size.width),
        height: f32::from(b.size.height),
    }
}

/// Restore stored bounds if they intersect a visible display; else centered default size.
pub fn resolve_bounds(
    stored: Option<&StoredBounds>,
    default_size: Size<Pixels>,
    cx: &App,
) -> Bounds<Pixels> {
    let fallback = Bounds::centered(None, default_size, cx);
    let Some(stored) = stored else {
        return fallback;
    };
    if stored.width < 200. || stored.height < 200. {
        return fallback;
    }
    let bounds = Bounds {
        origin: point(px(stored.x), px(stored.y)),
        size: size(px(stored.width), px(stored.height)),
    };
    let displays: Vec<_> = cx
        .displays()
        .iter()
        .map(|d| {
            let b = d.bounds();
            (
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            )
        })
        .collect();
    if window_geometry_store::bounds_intersects_any(
        stored.x,
        stored.y,
        stored.width,
        stored.height,
        &displays,
    ) {
        bounds
    } else {
        fallback
    }
}

/// Debounce disk flush after in-memory bounds update. Generation ignores stale timers.
pub fn debounce_flush<V: 'static>(cx: &mut Context<V>) {
    let generation = window_geometry_store::next_save_generation();
    cx.spawn(async move |this, cx| {
        Timer::after(DEBOUNCE).await;
        this.update(cx, |_, _| {
            window_geometry_store::flush_if_generation(generation);
        })
        .ok();
    })
    .detach();
}

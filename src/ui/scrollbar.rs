//! Overlay vertical scrollbar, Zed-shaped: thin thumb on the scroll container,
//! driven by a tracked [`ScrollHandle`] / [`UniformListScrollHandle`].
//!
//! Ported from BeadsViewer. Auto-hides after idle; shows again on scroll, track
//! hover, or drag. Handles live in a Global registry — `use_keyed_state` cannot
//! run during Render::render.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    App, Context, CursorStyle, Div, Entity, Global, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollHandle, Task, Timer, Window, canvas, div,
    point, prelude::*, px, rgb,
};

pub(crate) const THUMB_WIDTH: f32 = 6.;
pub(crate) const TRACK_WIDTH: f32 = 10.;
const MIN_THUMB: f32 = 24.;
pub(crate) const PAD: f32 = 4.;
pub(crate) const HIDE_DELAY: Duration = Duration::from_secs(1);
pub(crate) const THUMB_IDLE: u32 = 0xd8dde6;
pub(crate) const THUMB_ACTIVE: u32 = 0xb8c0cc;

struct Handle(ScrollHandle);

impl Handle {
    fn offset_y(&self) -> Pixels {
        self.0.offset().y
    }

    fn max_offset_y(&self) -> Pixels {
        self.0.max_offset().height
    }

    fn viewport_height(&self) -> Pixels {
        self.0.bounds().size.height
    }

    fn set_offset_y(&self, y: Pixels) {
        let x = self.0.offset().x;
        self.0.set_offset(point(x, y));
    }

    fn clone_scroll(&self) -> ScrollHandle {
        self.0.clone()
    }
}

pub(crate) struct ThumbGeom {
    pub(crate) thumb_height: Pixels,
    pub(crate) thumb_top: Pixels,
    max_offset: Pixels,
    pub(crate) track_height: Pixels,
}

impl ThumbGeom {
    /// Pure geometry seam: viewport + max scroll offset + current offset → thumb.
    pub(crate) fn from_metrics(
        viewport: Pixels,
        max_offset: Pixels,
        offset_y: Pixels,
    ) -> Option<Self> {
        let track_height = (viewport - px(PAD * 2.)).max(px(0.));
        if max_offset <= px(0.) || track_height <= px(0.) {
            return None;
        }
        let content = viewport + max_offset;
        let thumb_height = (track_height * (viewport / content)).max(px(MIN_THUMB));
        let travel = (track_height - thumb_height).max(px(0.));
        let ratio = (-offset_y / max_offset).clamp(0., 1.);
        Some(Self {
            thumb_height,
            thumb_top: travel * ratio,
            max_offset,
            track_height,
        })
    }

    /// Scroll offset (negative, like `ScrollHandle`) that puts the thumb top at
    /// `thumb_top` within the track.
    pub(crate) fn offset_for(&self, thumb_top: Pixels) -> Pixels {
        let travel = (self.track_height - self.thumb_height).max(px(0.));
        let ratio = if travel > px(0.) {
            (thumb_top / travel).clamp(0., 1.)
        } else {
            0.
        };
        -self.max_offset * ratio
    }

    fn compute(handle: &Handle) -> Option<Self> {
        Self::from_metrics(
            handle.viewport_height(),
            handle.max_offset_y(),
            handle.offset_y(),
        )
    }
}

/// Map a driver's scroll offset onto a peer that may have a different max_offset.
#[cfg(test)]
pub fn peer_offset(driver_y: Pixels, driver_max: Pixels, peer_max: Pixels) -> Pixels {
    if driver_max <= px(0.) {
        return px(0.);
    }
    let ratio = (-driver_y / driver_max).clamp(0., 1.);
    -peer_max * ratio
}

/// Lockstep scroll for panes that share content height (Diff L / gutter / R).
#[cfg(test)]
pub fn sync_lockstep(handles: &[&ScrollHandle], last_y: &mut Option<Pixels>) {
    if handles.is_empty() {
        return;
    }
    let ys: Vec<_> = handles.iter().map(|h| h.offset().y).collect();
    let driver_y = match *last_y {
        // First observation: never prefer "handle[0] at 0" over a pane that already moved.
        None => ys.iter().copied().find(|&y| y != px(0.)).unwrap_or(ys[0]),
        Some(last) => match ys.iter().copied().find(|&y| y != last) {
            Some(y) => y,
            None => return,
        },
    };
    if Some(driver_y) == *last_y {
        return;
    }
    for h in handles {
        let x = h.offset().x;
        h.set_offset(point(x, driver_y));
    }
    *last_y = Some(driver_y);
}

pub struct VerticalScrollbar {
    handle: Handle,
    drag_grab: Option<Pixels>,
    last_offset: Option<Pixels>,
    visible: bool,
    hovered: bool,
    hide_task: Option<Task<()>>,
}

impl VerticalScrollbar {
    fn new() -> Self {
        Self {
            handle: Handle(ScrollHandle::new()),
            drag_grab: None,
            last_offset: None,
            visible: true,
            hovered: false,
            hide_task: None,
        }
    }

    pub fn div_handle(&self) -> ScrollHandle {
        self.handle.clone_scroll()
    }

    fn jump_to_thumb_top(&self, thumb_top: Pixels, geom: &ThumbGeom) {
        self.handle.set_offset_y(geom.offset_for(thumb_top));
    }

    fn thumb_shown(&self) -> bool {
        self.visible || self.hovered || self.drag_grab.is_some()
    }

    /// Show the thumb and (re)arm the idle hide timer. Safe to call from `render`
    /// — does not `notify` (the current frame already paints the new state).
    fn reveal(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.arm_hide(cx);
    }

    fn arm_hide(&mut self, cx: &mut Context<Self>) {
        self.hide_task = Some(cx.spawn(async move |this, cx| {
            Timer::after(HIDE_DELAY).await;
            this.update(cx, |this, cx| {
                this.hide_task.take();
                if this.hovered || this.drag_grab.is_some() {
                    return;
                }
                if this.visible {
                    this.visible = false;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}

impl Render for VerticalScrollbar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(geom) = ThumbGeom::compute(&self.handle) else {
            // Stay out of layout / hit-testing so the scroll child receives the wheel.
            return div()
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .w(px(0.))
                .h(px(0.))
                .into_any_element();
        };

        let offset = self.handle.offset_y();
        if self.last_offset != Some(offset) {
            self.last_offset = Some(offset);
            self.reveal(cx);
        } else if self.visible
            && self.hide_task.is_none()
            && !self.hovered
            && self.drag_grab.is_none()
        {
            // First paint (or after hover/drag ended without a pending timer).
            self.arm_hide(cx);
        }

        let thumb_top = geom.thumb_top;
        let thumb_h = geom.thumb_height;
        let dragging = self.drag_grab.is_some();
        let show_thumb = self.thumb_shown();
        let entity = cx.entity();
        let inset = px((TRACK_WIDTH - THUMB_WIDTH) / 2.);

        let track = div()
            .absolute()
            .top(px(PAD))
            .right(px(0.))
            .w(px(TRACK_WIDTH))
            .h(geom.track_height)
            .cursor(CursorStyle::Arrow);

        let thumb = div()
            .absolute()
            .top(thumb_top)
            .right(inset)
            .w(px(THUMB_WIDTH))
            .h(thumb_h)
            .rounded_full()
            .opacity(if show_thumb { 1. } else { 0. })
            .bg(if dragging {
                rgb(THUMB_ACTIVE)
            } else {
                rgb(THUMB_IDLE)
            });

        track
            .child(thumb)
            .child(
                canvas(
                    |_, _, _| (),
                    move |track, _, window, _| {
                        let entity = entity.clone();
                        window.on_mouse_event({
                            let entity = entity.clone();
                            move |event: &MouseDownEvent, _, _, cx| {
                                if event.button != MouseButton::Left
                                    || !track.contains(&event.position)
                                {
                                    return;
                                }
                                entity.update(cx, |this, cx| {
                                    let Some(geom) = ThumbGeom::compute(&this.handle) else {
                                        return;
                                    };
                                    let local_y = event.position.y - track.origin.y;
                                    let thumb_bottom = geom.thumb_top + geom.thumb_height;
                                    if local_y >= geom.thumb_top && local_y <= thumb_bottom {
                                        this.drag_grab = Some(local_y - geom.thumb_top);
                                    } else {
                                        let centered = local_y - geom.thumb_height / 2.;
                                        this.jump_to_thumb_top(centered, &geom);
                                        this.drag_grab = Some(geom.thumb_height / 2.);
                                    }
                                    this.visible = true;
                                    this.hide_task.take();
                                    cx.notify();
                                });
                            }
                        });

                        let entity_move = entity.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                            let hovering = track.contains(&event.position);
                            entity_move.update(cx, |this, cx| {
                                let mut dirty = false;
                                if this.hovered != hovering {
                                    this.hovered = hovering;
                                    if hovering {
                                        this.visible = true;
                                        this.hide_task.take();
                                    } else if this.drag_grab.is_none() {
                                        this.arm_hide(cx);
                                    }
                                    dirty = true;
                                }
                                if let Some(grab) = this.drag_grab {
                                    let Some(geom) = ThumbGeom::compute(&this.handle) else {
                                        return;
                                    };
                                    let local_y = event.position.y - track.origin.y;
                                    this.jump_to_thumb_top(local_y - grab, &geom);
                                    dirty = true;
                                }
                                if dirty {
                                    cx.notify();
                                }
                            });
                        });

                        let entity_up = entity;
                        window.on_mouse_event(move |event: &MouseUpEvent, _, _, cx| {
                            if event.button != MouseButton::Left {
                                return;
                            }
                            entity_up.update(cx, |this, cx| {
                                if this.drag_grab.take().is_some() {
                                    if !this.hovered {
                                        this.arm_hide(cx);
                                    }
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

#[derive(Default)]
struct Registry {
    divs: HashMap<String, Entity<VerticalScrollbar>>,
}

impl Global for Registry {}

fn registry(cx: &mut App) -> &mut Registry {
    if !cx.has_global::<Registry>() {
        cx.set_global(Registry::default());
    }
    cx.global_mut::<Registry>()
}

/// Stable vertical scrollbar for a `overflow_y_scroll` + `track_scroll` div (trailing edge).
pub fn vertical(id: impl Into<String>, cx: &mut App) -> (ScrollHandle, Entity<VerticalScrollbar>) {
    let id = id.into();
    if let Some(existing) = cx
        .try_global::<Registry>()
        .and_then(|reg| reg.divs.get(&id).cloned())
    {
        let handle = existing.read(cx).div_handle();
        return (handle, existing);
    }
    let entity = cx.new(|_| VerticalScrollbar::new());
    registry(cx).divs.insert(id, entity.clone());
    let handle = entity.read(cx).div_handle();
    (handle, entity)
}

/// Wrap scrolled content with an overlay thumb (`flex_1` shell).
///
/// Both children are `absolute inset_0` so they take the flex item's definite
/// size. In-flow `size_full` content next to the scrollbar entity was getting an
/// indefinite/min width, so `text_ellipsis` collapsed titles/paths to a few glyphs.
pub fn overlay_flex(content: impl IntoElement, scrollbar: Entity<VerticalScrollbar>) -> Div {
    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(div().absolute().inset_0().child(content))
        .child(div().absolute().inset_0().child(scrollbar))
}

/// Cap height (comments): shrink-to-fit up to `max_h`, overlay thumb on top.
pub fn overlay_max(
    max_height: Pixels,
    content: impl IntoElement,
    scrollbar: Entity<VerticalScrollbar>,
) -> Div {
    div()
        .relative()
        .w_full()
        .max_h(max_height)
        // Content stays in-flow so the shell can shrink-wrap; only the thumb overlays.
        .child(content)
        .child(div().absolute().inset_0().child(scrollbar))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_geom_none_without_overflow() {
        assert!(ThumbGeom::from_metrics(px(100.), px(0.), px(0.)).is_none());
    }

    #[test]
    fn thumb_geom_at_top_and_bottom() {
        let top = ThumbGeom::from_metrics(px(100.), px(100.), px(0.)).unwrap();
        assert_eq!(top.thumb_top, px(0.));

        let bottom = ThumbGeom::from_metrics(px(100.), px(100.), px(-100.)).unwrap();
        let travel = bottom.track_height - bottom.thumb_height;
        assert_eq!(bottom.thumb_top, travel);
    }

    #[test]
    fn offset_for_inverts_thumb_top() {
        let geom = ThumbGeom::from_metrics(px(100.), px(300.), px(-150.)).unwrap();
        assert_eq!(geom.offset_for(geom.thumb_top), px(-150.));
        assert_eq!(geom.offset_for(px(-5.)), px(0.));
        assert_eq!(geom.offset_for(px(1000.)), px(-300.));
    }

    #[test]
    fn peer_offset_scales_across_unequal_max() {
        assert_eq!(peer_offset(px(0.), px(200.), px(100.)), px(0.));
        assert_eq!(peer_offset(px(-200.), px(200.), px(100.)), px(-100.));
        assert_eq!(peer_offset(px(-100.), px(200.), px(100.)), px(-50.));
    }

    #[test]
    fn peer_offset_zero_driver_max() {
        assert_eq!(peer_offset(px(-10.), px(0.), px(100.)), px(0.));
    }

    #[test]
    fn sync_lockstep_keeps_right_scroll_when_last_y_unset() {
        // Bug: old code treated left@0 as the driver whenever last_y was None,
        // wiping a right-pane wheel scroll back to 0.
        let left = ScrollHandle::new();
        let gutter = ScrollHandle::new();
        let right = ScrollHandle::new();
        right.set_offset(point(px(0.), px(-80.)));
        let mut last_y = None;
        sync_lockstep(&[&left, &gutter, &right], &mut last_y);
        assert_eq!(right.offset().y, px(-80.));
        assert_eq!(left.offset().y, px(-80.));
        assert_eq!(gutter.offset().y, px(-80.));
        assert_eq!(last_y, Some(px(-80.)));
    }

    #[test]
    fn sync_lockstep_follows_whichever_pane_moved() {
        let left = ScrollHandle::new();
        let gutter = ScrollHandle::new();
        let right = ScrollHandle::new();
        let mut last_y = Some(px(0.));
        left.set_offset(point(px(0.), px(-40.)));
        sync_lockstep(&[&left, &gutter, &right], &mut last_y);
        assert_eq!(right.offset().y, px(-40.));
        assert_eq!(last_y, Some(px(-40.)));

        right.set_offset(point(px(0.), px(-90.)));
        sync_lockstep(&[&left, &gutter, &right], &mut last_y);
        assert_eq!(left.offset().y, px(-90.));
        assert_eq!(gutter.offset().y, px(-90.));
    }
}

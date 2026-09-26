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
    MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollHandle, Task, Timer, UniformListScrollHandle,
    Window, canvas, div, point, prelude::*, px, rgb,
};

const THUMB_WIDTH: f32 = 6.;
const TRACK_WIDTH: f32 = 10.;
const MIN_THUMB: f32 = 24.;
const PAD: f32 = 4.;
const HIDE_DELAY: Duration = Duration::from_secs(1);
const THUMB_IDLE: u32 = 0xd8dde6;
const THUMB_ACTIVE: u32 = 0xb8c0cc;

/// Which outer edge hosts the overlay thumb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Old / left code pane — thumb on the leading (left) edge.
    Leading,
    /// Default for lists and the new / right code pane.
    Trailing,
}

enum Handle {
    Div(ScrollHandle),
    List(UniformListScrollHandle),
}

impl Handle {
    fn offset_y(&self) -> Pixels {
        match self {
            Self::Div(h) => h.offset().y,
            Self::List(h) => h.0.borrow().base_handle.offset().y,
        }
    }

    fn max_offset_y(&self) -> Pixels {
        match self {
            Self::Div(h) => h.max_offset().height,
            Self::List(h) => h.0.borrow().base_handle.max_offset().height,
        }
    }

    fn viewport_height(&self) -> Pixels {
        match self {
            Self::Div(h) => h.bounds().size.height,
            Self::List(h) => h.0.borrow().base_handle.bounds().size.height,
        }
    }

    fn set_offset_y(&self, y: Pixels) {
        match self {
            Self::Div(h) => {
                let x = h.offset().x;
                h.set_offset(point(x, y));
            }
            Self::List(h) => {
                let base = &h.0.borrow().base_handle;
                let x = base.offset().x;
                base.set_offset(point(x, y));
            }
        }
    }
}

struct ThumbGeom {
    thumb_height: Pixels,
    thumb_top: Pixels,
    max_offset: Pixels,
    track_height: Pixels,
}

impl ThumbGeom {
    /// Pure geometry seam: viewport + max scroll offset + current offset → thumb.
    fn from_metrics(viewport: Pixels, max_offset: Pixels, offset_y: Pixels) -> Option<Self> {
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

    fn compute(handle: &Handle) -> Option<Self> {
        Self::from_metrics(
            handle.viewport_height(),
            handle.max_offset_y(),
            handle.offset_y(),
        )
    }
}

/// Map a driver's scroll offset onto a peer that may have a different max_offset.
pub fn peer_offset(driver_y: Pixels, driver_max: Pixels, peer_max: Pixels) -> Pixels {
    if driver_max <= px(0.) {
        return px(0.);
    }
    let ratio = (-driver_y / driver_max).clamp(0., 1.);
    -peer_max * ratio
}

/// Lockstep scroll for panes that share content height (Diff L / gutter / R).
pub fn sync_lockstep(handles: &[&ScrollHandle], last_y: &mut Option<Pixels>) {
    if handles.is_empty() {
        return;
    }
    let ys: Vec<_> = handles.iter().map(|h| h.offset().y).collect();
    let driver_y = ys
        .iter()
        .copied()
        .find(|&y| Some(y) != *last_y)
        .unwrap_or(ys[0]);
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
    edge: Edge,
    drag_grab: Option<Pixels>,
    last_offset: Option<Pixels>,
    visible: bool,
    hovered: bool,
    hide_task: Option<Task<()>>,
}

impl VerticalScrollbar {
    fn for_div(edge: Edge) -> Self {
        Self {
            handle: Handle::Div(ScrollHandle::new()),
            edge,
            drag_grab: None,
            last_offset: None,
            visible: true,
            hovered: false,
            hide_task: None,
        }
    }

    fn for_list(edge: Edge) -> Self {
        Self {
            handle: Handle::List(UniformListScrollHandle::new()),
            edge,
            drag_grab: None,
            last_offset: None,
            visible: true,
            hovered: false,
            hide_task: None,
        }
    }

    pub fn div_handle(&self) -> ScrollHandle {
        match &self.handle {
            Handle::Div(h) => h.clone(),
            Handle::List(_) => panic!("div_handle on list scrollbar"),
        }
    }

    pub fn list_handle(&self) -> UniformListScrollHandle {
        match &self.handle {
            Handle::List(h) => h.clone(),
            Handle::Div(_) => panic!("list_handle on div scrollbar"),
        }
    }

    fn jump_to_thumb_top(&self, thumb_top: Pixels, geom: &ThumbGeom) {
        let travel = (geom.track_height - geom.thumb_height).max(px(0.));
        let ratio = if travel > px(0.) {
            (thumb_top / travel).clamp(0., 1.)
        } else {
            0.
        };
        self.handle.set_offset_y(-geom.max_offset * ratio);
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
            return div().into_any_element();
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
        let edge = self.edge;
        let inset = px((TRACK_WIDTH - THUMB_WIDTH) / 2.);

        let mut track = div()
            .absolute()
            .top(px(PAD))
            .w(px(TRACK_WIDTH))
            .h(geom.track_height)
            .cursor(CursorStyle::Arrow);
        track = match edge {
            Edge::Trailing => track.right(px(0.)),
            Edge::Leading => track.left(px(0.)),
        };

        let mut thumb = div()
            .absolute()
            .top(thumb_top)
            .w(px(THUMB_WIDTH))
            .h(thumb_h)
            .rounded_full()
            .opacity(if show_thumb { 1. } else { 0. })
            .bg(if dragging {
                rgb(THUMB_ACTIVE)
            } else {
                rgb(THUMB_IDLE)
            });
        thumb = match edge {
            Edge::Trailing => thumb.right(inset),
            Edge::Leading => thumb.left(inset),
        };

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
    lists: HashMap<String, Entity<VerticalScrollbar>>,
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
    vertical_on(id, Edge::Trailing, cx)
}

/// Stable vertical scrollbar on a chosen outer edge.
pub fn vertical_on(
    id: impl Into<String>,
    edge: Edge,
    cx: &mut App,
) -> (ScrollHandle, Entity<VerticalScrollbar>) {
    let id = id.into();
    if let Some(existing) = cx
        .try_global::<Registry>()
        .and_then(|reg| reg.divs.get(&id).cloned())
    {
        let handle = existing.read(cx).div_handle();
        return (handle, existing);
    }
    let entity = cx.new(|_| VerticalScrollbar::for_div(edge));
    registry(cx).divs.insert(id, entity.clone());
    let handle = entity.read(cx).div_handle();
    (handle, entity)
}

/// Stable vertical scrollbar for a `uniform_list` + `track_scroll`.
pub fn list(
    id: impl Into<String>,
    cx: &mut App,
) -> (UniformListScrollHandle, Entity<VerticalScrollbar>) {
    let id = id.into();
    if let Some(existing) = cx
        .try_global::<Registry>()
        .and_then(|reg| reg.lists.get(&id).cloned())
    {
        let handle = existing.read(cx).list_handle();
        return (handle, existing);
    }
    let entity = cx.new(|_| VerticalScrollbar::for_list(Edge::Trailing));
    registry(cx).lists.insert(id, entity.clone());
    let handle = entity.read(cx).list_handle();
    (handle, entity)
}

/// Wrap scrolled content with an overlay thumb (`flex_1` shell).
pub fn overlay_flex(content: impl IntoElement, scrollbar: Entity<VerticalScrollbar>) -> Div {
    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(content)
        .child(scrollbar)
}

/// Fixed-height overlay shell (branch picker, comments strip).
pub fn overlay_box(content: impl IntoElement, scrollbar: Entity<VerticalScrollbar>) -> Div {
    div()
        .relative()
        .size_full()
        .child(content)
        .child(scrollbar)
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
    fn peer_offset_scales_across_unequal_max() {
        assert_eq!(peer_offset(px(0.), px(200.), px(100.)), px(0.));
        assert_eq!(peer_offset(px(-200.), px(200.), px(100.)), px(-100.));
        assert_eq!(peer_offset(px(-100.), px(200.), px(100.)), px(-50.));
    }

    #[test]
    fn peer_offset_zero_driver_max() {
        assert_eq!(peer_offset(px(-10.), px(0.), px(100.)), px(0.));
    }
}

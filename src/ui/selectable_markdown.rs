//! Formatted Markdown with mouse drag selection and ⌘/Ctrl+C copy of plain text.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Entity, FocusHandle, Focusable, Global,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, SharedString, Window, canvas, div, hsla, point, prelude::*,
};

use super::appearance::{self, UiTextSize};
use super::markdown::{self, MarkdownLink, MarkdownSegment, RenderSink};
use super::splitter;
use super::theme;

pub struct SelectableMarkdown {
    element_id: SharedString,
    focus_handle: FocusHandle,
    source: SharedString,
    plain: SharedString,
    links: Vec<MarkdownLink>,
    segments: Vec<MarkdownSegment>,
    anchor: usize,
    selected: Range<usize>,
    pending: bool,
}

impl SelectableMarkdown {
    pub fn new(
        element_id: impl Into<SharedString>,
        source: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            element_id: element_id.into(),
            focus_handle: cx.focus_handle(),
            source: source.into(),
            plain: SharedString::default(),
            links: Vec::new(),
            segments: Vec::new(),
            anchor: 0,
            selected: 0..0,
            pending: false,
        }
    }

    fn selected_text(&self) -> &str {
        let start = self.selected.start.min(self.plain.len());
        let end = self.selected.end.min(self.plain.len()).max(start);
        &self.plain[start..end]
    }

    fn copy(&self, cx: &App) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    fn select_to(&mut self, head: usize) {
        let head = head.min(self.plain.len());
        if head < self.anchor {
            self.selected = head..self.anchor;
        } else {
            self.selected = self.anchor..head;
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "a" if event.keystroke.modifiers.secondary() => {
                self.anchor = 0;
                self.selected = 0..self.plain.len();
                cx.notify();
                cx.stop_propagation();
            }
            "c" if event.keystroke.modifiers.secondary() => {
                self.copy(cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

fn position_for_plain_index(
    segments: &[MarkdownSegment],
    index: usize,
) -> Option<(Point<Pixels>, Pixels)> {
    for segment in segments {
        let Some(len) = layout_len(segment) else {
            continue;
        };
        let end = segment.plain_start + len;
        if index < segment.plain_start {
            break;
        }
        if index > end {
            continue;
        }
        let within = index - segment.plain_start;
        let position = segment.layout.position_for_index(within)?;
        return Some((position, segment.layout.line_height()));
    }
    None
}

fn layout_bounds(segment: &MarkdownSegment) -> Option<Bounds<Pixels>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| segment.layout.bounds())).ok()
}

fn layout_len(segment: &MarkdownSegment) -> Option<usize> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| segment.layout.len())).ok()
}

impl Focusable for SelectableMarkdown {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SelectableMarkdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fonts = markdown::fonts_from_app(cx);
        let mut sink = RenderSink::default();
        let body = markdown::render_with_sink(self.source.as_ref(), &fonts, &mut sink);
        self.plain = SharedString::from(sink.plain);
        self.links = sink.links;
        self.segments = sink.segments;
        if self.selected.end > self.plain.len() {
            self.selected = 0..0;
            self.anchor = 0;
            self.pending = false;
        }

        let entity = cx.entity();
        let selected = self.selected.clone();
        let segments = self.segments.clone();
        let links = self.links.clone();
        let plain_len = self.plain.len();
        let element_id = self.element_id.clone();

        div()
            .id(element_id)
            .track_focus(&self.focus_handle)
            .w_full()
            .cursor(CursorStyle::IBeam)
            .font_family(appearance::ui_font(cx))
            .ui_text_size(appearance::UI_FONT_SIZE_DEFAULT as f32, cx)
            .text_color(theme::text())
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div().relative().w_full().child(body).child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _cx| {
                            // Layout bounds of scrolled content can extend past the
                            // clip; hit-test only the visible intersection so a
                            // splitter drag below the MR card does not start a selection.
                            let hit_bounds = bounds.intersect(&window.content_mask().bounds);

                            if selected.start < selected.end
                                && let (
                                    Some((start_pos, start_height)),
                                    Some((end_pos, end_height)),
                                ) = (
                                    position_for_plain_index(&segments, selected.start),
                                    position_for_plain_index(&segments, selected.end),
                                )
                            {
                                paint_selection_quads(
                                    window,
                                    bounds,
                                    start_pos,
                                    start_height,
                                    end_pos,
                                    end_height,
                                );
                            }

                            let entity_down = entity.clone();
                            let segments_down = segments.clone();
                            window.on_mouse_event({
                                move |event: &MouseDownEvent, _, window, cx| {
                                    if event.button != MouseButton::Left {
                                        return;
                                    }
                                    if splitter::is_resizing()
                                        || !hit_bounds.contains(&event.position)
                                    {
                                        entity_down.update(cx, |this, cx| {
                                            if this.pending
                                                || this.selected.start != this.selected.end
                                            {
                                                this.pending = false;
                                                this.selected = 0..0;
                                                cx.notify();
                                            }
                                        });
                                        return;
                                    }
                                    entity_down.update(cx, |this, cx| {
                                        window.focus(&this.focus_handle);
                                        let ix = index_for_position(
                                            &segments_down,
                                            plain_len,
                                            event.position,
                                        );
                                        if event.click_count >= 3 {
                                            this.anchor = 0;
                                            this.selected = 0..this.plain.len();
                                            this.pending = false;
                                        } else if event.click_count == 2 {
                                            this.selected = word_range(&this.plain, ix);
                                            this.anchor = this.selected.start;
                                            this.pending = false;
                                        } else {
                                            this.anchor = ix;
                                            this.selected = ix..ix;
                                            this.pending = true;
                                        }
                                        cx.notify();
                                    });
                                }
                            });

                            let entity_move = entity.clone();
                            let segments_move = segments.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                                entity_move.update(cx, |this, cx| {
                                    if splitter::is_resizing() {
                                        if this.pending || this.selected.start != this.selected.end
                                        {
                                            this.pending = false;
                                            this.selected = 0..0;
                                            cx.notify();
                                        }
                                        return;
                                    }
                                    if !this.pending || !event.dragging() {
                                        return;
                                    }
                                    let ix = index_for_position(
                                        &segments_move,
                                        plain_len,
                                        event.position,
                                    );
                                    this.select_to(ix);
                                    cx.notify();
                                });
                            });

                            let entity_up = entity;
                            let segments_up = segments;
                            let links_up = links;
                            window.on_mouse_event(move |event: &MouseUpEvent, _, _, cx| {
                                if event.button != MouseButton::Left {
                                    return;
                                }
                                entity_up.update(cx, |this, cx| {
                                    let was_pending = this.pending;
                                    if this.pending {
                                        this.pending = false;
                                    }
                                    // Splitter may clear `is_resizing` before this
                                    // handler; never treat a release outside the
                                    // clipped markdown as a link click.
                                    if !hit_bounds.contains(&event.position) {
                                        cx.notify();
                                        return;
                                    }
                                    let ix = index_for_position(
                                        &segments_up,
                                        this.plain.len(),
                                        event.position,
                                    );
                                    if was_pending
                                        && this.selected.start == this.selected.end
                                        && let Some(url) = links_up
                                            .iter()
                                            .find(|link| link.range.contains(&ix))
                                            .map(|link| link.url.clone())
                                    {
                                        cx.open_url(&url);
                                    }
                                    cx.notify();
                                });
                            });
                        },
                    )
                    .absolute()
                    .size_full()
                    .inset_0(),
                ),
            )
    }
}

fn paint_selection_quads(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    start_pos: Point<Pixels>,
    start_height: Pixels,
    end_pos: Point<Pixels>,
    end_height: Pixels,
) {
    let color = hsla(0.58, 0.55, 0.72, 0.35);
    let clear = hsla(0., 0., 0., 0.);
    if start_pos.y == end_pos.y {
        window.paint_quad(gpui::PaintQuad {
            bounds: Bounds::from_corners(start_pos, point(end_pos.x, end_pos.y + end_height)),
            corner_radii: Default::default(),
            background: color.into(),
            border_widths: Default::default(),
            border_color: clear,
            border_style: Default::default(),
        });
        return;
    }
    window.paint_quad(gpui::PaintQuad {
        bounds: Bounds::from_corners(start_pos, point(bounds.right(), start_pos.y + start_height)),
        corner_radii: Default::default(),
        background: color.into(),
        border_widths: Default::default(),
        border_color: clear,
        border_style: Default::default(),
    });
    if end_pos.y > start_pos.y + start_height {
        window.paint_quad(gpui::PaintQuad {
            bounds: Bounds::from_corners(
                point(bounds.left(), start_pos.y + start_height),
                point(bounds.right(), end_pos.y),
            ),
            corner_radii: Default::default(),
            background: color.into(),
            border_widths: Default::default(),
            border_color: clear,
            border_style: Default::default(),
        });
    }
    window.paint_quad(gpui::PaintQuad {
        bounds: Bounds::from_corners(
            point(bounds.left(), end_pos.y),
            point(end_pos.x, end_pos.y + end_height),
        ),
        corner_radii: Default::default(),
        background: color.into(),
        border_widths: Default::default(),
        border_color: clear,
        border_style: Default::default(),
    });
}

fn index_for_position(
    segments: &[MarkdownSegment],
    plain_len: usize,
    position: Point<Pixels>,
) -> usize {
    if plain_len == 0 || segments.is_empty() {
        return 0;
    }

    let mut previous_end = 0usize;
    for segment in segments {
        let Some(bounds) = layout_bounds(segment) else {
            continue;
        };
        let Some(len) = layout_len(segment) else {
            continue;
        };
        let segment_end = (segment.plain_start + len).min(plain_len);

        if position.y > bounds.bottom() {
            previous_end = segment_end;
            continue;
        }
        if position.y < bounds.top() {
            return previous_end;
        }

        return match segment.layout.index_for_position(position) {
            Ok(ix) | Err(ix) => (segment.plain_start + ix).min(plain_len),
        };
    }
    plain_len
}

/// Stable selectable markdown field keyed by `id`.
pub fn view(
    id: impl Into<String>,
    source: impl Into<SharedString>,
    cx: &mut App,
) -> Entity<SelectableMarkdown> {
    let id = id.into();
    let source = source.into();
    let entity = if let Some(existing) = cx
        .try_global::<MarkdownRegistry>()
        .and_then(|reg| reg.0.get(&id).cloned())
    {
        existing
    } else {
        let entity = cx.new(|cx| SelectableMarkdown::new(id.clone(), source.clone(), cx));
        markdown_registry(cx).0.insert(id, entity.clone());
        entity
    };
    entity.update(cx, |this, cx| {
        if this.source != source {
            this.source = source;
            this.selected = 0..0;
            this.pending = false;
            this.anchor = 0;
            cx.notify();
        }
    });
    entity
}

#[derive(Default)]
struct MarkdownRegistry(HashMap<String, Entity<SelectableMarkdown>>);

impl Global for MarkdownRegistry {}

fn markdown_registry(cx: &mut App) -> &mut MarkdownRegistry {
    if !cx.has_global::<MarkdownRegistry>() {
        cx.set_global(MarkdownRegistry::default());
    }
    cx.global_mut::<MarkdownRegistry>()
}

fn word_range(content: &str, index: usize) -> Range<usize> {
    let index = index.min(content.len());
    if !content.is_char_boundary(index) {
        return 0..content.len();
    }
    let start = content[..index]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let end = content[index..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, _)| index + i)
        .unwrap_or(content.len());
    start..end
}

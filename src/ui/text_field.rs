use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, Render, ShapedLine, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::appearance::{self, UiTextSize};
use super::theme;

actions!(
    text_field,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Confirm,
    ]
);

/// Emitted on Enter (`Confirm`); the owner decides what committing means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldEvent {
    Confirm,
}

/// Visual variant. `Default` is the original full-width field; `Settings`
/// follows Zed's settings input (min 256px wide, focused border); `Number` is
/// the bare centered value inside a `NumberField`, which draws the frame;
/// `Search` is the frameless full-width query bar atop a picker popover;
/// `Draft` is the frameless body field inside the Diff draft dock, which draws
/// its own frame and is taller than one line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextFieldStyle {
    #[default]
    Default,
    Settings,
    Number,
    Search,
    Draft,
}

pub struct TextField {
    focus_handle: FocusHandle,
    style: TextFieldStyle,
    content: SharedString,
    placeholder: SharedString,
    masked: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    /// Draft multi-line: `(byte_start_in_content, shaped line without the trailing newline)`.
    last_lines: Option<Vec<(usize, ShapedLine)>>,
    last_bounds: Option<Bounds<Pixels>>,
    /// Horizontal scroll that keeps the cursor inside a field narrower than its text.
    scroll_x: Pixels,
    is_selecting: bool,
}

impl TextField {
    pub fn new(placeholder: impl Into<SharedString>, masked: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            style: TextFieldStyle::Default,
            content: "".into(),
            placeholder: placeholder.into(),
            masked,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_lines: None,
            last_bounds: None,
            scroll_x: px(0.),
            is_selecting: false,
        }
    }

    pub fn with_style(mut self, style: TextFieldStyle) -> Self {
        self.style = style;
        self
    }

    /// Makes the field a Tab stop at `index` (see `Window::focus_next`).
    pub fn tab_index(mut self, index: isize) -> Self {
        self.focus_handle = self.focus_handle.tab_index(index).tab_stop(true);
        self
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    /// Visual rows for Draft (and any content with newlines): one per soft line.
    pub fn visual_line_count(&self) -> usize {
        self.content.split('\n').count().max(1)
    }

    pub fn set_content(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    /// Insert `text` at the caret, replacing any selection — what typing does,
    /// for owners that need to place characters a key binding cannot produce.
    pub fn insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, text, window, cx);
    }

    fn display_text(&self, focused: bool) -> SharedString {
        if self.masked && !focused && !self.content.is_empty() {
            return "•".repeat(self.content.chars().count()).into();
        }
        self.content.clone()
    }
    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            // Draft keeps newlines in content (paint flattens via
            // `single_line_shape_text`). Other styles stay single-line.
            let text = if self.style == TextFieldStyle::Draft {
                text
            } else {
                text.replace('\n', " ")
            };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let Some(bounds) = self.last_bounds.as_ref() else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }

        if let Some(lines) = self.last_lines.as_ref() {
            let line_h = bounds.size.height / lines.len().max(1) as f32;
            let mut row = ((position.y - bounds.top()) / line_h).floor() as usize;
            row = row.min(lines.len().saturating_sub(1));
            let (start, line) = &lines[row];
            let local = line.closest_index_for_x(position.x - bounds.left());
            return (*start + local).min(self.content.len());
        }

        let Some(line) = self.last_layout.as_ref() else {
            return 0;
        };
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = clamp_range(range, self.content.len());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = clamp_range(range, self.content.len());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        // `new_selected_range` is relative to the inserted text; both ends offset
        // by `range.start` (not `range.end` — that skews the caret during IME).
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selected_range = clamp_range(self.selected_range.clone(), self.content.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;

        // Stale layout vs content (e.g. mid-IME before the next paint) — do not
        // assert; macOS calls this from nounwind ObjC and a panic aborts.
        if last_layout.text != self.display_text(true) {
            return None;
        }
        let utf8_index = last_layout.index_for_x(line_point.x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

/// Clamp a byte range into `0..len`, ensuring `start <= end`.
fn clamp_range(range: Range<usize>, len: usize) -> Range<usize> {
    let start = range.start.min(len);
    let end = range.end.min(len).max(start);
    start..end
}

struct TextElement {
    input: Entity<TextField>,
}

/// Caret / selection inset from the field's top and bottom edges.
const CARET_INSET: f32 = 3.;

struct PrepaintState {
    /// Single-line styles.
    line: Option<ShapedLine>,
    /// Draft multi-line: one shaped line per visual row, with content byte start.
    lines: Option<Vec<(usize, ShapedLine)>>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    scroll_x: Pixels,
    ink_nudge: Pixels,
    line_height: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let multiline = input.style == TextFieldStyle::Draft;
        let rows = if multiline {
            input.visual_line_count()
        } else {
            1
        };
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * rows as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let centered = input.style == TextFieldStyle::Number;
        let multiline = input.style == TextFieldStyle::Draft;
        let focused = input.focus_handle.is_focused(window);
        let content = input.display_text(focused);
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();
        let line_height = window.line_height();

        let (display_text, text_color) = if input.content.is_empty() {
            (
                input.placeholder.clone(),
                theme::software_palette().text.placeholder.into(),
            )
        } else {
            (content, style.color)
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let run_for = |text: &str| TextRun {
            len: text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        if multiline {
            let slices = draft_line_slices(&display_text);
            let mut shaped: Vec<(usize, ShapedLine)> = Vec::with_capacity(slices.len());
            for (start, piece) in &slices {
                let line = window.text_system().shape_line(
                    (*piece).to_owned().into(),
                    font_size,
                    &[run_for(piece)],
                    None,
                );
                shaped.push((*start, line));
            }
            let ink_nudge = shaped
                .first()
                .map(|(_, line)| ink_nudge_for_line(line, &display_text, font_size, cx))
                .unwrap_or(px(0.));

            let cursor_width = px(2.);
            let (line_ix, local) = cursor_line_local(&shaped, cursor, display_text.len());
            let (_, cursor_line) = &shaped[line_ix];
            let cursor_x = cursor_line.x_for_index(local);
            let row_top = bounds.top() + line_height * line_ix as f32;
            let chrome_top = row_top + px(CARET_INSET);
            let chrome_bottom = row_top + line_height - px(CARET_INSET);
            let left = bounds.left();

            let mut selection = Vec::new();
            let cursor_quad = if selected_range.is_empty() {
                Some(fill(
                    Bounds::new(
                        point(left + cursor_x, chrome_top),
                        size(cursor_width, chrome_bottom - chrome_top),
                    ),
                    theme::software_palette().field.caret,
                ))
            } else {
                let (a, b) = (selected_range.start, selected_range.end);
                let (start_ix, start_local) = cursor_line_local(&shaped, a, display_text.len());
                let (end_ix, end_local) = cursor_line_local(&shaped, b, display_text.len());
                for row in start_ix..=end_ix {
                    let (byte_start, line) = &shaped[row];
                    let x0 = if row == start_ix {
                        line.x_for_index(start_local)
                    } else {
                        px(0.)
                    };
                    let x1 = if row == end_ix {
                        line.x_for_index(end_local)
                    } else {
                        line.width
                    };
                    let top = bounds.top() + line_height * row as f32 + px(CARET_INSET);
                    let bottom = bounds.top() + line_height * (row as f32 + 1.) - px(CARET_INSET);
                    selection.extend(selection_quads(
                        left + x0,
                        left + x1.max(x0 + px(1.)),
                        top,
                        bottom,
                        theme::software_palette().field,
                    ));
                    let _ = byte_start;
                }
                None
            };

            return PrepaintState {
                line: None,
                lines: Some(shaped),
                cursor: cursor_quad,
                selection,
                scroll_x: px(0.),
                ink_nudge,
                line_height,
            };
        }

        // Single-line styles: never pass `\n` to shape_line.
        let display_text = single_line_shape_text(&display_text);
        let run = run_for(&display_text);
        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            let mark_start = marked_range.start.min(display_text.len());
            let mark_end = marked_range.end.min(display_text.len()).max(mark_start);
            vec![
                TextRun {
                    len: mark_start,
                    ..run.clone()
                },
                TextRun {
                    len: mark_end - mark_start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - mark_end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let line = window
            .text_system()
            .shape_line(display_text.clone(), font_size, &runs, None);
        let ink_nudge = ink_nudge_for_line(&line, &display_text, font_size, cx);

        let cursor_width = px(2.);
        let visible = bounds.size.width - cursor_width;
        let cursor_x = line.x_for_index(cursor);
        let mut scroll_x = input.scroll_x.max(px(0.));
        if line.width <= visible {
            scroll_x = if centered {
                -((bounds.size.width - line.width) / 2.).floor()
            } else {
                px(0.)
            };
        } else {
            if cursor_x - scroll_x > visible {
                scroll_x = cursor_x - visible;
            }
            if cursor_x < scroll_x {
                scroll_x = cursor_x;
            }
            if scroll_x > line.width - visible {
                scroll_x = line.width - visible;
            }
        }
        let left = bounds.left() - scroll_x;
        let chrome_top = bounds.top() + px(CARET_INSET);
        let chrome_bottom = bounds.bottom() - px(CARET_INSET);
        let (selection, cursor_quad) = if selected_range.is_empty() {
            (
                Vec::new(),
                Some(fill(
                    Bounds::new(
                        point(left + cursor_x, chrome_top),
                        size(cursor_width, chrome_bottom - chrome_top),
                    ),
                    theme::software_palette().field.caret,
                )),
            )
        } else {
            (
                selection_quads(
                    left + line.x_for_index(selected_range.start),
                    left + line.x_for_index(selected_range.end),
                    chrome_top,
                    chrome_bottom,
                    theme::software_palette().field,
                ),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            lines: None,
            cursor: cursor_quad,
            selection,
            scroll_x,
            ink_nudge,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let scroll_x = prepaint.scroll_x;
        let ink_nudge = prepaint.ink_nudge;
        let line_height = prepaint.line_height;
        let hit_bounds = Bounds::new(
            point(bounds.left() - scroll_x, bounds.top()),
            size(bounds.size.width + scroll_x, bounds.size.height),
        );

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            if let Some(lines) = prepaint.lines.take() {
                for (row, (_start, line)) in lines.iter().enumerate() {
                    let origin = point(
                        hit_bounds.origin.x,
                        hit_bounds.origin.y + line_height * row as f32 + ink_nudge,
                    );
                    line.paint(origin, line_height, window, cx).unwrap();
                }
                self.input.update(cx, |input, _cx| {
                    input.last_layout = None;
                    input.last_lines = Some(lines);
                    input.last_bounds = Some(hit_bounds);
                    input.scroll_x = scroll_x;
                });
            } else {
                let line = prepaint.line.take().unwrap();
                let paint_origin = point(hit_bounds.origin.x, hit_bounds.origin.y + ink_nudge);
                line.paint(paint_origin, line_height, window, cx).unwrap();
                self.input.update(cx, |input, _cx| {
                    input.last_layout = Some(line);
                    input.last_lines = None;
                    input.last_bounds = Some(hit_bounds);
                    input.scroll_x = scroll_x;
                });
            }

            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
    }
}

/// Optical y shift from the shaped line's metrics and a probe glyph's ink box.
fn ink_nudge_for_line(
    line: &ShapedLine,
    display_text: &str,
    font_size: Pixels,
    cx: &App,
) -> Pixels {
    let text = cx.text_system();
    let ascent: f32 = line.ascent.into();
    let descent: f32 = line.descent.into();
    let (ink_above, ink_below) = match display_text
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
    {
        Some((ix, ch)) => {
            let font_id = line
                .font_id_for_index(ix)
                .unwrap_or_else(|| text.resolve_font(&gpui::font(appearance::ui_font(cx))));
            match text.typographic_bounds(font_id, font_size, ch) {
                Ok(bounds) => appearance::ink_extents_from_bounds(bounds),
                Err(_) => (text.cap_height(font_id, font_size).into(), 0.),
            }
        }
        None => {
            let font_id = text.resolve_font(&gpui::font(appearance::ui_font(cx)));
            (text.cap_height(font_id, font_size).into(), 0.)
        }
    };
    px(appearance::ink_center_nudge(
        ascent, descent, ink_above, ink_below,
    ))
}

/// Keep the selection wash light enough for text contrast, and draw a strong
/// 1px edge at both ends of its height so the selected span remains obvious.
fn selection_quads(
    left: Pixels,
    right: Pixels,
    top: Pixels,
    bottom: Pixels,
    colors: theme::FieldColors,
) -> Vec<PaintQuad> {
    let right = right.max(left + px(1.));
    let width = right - left;
    vec![
        fill(
            Bounds::from_corners(point(left, top), point(right, bottom)),
            colors.selection,
        ),
        fill(
            Bounds::new(point(left, top), size(width, px(1.))),
            colors.selection_outline,
        ),
        fill(
            Bounds::new(point(left, bottom - px(1.)), size(width, px(1.))),
            colors.selection_outline,
        ),
    ]
}

impl Render for TextField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .key_context("TextField")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(|_, _: &Confirm, _, cx| cx.emit(TextFieldEvent::Confirm)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .map(|field| {
                let framed = |field: gpui::Div| {
                    field
                        .h(px(32.))
                        .line_height(px(32.))
                        .px_2()
                        .bg(theme::software_palette().field.surface)
                        .border_1()
                        .border_color(theme::software_palette().field.border)
                        .rounded(px(6.))
                };
                match self.style {
                    TextFieldStyle::Default => framed(field).w_full(),
                    TextFieldStyle::Settings => framed(field).min_w(px(256.)).focus(|field| {
                        field.border_color(theme::software_palette().field.focused_border)
                    }),
                    // NumberField outer is h(28)+p(2) → 24px content.
                    TextFieldStyle::Number => field.size_full().line_height(px(24.)).px_1(),
                    TextFieldStyle::Search => field
                        .w_full()
                        .h(px(theme::FIND_FIELD_HEIGHT))
                        .line_height(px(theme::FIND_FIELD_HEIGHT))
                        .px_2(),
                    // The dock owns the frame; the caret sits on the field's
                    // first line rather than centred in its taller box.
                    TextFieldStyle::Draft => field
                        .w_full()
                        .h_full()
                        .items_start()
                        .line_height(px(22.))
                        .px_2()
                        .py_1(),
                }
            })
            .ui_text_size(13., cx)
            .text_color(theme::software_palette().text.primary)
            .font_family(appearance::ui_font(cx))
            .child(TextElement { input: cx.entity() })
    }
}

/// Split Draft content into visual rows. Each entry is `(byte_start, text)`
/// where `text` never contains `\n` (the newline itself sits between rows).
fn draft_line_slices(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            out.push((start, &text[start..i]));
            start = i + ch.len_utf8();
        }
    }
    out.push((start, &text[start..]));
    if out.is_empty() {
        out.push((0, ""));
    }
    out
}

/// Map a content byte offset onto `(line_index, offset_within_shaped_line)`.
fn cursor_line_local(
    lines: &[(usize, ShapedLine)],
    offset: usize,
    content_len: usize,
) -> (usize, usize) {
    let offset = offset.min(content_len);
    if lines.is_empty() {
        return (0, 0);
    }
    for (ix, (start, line)) in lines.iter().enumerate() {
        let next_start = lines.get(ix + 1).map(|(s, _)| *s).unwrap_or(usize::MAX);
        if offset < next_start || ix + 1 == lines.len() {
            let local = offset.saturating_sub(*start).min(line.len());
            return (ix, local);
        }
        let _ = line;
    }
    let last = lines.len() - 1;
    (last, lines[last].1.len())
}

/// GPUI `shape_line` panics on `\n`. Non-Draft styles flatten; Draft uses
/// [`draft_line_slices`].
fn single_line_shape_text(text: &str) -> SharedString {
    if text.as_bytes().contains(&b'\n') {
        text.replace('\n', " ").into()
    } else {
        text.to_owned().into()
    }
}

impl gpui::EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{draft_line_slices, single_line_shape_text};

    #[test]
    fn draft_line_slices_splits_on_newlines() {
        assert_eq!(draft_line_slices("a\nb"), vec![(0, "a"), (2, "b")]);
        assert_eq!(draft_line_slices("a\n"), vec![(0, "a"), (2, "")]);
        assert_eq!(draft_line_slices(""), vec![(0, "")]);
        assert_eq!(draft_line_slices("plain"), vec![(0, "plain")]);
    }

    #[test]
    fn single_line_shape_text_flattens_newlines_without_changing_len() {
        let raw = "line one\nline two\n";
        let flat = single_line_shape_text(raw);
        assert_eq!(flat.as_ref(), "line one line two ");
        assert_eq!(flat.len(), raw.len());
        assert!(!flat.as_bytes().contains(&b'\n'));
    }
}

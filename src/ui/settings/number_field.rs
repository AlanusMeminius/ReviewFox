use std::ops::RangeInclusive;

use gpui::{
    Context, ElementId, Entity, EventEmitter, Focusable, KeyBinding, Render, Subscription, Window,
    actions, div, prelude::*, px,
};

use super::Button;
use crate::ui::text_field::{TextField, TextFieldEvent, TextFieldStyle};
use crate::ui::theme;

actions!(number_field, [StepUp, StepDown]);

/// Key context on the whole field; Up/Down reach it from the text inside.
const CONTEXT: &str = "NumberField";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", StepUp, Some(CONTEXT)),
        KeyBinding::new("down", StepDown, Some(CONTEXT)),
    ]
}

/// A value the user applied: a step, or typed text committed on blur / Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberFieldEvent {
    Change(u32),
}

/// Zed `NumberField` in edit mode, step 1 only: `−`, the editable value, `+`.
/// The owner applies [`NumberFieldEvent::Change`] and feeds outside changes
/// (e.g. a reset) back through [`Self::set_value`]. Only the value is a Tab
/// stop; the buttons are for the pointer, Up/Down step from the keyboard.
pub struct NumberField {
    id: ElementId,
    value: u32,
    range: RangeInclusive<u32>,
    input: Entity<TextField>,
    _subscriptions: Vec<Subscription>,
}

impl NumberField {
    pub fn new(
        id: impl Into<ElementId>,
        value: u32,
        range: RangeInclusive<u32>,
        tab_index: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextField::new("", false, cx)
                .with_style(TextFieldStyle::Number)
                .tab_index(tab_index)
        });
        input.update(cx, |field, cx| field.set_content(value.to_string(), cx));
        let input_focus = input.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe(&input, |field, _, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Confirm => field.commit(cx),
            }),
            cx.on_blur(&input_focus, window, |field, _, cx| field.commit(cx)),
        ];
        Self {
            id: id.into(),
            value,
            range,
            input,
            _subscriptions: subscriptions,
        }
    }

    /// The value changed from outside: show it, dropping any typed text.
    pub fn set_value(&mut self, value: u32, cx: &mut Context<Self>) {
        if value != self.value {
            self.value = value;
            self.sync_text(cx);
            cx.notify();
        }
    }

    /// Takes a typed value that differs from the current one without emitting
    /// [`NumberFieldEvent::Change`], for an owner that must apply it itself
    /// (e.g. while closing). Unparseable text goes back to the current value.
    pub fn take_pending(&mut self, cx: &mut Context<Self>) -> Option<u32> {
        let pending = parse(self.input.read(cx).content(), &self.range)
            .filter(|&value| value != self.value);
        if let Some(value) = pending {
            self.value = value;
            cx.notify();
        }
        self.sync_text(cx);
        pending
    }

    /// Blur / Enter: apply the typed value, or go back to the current one.
    fn commit(&mut self, cx: &mut Context<Self>) {
        if let Some(value) = self.take_pending(cx) {
            cx.emit(NumberFieldEvent::Change(value));
        }
    }

    /// `−` / `+` / Up / Down, from the typed value when it parses.
    fn step(&mut self, up: bool, cx: &mut Context<Self>) {
        let from = parse(self.input.read(cx).content(), &self.range).unwrap_or(self.value);
        self.apply(step(from, up, &self.range), cx);
        self.sync_text(cx);
    }

    fn apply(&mut self, value: u32, cx: &mut Context<Self>) {
        if value != self.value {
            self.value = value;
            cx.emit(NumberFieldEvent::Change(value));
            cx.notify();
        }
    }

    fn sync_text(&self, cx: &mut Context<Self>) {
        let text = self.value.to_string();
        if self.input.read(cx).content() != text {
            self.input
                .update(cx, |field, cx| field.set_content(text, cx));
        }
    }

    fn step_button(&self, up: bool, cx: &mut Context<Self>) -> Button {
        let (suffix, icon) = if up {
            ("inc", "plus.svg")
        } else {
            ("dec", "minus.svg")
        };
        Button::icon_only(
            ElementId::Name(format!("{}-{suffix}", self.id).into()),
            icon,
        )
        .disabled(!can_step(self.value, up, &self.range))
        .on_click(cx.listener(move |field, _, _, cx| field.step(up, cx)))
    }
}

impl EventEmitter<NumberFieldEvent> for NumberField {}

impl Render for NumberField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        div()
            .id(self.id.clone())
            .key_context(CONTEXT)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(28.))
            .p(px(2.))
            .bg(theme::white())
            .border_1()
            .border_color(if focused {
                theme::border_focused()
            } else {
                theme::line()
            })
            .rounded(px(6.))
            .on_action(cx.listener(|field, _: &StepUp, _, cx| field.step(true, cx)))
            .on_action(cx.listener(|field, _: &StepDown, _, cx| field.step(false, cx)))
            .child(self.step_button(false, cx))
            .child(div().w(px(40.)).h_full().child(self.input.clone()))
            .child(self.step_button(true, cx))
    }
}

/// Typed text as a value: a number (fractions round), clamped to `range`.
/// Anything else is `None`, and the field goes back to its value.
fn parse(text: &str, range: &RangeInclusive<u32>) -> Option<u32> {
    let value = text.trim().parse::<f32>().ok().filter(|v| v.is_finite())?;
    Some(
        value
            .round()
            .clamp(*range.start() as f32, *range.end() as f32) as u32,
    )
}

/// One step from `value` (up or down), held inside `range`.
fn step(value: u32, up: bool, range: &RangeInclusive<u32>) -> u32 {
    let next = if up {
        value.saturating_add(1)
    } else {
        value.saturating_sub(1)
    };
    next.clamp(*range.start(), *range.end())
}

/// Whether a step from `value` would change it (`−` / `+` enabled).
fn can_step(value: u32, up: bool, range: &RangeInclusive<u32>) -> bool {
    step(value, up, range) != value
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGE: RangeInclusive<u32> = 11..=15;

    #[test]
    fn typed_numbers_parse_round_and_clamp() {
        assert_eq!(parse("14", &RANGE), Some(14));
        assert_eq!(parse(" 12 ", &RANGE), Some(12));
        assert_eq!(parse("13.6", &RANGE), Some(14));
        assert_eq!(parse("99", &RANGE), Some(15));
        assert_eq!(parse("2", &RANGE), Some(11));
        assert_eq!(parse("-3", &RANGE), Some(11));
    }

    #[test]
    fn unparseable_text_is_ignored() {
        assert_eq!(parse("", &RANGE), None);
        assert_eq!(parse("abc", &RANGE), None);
        assert_eq!(parse("14px", &RANGE), None);
        assert_eq!(parse("NaN", &RANGE), None);
        assert_eq!(parse("inf", &RANGE), None);
    }

    #[test]
    fn steps_by_one_and_stops_at_the_bounds() {
        assert_eq!(step(13, true, &RANGE), 14);
        assert_eq!(step(13, false, &RANGE), 12);
        assert_eq!(step(15, true, &RANGE), 15);
        assert_eq!(step(11, false, &RANGE), 11);
        assert!(can_step(14, true, &RANGE));
        assert!(!can_step(15, true, &RANGE));
        assert!(can_step(12, false, &RANGE));
        assert!(!can_step(11, false, &RANGE));
    }
}

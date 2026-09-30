//! Light Code Theme choice: an outlined trigger and a list of built-in labels.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, KeyBinding,
    MouseDownEvent, Pixels, Render, SharedString, Subscription, Window, actions, anchored, canvas,
    deferred, div, point, prelude::*, px, relative,
};

use super::{Button, ButtonSize, ButtonStyle};
use crate::ui::appearance::{self, Appearance, UiTextSize};
use crate::ui::code_theme::{self, CodeThemePairing, SoftwareThemeMode};
use crate::ui::theme;

actions!(
    code_theme_picker,
    [
        Open, SelectPrev, SelectNext, Confirm, Dismiss, FocusNext, FocusPrev
    ]
);

/// Key context on the trigger (Enter / Space open the list).
const TRIGGER_CONTEXT: &str = "CodeThemePickerTrigger";
/// Key context on the open list. Deeper than Settings, so Esc closes the list
/// instead of the window.
const CONTEXT: &str = "CodeThemePicker";

const WIDTH: f32 = 210.;
const ROW_HEIGHT: f32 = 28.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", Open, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("space", Open, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrev, Some(CONTEXT)),
    ]
}

/// The built-in id the user picked. `one-light` clears the stored field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeThemePickerEvent {
    Confirm(SharedString),
}

/// Lists [`code_theme::builtin_catalog`] by label. The trigger shows the label
/// the light choice resolves to, which is One Light when the stored id is absent
/// or unknown.
pub struct CodeThemePicker {
    trigger_focus: FocusHandle,
    list_focus: FocusHandle,
    trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
    open: bool,
    selected: usize,
    _on_blur: Subscription,
}

impl CodeThemePicker {
    pub fn new(tab_index: isize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list_focus = cx.focus_handle().tab_stop(true);
        let on_blur = cx.on_blur(&list_focus, window, |picker, window, cx| {
            if window.is_window_active() {
                picker.open = false;
                cx.notify();
            }
        });
        Self {
            trigger_focus: cx.focus_handle().tab_index(tab_index).tab_stop(true),
            list_focus,
            trigger_bounds: Rc::new(Cell::new(Bounds::default())),
            open: false,
            selected: 0,
            _on_blur: on_blur,
        }
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("settings-code-theme-{suffix}").into())
    }

    fn open_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = selected_index(cx);
        self.open = true;
        window.focus(&self.list_focus);
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            window.focus(&self.trigger_focus);
            cx.notify();
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.dismiss(window, cx);
        } else {
            self.open_list(window, cx);
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(theme) = code_theme::builtin_catalog().get(self.selected) else {
            return;
        };
        let id: SharedString = theme.id.clone().into();
        self.dismiss(window, cx);
        cx.emit(CodeThemePickerEvent::Confirm(id));
    }

    fn nudge(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = code_theme::builtin_catalog().len();
        if !self.open || len == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, len as isize - 1) as usize;
        cx.notify();
    }

    fn render_trigger(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.trigger_bounds.clone();
        div()
            .relative()
            .key_context(TRIGGER_CONTEXT)
            .on_action(cx.listener(|picker, _: &Open, window, cx| picker.toggle(window, cx)))
            .capture_any_mouse_down(cx.listener(|picker, _, window, _| {
                // A press on the trigger must not focus it: the list would blur,
                // close, and the click would reopen it.
                if picker.open {
                    window.prevent_default();
                }
            }))
            .child(
                Button::new(self.element_id("trigger"), resolved_label(cx))
                    .style(ButtonStyle::Outlined)
                    .size(ButtonSize::Medium)
                    .end_icon("chevrons_up_down.svg")
                    .track_focus(&self.trigger_focus)
                    .on_click(cx.listener(|picker, event: &ClickEvent, window, cx| {
                        // Enter / Space open on key-down. The key-up click would
                        // reopen the list after a choice returned focus here.
                        if !matches!(event, ClickEvent::Keyboard(_)) {
                            picker.toggle(window, cx);
                        }
                    })),
            )
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let trigger_bounds = self.trigger_bounds.clone();
        div()
            .id(self.element_id("popover"))
            .key_context(CONTEXT)
            .track_focus(&self.list_focus)
            .occlude()
            .flex()
            .flex_col()
            .w(px(WIDTH))
            .overflow_hidden()
            .py_1()
            .bg(theme::white())
            .border_1()
            .border_color(theme::border_variant())
            .rounded(px(6.))
            .shadow_lg()
            .font_family(appearance::ui_font(cx))
            .ui_text_size(14., cx)
            .text_color(theme::text())
            .on_action(cx.listener(|picker, _: &SelectPrev, _, cx| picker.nudge(-1, cx)))
            .on_action(cx.listener(|picker, _: &SelectNext, _, cx| picker.nudge(1, cx)))
            .on_action(cx.listener(|picker, _: &Confirm, window, cx| picker.confirm(window, cx)))
            .on_action(cx.listener(|picker, _: &Dismiss, window, cx| picker.dismiss(window, cx)))
            .on_action(cx.listener(|picker, _: &FocusNext, window, cx| {
                picker.dismiss(window, cx);
                window.focus_next();
            }))
            .on_action(cx.listener(|picker, _: &FocusPrev, window, cx| {
                picker.dismiss(window, cx);
                window.focus_prev();
            }))
            .on_mouse_down_out(
                cx.listener(move |picker, event: &MouseDownEvent, window, cx| {
                    if !trigger_bounds.get().contains(&event.position) {
                        picker.dismiss(window, cx);
                    }
                }),
            )
            .children(
                code_theme::builtin_catalog()
                    .iter()
                    .enumerate()
                    .map(|(ix, entry)| {
                        let selected = ix == self.selected;
                        let label = entry.label.clone();
                        div()
                            .id(ix)
                            .px_1()
                            .h(px(ROW_HEIGHT))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .size_full()
                                    .px_2()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .when(selected, |row| row.bg(theme::element_active()))
                                    .when(!selected, |row| row.hover(|row| row.bg(theme::hover())))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(label),
                                    ),
                            )
                            .on_click(cx.listener(move |picker, _, window, cx| {
                                picker.selected = ix;
                                picker.confirm(window, cx);
                            }))
                    }),
            )
    }
}

fn resolved_light(cx: &App) -> code_theme::ResolvedCodeTheme {
    code_theme::resolve(
        SoftwareThemeMode::Light,
        &CodeThemePairing {
            light: cx.global::<Appearance>().code_theme_light.clone(),
            dark: None,
        },
        code_theme::builtin_catalog(),
    )
}

fn resolved_label(cx: &App) -> SharedString {
    resolved_light(cx).label.into()
}

fn selected_index(cx: &App) -> usize {
    let id = resolved_light(cx).id;
    code_theme::builtin_catalog()
        .iter()
        .position(|theme| theme.id == id)
        .unwrap_or(0)
}

impl EventEmitter<CodeThemePickerEvent> for CodeThemePicker {}

impl Render for CodeThemePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.open.then(|| self.render_list(cx).into_any_element());
        div()
            .relative()
            .flex_none()
            .child(self.render_trigger(cx))
            .when_some(list, |picker, list| {
                picker.child(
                    div().absolute().top(relative(1.)).left_0().child(
                        deferred(
                            anchored()
                                .offset(point(px(0.), px(2.)))
                                .snap_to_window_with_margin(px(8.))
                                .child(list),
                        )
                        .with_priority(1),
                    ),
                )
            })
    }
}

//! A small options dropdown: outlined trigger + list of (id, label) rows.
//! Used for the Software Theme choice and both Code Theme pairing choices.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, KeyBinding, KeyContext,
    MouseDownEvent, Pixels, Render, SharedString, Subscription, Window, actions, anchored, canvas,
    deferred, div, point, prelude::*, px, relative,
};

use super::{Button, ButtonSize, ButtonStyle};
use crate::ui::appearance::{self, Appearance, UiTextSize};
use crate::ui::code_theme::{self, CodeThemePairing, SoftwareThemeMode};
use crate::ui::theme;

actions!(
    options_picker,
    [
        Open, SelectPrev, SelectNext, Confirm, Dismiss, FocusNext, FocusPrev
    ]
);

const WIDTH: f32 = 210.;
const ROW_HEIGHT: f32 = 28.;

/// Key bindings for one picker instance; contexts are per `id_prefix` so two
/// pickers never answer the same keys.
pub fn key_bindings(id_prefix: &'static str) -> Vec<KeyBinding> {
    let trigger = format!("{id_prefix}-trigger");
    let list = format!("{id_prefix}-list");
    vec![
        KeyBinding::new("enter", Open, Some(&trigger)),
        KeyBinding::new("space", Open, Some(&trigger)),
        KeyBinding::new("up", SelectPrev, Some(&list)),
        KeyBinding::new("down", SelectNext, Some(&list)),
        KeyBinding::new("enter", Confirm, Some(&list)),
        KeyBinding::new("escape", Dismiss, Some(&list)),
        KeyBinding::new("tab", FocusNext, Some(&list)),
        KeyBinding::new("shift-tab", FocusPrev, Some(&list)),
    ]
}

/// The option id the user picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionsPickerEvent {
    Confirm(SharedString),
}

/// Lists `options` (id, label) by label. The trigger shows the label of the
/// current id; an unknown current id shows the first option.
pub struct OptionsPicker {
    id_prefix: &'static str,
    options: Vec<(SharedString, SharedString)>,
    current: SharedString,
    trigger_focus: FocusHandle,
    list_focus: FocusHandle,
    trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
    open: bool,
    selected: usize,
    _on_blur: Subscription,
}

impl OptionsPicker {
    pub fn new(
        id_prefix: &'static str,
        options: Vec<(SharedString, SharedString)>,
        current: SharedString,
        tab_index: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_focus = cx.focus_handle().tab_stop(true);
        let on_blur = cx.on_blur(&list_focus, window, |picker, window, cx| {
            if window.is_window_active() {
                picker.open = false;
                cx.notify();
            }
        });
        Self {
            id_prefix,
            options,
            current,
            trigger_focus: cx.focus_handle().tab_index(tab_index).tab_stop(true),
            list_focus,
            trigger_bounds: Rc::new(Cell::new(Bounds::default())),
            open: false,
            selected: 0,
            _on_blur: on_blur,
        }
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("{}-{suffix}", self.id_prefix).into())
    }

    fn current_index(&self) -> usize {
        self.options
            .iter()
            .position(|(id, _)| *id == self.current)
            .unwrap_or(0)
    }

    fn current_label(&self) -> SharedString {
        self.options
            .get(self.current_index())
            .map(|(_, label)| label.clone())
            .unwrap_or_default()
    }

    fn open_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.current_index();
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
        let Some((id, _)) = self.options.get(self.selected) else {
            return;
        };
        let id = id.clone();
        self.current = id.clone();
        self.dismiss(window, cx);
        cx.emit(OptionsPickerEvent::Confirm(id));
    }

    fn nudge(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.options.len();
        if !self.open || len == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, len as isize - 1) as usize;
        cx.notify();
    }

    fn render_trigger(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.trigger_bounds.clone();
        let trigger_context = format!("{}-trigger", self.id_prefix);
        div()
            .relative()
            .key_context(KeyContext::try_from(trigger_context.as_str()).unwrap())
            .on_action(cx.listener(|picker, _: &Open, window, cx| picker.toggle(window, cx)))
            .capture_any_mouse_down(cx.listener(|picker, _, window, _| {
                // A press on the trigger must not focus it: the list would blur,
                // close, and the click would reopen it.
                if picker.open {
                    window.prevent_default();
                }
            }))
            .child(
                Button::new(self.element_id("trigger"), self.current_label())
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
        let list_context = format!("{}-list", self.id_prefix);
        div()
            .id(self.element_id("popover"))
            .key_context(KeyContext::try_from(list_context.as_str()).unwrap())
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
            .children(self.options.iter().enumerate().map(|(ix, (_, label))| {
                let selected = ix == self.selected;
                let label = label.clone();
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
            }))
    }
}

impl EventEmitter<OptionsPickerEvent> for OptionsPicker {}

impl Render for OptionsPicker {
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

/// Options for either Code Theme choice: every built-in theme by label.
pub fn code_theme_options() -> Vec<(SharedString, SharedString)> {
    code_theme::builtin_catalog()
        .iter()
        .map(|entry| (entry.id.clone().into(), entry.label.clone().into()))
        .collect()
}

/// One stored choice resolved to a catalog id (unknown ids fall back by mode).
pub fn resolved_code_theme_id(cx: &App, mode: SoftwareThemeMode) -> SharedString {
    code_theme::resolve(
        mode,
        &CodeThemePairing {
            light: cx.global::<Appearance>().code_theme_light.clone(),
            dark: cx.global::<Appearance>().code_theme_dark.clone(),
        },
        code_theme::builtin_catalog(),
    )
    .id
    .into()
}

/// Options for the Software Theme choice.
pub fn software_theme_options() -> Vec<(SharedString, SharedString)> {
    vec![
        ("light".into(), "Light".into()),
        ("dark".into(), "Dark".into()),
    ]
}

/// The active Software Theme as its stored id.
pub fn current_software_theme_id(cx: &App) -> SharedString {
    match cx.global::<Appearance>().software_theme {
        SoftwareThemeMode::Light => "light",
        SoftwareThemeMode::Dark => "dark",
    }
    .into()
}

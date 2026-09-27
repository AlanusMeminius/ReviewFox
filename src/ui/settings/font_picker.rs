use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    Bounds, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    KeyBinding, ListSizingBehavior, MouseDownEvent, Pixels, Render, ScrollStrategy, SharedString,
    Subscription, UniformListScrollHandle, UniformListScrollState, Window, actions, anchored,
    canvas, deferred, div, point, prelude::*, px, relative, rems, uniform_list,
};

use super::font_list::FontList;
use super::{Button, ButtonSize, ButtonStyle};
use crate::ui::appearance::{self, Appearance, Family, FontRole, UiTextSize};
use crate::ui::scrollbar;
use crate::ui::text_field::{TextField, TextFieldEvent, TextFieldStyle};
use crate::ui::theme;

actions!(font_picker, [Open, SelectPrev, SelectNext, Dismiss]);

/// Key context on the trigger (Enter / Space open the popover).
const TRIGGER_CONTEXT: &str = "FontPickerTrigger";
/// Key context on the popover; deeper than `Settings`, so Esc dismisses the
/// popover instead of closing the window.
const CONTEXT: &str = "FontPicker";

const WIDTH: f32 = 210.;
const ROW_HEIGHT: f32 = 28.;
const NOT_INSTALLED: &str = "not installed";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", Open, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("space", Open, Some(TRIGGER_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
    ]
}

/// A font the user picked from the list; the owner stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontPickerEvent {
    Confirm(SharedString),
}

/// Zed `FontPicker` in a `PopoverMenu`: an Outlined trigger showing the
/// role's family, opening a searchable list of the installed fonts below it.
/// Only listed names can be chosen. The owner applies
/// [`FontPickerEvent::Confirm`]; the trigger reads the [`Appearance`] Global.
pub struct FontPicker {
    id: SharedString,
    role: FontRole,
    trigger_focus: FocusHandle,
    /// Window bounds of the trigger, so a mouse-down on it leaves the popover
    /// to the trigger's own click (which closes it).
    trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
    popover: Option<Popover>,
}

struct Popover {
    list: FontList,
    search: Entity<TextField>,
    /// Last query applied to `list`; the field also notifies on cursor moves.
    query: String,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl FontPicker {
    pub fn new(
        id: impl Into<SharedString>,
        role: FontRole,
        tab_index: isize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            role,
            trigger_focus: cx.focus_handle().tab_index(tab_index).tab_stop(true),
            trigger_bounds: Rc::new(Cell::new(Bounds::default())),
            popover: None,
        }
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("{}-{suffix}", self.id).into())
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popover.is_some() {
            self.dismiss(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    /// Fresh list and empty query, the selection on the rendered family and
    /// scrolled into view; the search field takes focus.
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.role.family(cx.global::<Appearance>()).name.clone();
        let fonts = appearance::installed_fonts(cx)
            .iter()
            .map(|name| SharedString::from(name.clone()))
            .collect();
        let list = FontList::new(fonts, current);

        let search = cx.new(|cx| {
            TextField::new("Search fonts…", false, cx).with_style(TextFieldStyle::Search)
        });
        let search_focus = search.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.observe(&search, |picker, search, cx| {
                let query = search.read(cx).content().to_string();
                if let Some(popover) = &mut picker.popover
                    && popover.query != query
                {
                    popover.list.set_query(&query);
                    popover.query = query;
                    popover
                        .scroll
                        .scroll_to_item(popover.list.selected_index(), ScrollStrategy::Top);
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &search,
                window,
                |picker, _, event, window, cx| match event {
                    TextFieldEvent::Confirm => picker.confirm(window, cx),
                },
            ),
            // Tab (or anything else) moving focus away closes it; focus stays
            // where it went.
            cx.on_blur(&search_focus, window, |picker, _, cx| {
                picker.popover = None;
                cx.notify();
            }),
        ];

        // The list scrolls through the registry's handle so the overlay
        // scrollbar tracks it; each open starts from the top.
        let (base_handle, _) = scrollbar::vertical(format!("{}-list-sb", self.id), cx);
        base_handle.set_offset(point(px(0.), px(0.)));
        let scroll = UniformListScrollHandle(Rc::new(RefCell::new(UniformListScrollState {
            base_handle,
            ..Default::default()
        })));
        scroll.scroll_to_item_strict(list.selected_index(), ScrollStrategy::Center);

        window.focus(&search_focus);
        self.popover = Some(Popover {
            list,
            search,
            query: String::new(),
            scroll,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Close and give focus back to the trigger (Esc, a choice, a click outside).
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popover.take().is_some() {
            window.focus(&self.trigger_focus);
            cx.notify();
        }
    }

    /// Enter / a row click: emit the selected font and close. Nothing when
    /// no font matches.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(font) = self
            .popover
            .as_ref()
            .and_then(|popover| popover.list.selected().cloned())
        else {
            return;
        };
        self.dismiss(window, cx);
        cx.emit(FontPickerEvent::Confirm(font));
    }

    fn select(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FontList)) {
        if let Some(popover) = &mut self.popover {
            change(&mut popover.list);
            popover
                .scroll
                .scroll_to_item(popover.list.selected_index(), ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn render_trigger(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let family = self.role.family(cx.global::<Appearance>());
        let bounds = self.trigger_bounds.clone();
        div()
            .relative()
            .key_context(TRIGGER_CONTEXT)
            .on_action(cx.listener(|picker, _: &Open, window, cx| picker.toggle(window, cx)))
            .child(
                Button::new(self.element_id("trigger"), trigger_label(family))
                    .style(ButtonStyle::Outlined)
                    .size(ButtonSize::Medium)
                    .end_icon("chevrons_up_down.svg")
                    .when(family.not_installed, |button| button.hint(NOT_INSTALLED))
                    .track_focus(&self.trigger_focus)
                    .on_click(cx.listener(|picker, event: &ClickEvent, window, cx| {
                        // Enter / Space go through `Open` on key-down; the
                        // key-up click would reopen the popover after Enter
                        // confirmed a font and focus came back here.
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

    fn render_popover(&self, popover: &Popover, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, scrollbar) = scrollbar::vertical(format!("{}-list-sb", self.id), cx);
        let count = popover.list.len();
        let trigger_bounds = self.trigger_bounds.clone();

        let list = if count == 0 {
            div()
                .px_2()
                .py(px(6.))
                .ui_text_size(14., cx)
                .text_color(theme::muted())
                .child("No matches")
                .into_any_element()
        } else {
            uniform_list(
                self.element_id("list"),
                count,
                cx.processor(|picker, range: Range<usize>, _, cx| picker.render_rows(range, cx)),
            )
            .track_scroll(popover.scroll.clone())
            // As tall as its rows, up to the list area's max height.
            .with_sizing_behavior(ListSizingBehavior::Infer)
            .flex_grow()
            .into_any_element()
        };

        div()
            .id(self.element_id("popover"))
            .key_context(CONTEXT)
            .occlude()
            .flex()
            .flex_col()
            .w(px(WIDTH))
            .overflow_hidden()
            .bg(theme::white())
            .border_1()
            .border_color(theme::border_variant())
            .rounded(px(6.))
            .shadow_lg()
            // Deferred content does not inherit the window's text style.
            .font_family(appearance::ui_font(cx))
            .ui_text_size(14., cx)
            .text_color(theme::text())
            .on_action(
                cx.listener(|picker, _: &SelectPrev, _, cx| {
                    picker.select(cx, FontList::select_prev)
                }),
            )
            .on_action(
                cx.listener(|picker, _: &SelectNext, _, cx| {
                    picker.select(cx, FontList::select_next)
                }),
            )
            .on_action(cx.listener(|picker, _: &Dismiss, window, cx| picker.dismiss(window, cx)))
            .on_mouse_down_out(
                cx.listener(move |picker, event: &MouseDownEvent, window, cx| {
                    if !trigger_bounds.get().contains(&event.position) {
                        picker.dismiss(window, cx);
                    }
                }),
            )
            .child(
                div()
                    .flex_none()
                    .border_b_1()
                    .border_color(theme::border_variant())
                    .child(popover.search.clone()),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_grow()
                    // Zed's picker caps the list area, not the whole popover.
                    .max_h(rems(18.))
                    .overflow_hidden()
                    .py_1()
                    .child(list)
                    .child(div().absolute().inset_0().child(scrollbar)),
            )
    }

    /// Rows in the normal UI Font (not each in its own face), inset like
    /// Zed's `ListItem`; a click selects and confirms.
    fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let Some(popover) = &self.popover else {
            return Vec::new();
        };
        range
            .filter_map(|ix| {
                let name = popover.list.get(ix)?.clone();
                let selected = ix == popover.list.selected_index();
                Some(
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
                                        .child(name),
                                ),
                        )
                        .on_click(cx.listener(move |picker, _, window, cx| {
                            if let Some(popover) = &mut picker.popover {
                                popover.list.select(ix);
                            }
                            picker.confirm(window, cx);
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

/// The trigger text: `Default (<resolved>)` when unset, else the stored
/// name, also when it is not installed (the trigger adds the hint).
fn trigger_label(family: &Family) -> SharedString {
    match &family.stored {
        None => format!("Default ({})", family.name).into(),
        Some(stored) => stored.clone().into(),
    }
}

impl EventEmitter<FontPickerEvent> for FontPicker {}

impl Render for FontPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .flex_none()
            .child(self.render_trigger(cx))
            .when_some(self.popover.as_ref(), |picker, popover| {
                // Anchored at the trigger's bottom-left, 2px below; deferred so
                // it paints above the page and escapes its scroll clip.
                picker.child(
                    div().absolute().top(relative(1.)).left_0().child(
                        deferred(
                            anchored()
                                .offset(point(px(0.), px(2.)))
                                .snap_to_window_with_margin(px(8.))
                                .child(self.render_popover(popover, cx)),
                        )
                        .with_priority(1),
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(name: &'static str, stored: Option<&str>, not_installed: bool) -> Family {
        Family {
            name: name.into(),
            stored: stored.map(str::to_string),
            not_installed,
        }
    }

    #[test]
    fn trigger_shows_default_with_the_resolved_family_when_unset() {
        let unset = family("IBM Plex Mono", None, false);
        assert_eq!(trigger_label(&unset), "Default (IBM Plex Mono)");
    }

    #[test]
    fn trigger_shows_the_stored_family_even_when_not_installed() {
        assert_eq!(
            trigger_label(&family("Inter", Some("Inter"), false)),
            "Inter"
        );
        let gone = family("IBM Plex Sans", Some("Gone Sans"), true);
        assert_eq!(trigger_label(&gone), "Gone Sans");
    }
}

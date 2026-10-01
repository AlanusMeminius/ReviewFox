//! Square icon-only chrome button: the titlebar toolbars' one control shape.

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, svg,
};

use super::theme;
use super::tooltip::Tooltip;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: &'static str,
    tooltip: SharedString,
    shortcut: Option<SharedString>,
    pressed: bool,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    /// `icon` is an asset path such as `"search.svg"`; `tooltip` names the action,
    /// since the button carries no text of its own.
    pub fn new(
        id: impl Into<ElementId>,
        icon: &'static str,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            tooltip: tooltip.into(),
            shortcut: None,
            pressed: false,
            disabled: false,
            on_click: None,
        }
    }

    /// Shown after the name in the tooltip.
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Toggle state: pressed buttons sit on the range tint with an accent glyph.
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let palette = theme::software_palette();
        let color = if self.disabled {
            palette.control.disabled_icon
        } else if self.pressed {
            palette.field.caret
        } else {
            palette.text.chrome_icon
        };
        let pressed = self.pressed;
        let enabled = !self.disabled;
        div()
            .id(self.id)
            .w(theme::TOGGLE_SIZE)
            .h(theme::TOGGLE_SIZE)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .when(pressed && enabled, |button| {
                button.bg(palette.control.selected)
            })
            .tooltip(Tooltip::text(self.tooltip, self.shortcut))
            .when(enabled, |button| {
                button
                    .hover(move |button| {
                        button.bg(if pressed {
                            palette.control.selected_hover
                        } else {
                            palette.control.hover
                        })
                    })
                    .active(move |button| {
                        button.bg(if pressed {
                            palette.control.selected_pressed
                        } else {
                            palette.control.pressed
                        })
                    })
            })
            .when_some(self.on_click.filter(|_| enabled), |button, handler| {
                button
                    .cursor_pointer()
                    .on_click(move |event, window, cx| handler(event, window, cx))
            })
            .child(
                svg()
                    .size(theme::ICON_SIZE)
                    .flex_none()
                    .path(self.icon)
                    .text_color(color),
            )
    }
}

use gpui::{App, Hsla, Rgba, SharedString, Window, div, prelude::*, px, svg};

use crate::ui::theme;

use super::Button;

/// Zed `ConfiguredApiCard`: optional status icon + label on the left, buttons
/// on the right. Callers pass Subtle, Default-size, small-label buttons with a
/// muted leading icon to match Zed.
#[derive(IntoElement)]
pub struct ConfiguredCard {
    label: SharedString,
    icon: Option<(SharedString, Hsla)>,
    actions: Vec<Button>,
}

impl ConfiguredCard {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            actions: Vec::new(),
        }
    }

    /// Leading status icon, e.g. `("check.svg", theme::success())`.
    pub fn icon(mut self, icon: impl Into<SharedString>, color: impl Into<Hsla>) -> Self {
        self.icon = Some((icon.into(), color.into()));
        self
    }

    /// Appends a right-side button; several render left to right.
    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button);
        self
    }
}

impl RenderOnce for ConfiguredCard {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_1()
            .w_full()
            .min_w_0()
            .mt(px(2.))
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(theme::border_variant())
            .bg(Rgba {
                a: 0.5,
                ..theme::hover()
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .text_size(px(14.))
                    .text_color(theme::text())
                    .when_some(self.icon, |left, (icon, color)| {
                        left.child(svg().size(theme::ICON_SIZE).flex_none().path(icon).text_color(color))
                    })
                    .child(self.label),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .children(self.actions),
            )
    }
}

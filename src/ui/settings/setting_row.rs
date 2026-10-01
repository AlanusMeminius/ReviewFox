use gpui::{AnyElement, App, ElementId, SharedString, Window, div, prelude::*, px, relative};

use crate::ui::appearance::UiTextSize;
use crate::ui::theme;

/// One setting (Zed `render_settings_item_layout`): title + description on the
/// left and control on the right, with generous spacing between rows.
#[derive(IntoElement)]
pub struct SettingRow {
    id: ElementId,
    title: SharedString,
    description: Option<AnyElement>,
    title_action: Option<AnyElement>,
    control: Option<AnyElement>,
    narrow: bool,
    last: bool,
}

impl SettingRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            title_action: None,
            control: None,
            narrow: false,
            last: false,
        }
    }

    /// Plain text or an element (e.g. text with a link); rendered 12px muted.
    pub fn description(mut self, description: impl IntoElement) -> Self {
        self.description = Some(description.into_any_element());
        self
    }

    /// Sits right after the title, e.g. a Reset to Default icon button.
    pub fn title_action(mut self, action: impl IntoElement) -> Self {
        self.title_action = Some(action.into_any_element());
        self
    }

    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    /// Caps the left column at 1/2 instead of 2/3 (credential rows).
    pub fn narrow(mut self) -> Self {
        self.narrow = true;
        self
    }

    /// Last row of a section: 40px bottom padding.
    pub fn last(mut self, last: bool) -> Self {
        self.last = last;
        self
    }
}

impl RenderOnce for SettingRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = theme::software_palette();
        div()
            .id(self.id)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_4()
            .w_full()
            .min_w_0()
            .pt(px(16.))
            .map(|row| {
                if self.last {
                    row.pb(px(40.))
                } else {
                    row.pb(px(16.))
                }
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(relative(if self.narrow { 0.5 } else { 2. / 3. }))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .ui_text_size(14., cx)
                            .text_color(palette.text.primary)
                            .child(self.title)
                            .children(self.title_action),
                    )
                    .when_some(self.description, |left, description| {
                        left.child(
                            div()
                                .ui_text_size(12., cx)
                                .text_color(palette.text.secondary)
                                .child(description),
                        )
                    }),
            )
            .children(self.control)
    }
}

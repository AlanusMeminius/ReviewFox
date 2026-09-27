use gpui::{App, Rgba, SharedString, Window, div, prelude::*, px};

use crate::ui::{appearance, theme};

/// Zed `SettingsSectionHeader`: small muted mono label over a faded divider.
/// No horizontal padding; the content pane supplies it.
#[derive(IntoElement)]
pub struct SectionHeader {
    label: SharedString,
}

impl SectionHeader {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl RenderOnce for SectionHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(6.))
            .child(
                div()
                    .font_family(appearance::code_font(cx))
                    .text_size(px(12.))
                    .text_color(theme::muted())
                    .child(self.label),
            )
            .child(div().w_full().h(px(1.)).bg(Rgba {
                a: 0.6,
                ..theme::line()
            }))
    }
}

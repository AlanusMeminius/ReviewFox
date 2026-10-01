use gpui::{App, SharedString, Window, div, prelude::*, px};

use crate::ui::{appearance, theme};

/// Settings section heading: small muted mono label with spacing below.
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
        let palette = theme::software_palette();
        div().flex().flex_col().w_full().pb(px(6.)).child(
            div()
                .font_family(appearance::code_font(cx))
                .text_size(px(12.))
                .text_color(palette.text.secondary)
                .child(self.label),
        )
    }
}

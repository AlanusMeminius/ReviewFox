//! The shared highlighted row used by the font and Code Theme popovers.

use gpui::{ElementId, SharedString, div, prelude::*, px};

use crate::ui::theme;

/// Keep the label's left edge at 9px while the interaction indicator grows
/// from 1px at rest to 2px on hover and 4px while pressed.
pub fn option_row(
    id: impl Into<ElementId>,
    selected: bool,
    label: SharedString,
) -> impl IntoElement {
    let palette = theme::software_palette();
    let colors = palette.settings.option_row;
    div()
        .id(id)
        .flex()
        .items_center()
        .size_full()
        .pl(px(8.))
        .pr(px(8.))
        .rounded(px(4.))
        .border_1()
        .border_color(gpui::Rgba {
            a: 0.,
            ..colors.indicator
        })
        .cursor_pointer()
        .when(selected, |row| {
            row.bg(colors.selected)
                .border_color(colors.indicator)
                .hover(move |row| row.bg(colors.selected_hover).border_l_2().pl(px(7.)))
                .active(move |row| row.bg(colors.selected_pressed).border_l_4().pl(px(5.)))
        })
        .when(!selected, |row| {
            row.hover(move |row| {
                row.bg(colors.hover)
                    .border_color(palette.text.secondary)
                    .border_l_2()
                    .pl(px(7.))
            })
            .active(move |row| {
                row.bg(colors.pressed)
                    .border_color(colors.indicator)
                    .border_l_4()
                    .pl(px(5.))
            })
        })
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(label),
        )
}

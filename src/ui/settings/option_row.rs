//! The shared highlighted row used by the font and Code Theme popovers.

use gpui::{ElementId, SharedString, div, prelude::*, px};

use crate::ui::theme;

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
        .px(px(9.))
        .rounded(px(4.))
        .cursor_pointer()
        .when(selected, |row| {
            row.bg(colors.selected)
                .hover(move |row| row.bg(colors.selected_hover))
                .active(move |row| row.bg(colors.selected_pressed))
        })
        .when(!selected, |row| {
            row.hover(move |row| row.bg(colors.hover))
                .active(move |row| row.bg(colors.pressed))
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

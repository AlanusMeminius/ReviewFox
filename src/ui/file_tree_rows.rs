//! Shared ChangedPath tree row chrome for main Changes + Diff file trees.

use gpui::{
    App, Div, ElementId, FontWeight, InteractiveElement, ParentElement, SharedString, Stateful,
    Styled, div, prelude::FluentBuilder, px, rgb, svg,
};

use crate::domain::{ChangedPath, PathStatus};

use super::appearance::UiTextSize;
use super::theme;

/// Capsule corner on tree rows — tighter than `rounded_lg` on a 22px row.
const ROW_RADIUS: f32 = 4.;
/// Base left padding before depth indent (`depth * 12`).
const ROW_INDENT_BASE: f32 = 8.;
const ROW_HOVER: u32 = 0xf6f8fb;

pub fn dir_row(
    id: impl Into<ElementId>,
    depth: u32,
    name: impl Into<SharedString>,
    collapsed: bool,
    cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .mx_1()
        .h(px(22.))
        .pl(px(ROW_INDENT_BASE + depth as f32 * 12.))
        .pr_1()
        .rounded(px(ROW_RADIUS))
        .flex()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .hover(|d| d.bg(rgb(ROW_HOVER)))
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .size(theme::ICON_SIZE)
                        .path(if collapsed {
                            "folder.svg"
                        } else {
                            "folder_open.svg"
                        })
                        .text_color(theme::muted()),
                ),
        )
        .child(
            div()
                .h(px(16.))
                .flex()
                .items_center()
                .ui_text_size(12., cx)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(name.into()),
        )
}

pub fn file_row(
    id: impl Into<ElementId>,
    depth: u32,
    path: &ChangedPath,
    selected: bool,
    mono: SharedString,
    cx: &App,
) -> Stateful<Div> {
    let name = path.file_name().to_string();
    let status = path.status;
    let add = path.additions;
    let del = path.deletions;
    // Solid accent selected fill kills status chroma — match sidebar icons: white on blue.
    let on_selected = theme::on_sidebar_selected();
    let name_color = if selected {
        on_selected
    } else {
        theme::text()
    };
    let status_color = if selected {
        on_selected
    } else {
        match status {
            PathStatus::Add => rgb(0x1a7f4b),
            PathStatus::Delete => rgb(0xb42318),
            PathStatus::Modify => rgb(0x9a6700),
        }
    };
    let add_color = if selected {
        on_selected
    } else {
        rgb(0x1a7f4b)
    };
    let del_color = if selected {
        on_selected
    } else {
        rgb(0xb42318)
    };

    div()
        .id(id)
        .mx_1()
        .h(px(22.))
        .pl(px(ROW_INDENT_BASE + depth as f32 * 12.))
        .pr_1()
        .rounded(px(ROW_RADIUS))
        .flex()
        .items_center()
        .gap_1()
        .when(selected, |d| d.bg(theme::sidebar_selected()))
        .hover(move |d| {
            if selected {
                d.bg(theme::sidebar_selected())
            } else {
                d.bg(rgb(ROW_HOVER))
            }
        })
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .font_family(mono.clone())
                .text_xs()
                .text_color(status_color)
                .child(status.letter()),
        )
        .child(
            div()
                .h(px(16.))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .flex()
                .items_center()
                .ui_text_size(12., cx)
                .text_color(name_color)
                .child(name),
        )
        .child(
            div()
                .h(px(16.))
                .flex()
                .items_center()
                .font_family(mono)
                .text_xs()
                .gap_1()
                .child(div().text_color(add_color).child(format!("+{add}")))
                .child(div().text_color(del_color).child(format!("−{del}"))),
        )
}

//! Shared ChangedPath tree row chrome for main Changes + Diff file trees.

use gpui::{
    App, Div, ElementId, FontWeight, InteractiveElement, ObjectFit, ParentElement, Pixels, Rgba,
    SharedString, Stateful, StatefulInteractiveElement, Styled, StyledImage, TextRun, black, div,
    font, img, prelude::FluentBuilder, px, rgb, svg,
};

use crate::domain::{ChangedPath, PathStatus};

use super::appearance::{self, UiTextSize};
use super::file_icons::file_icon_for_path;
use super::theme;

/// Capsule corner on tree rows — tighter than `rounded_lg` on a 22px row.
const ROW_RADIUS: f32 = 4.;
/// Base left padding before depth indent (`depth * 12`).
const ROW_INDENT_BASE: f32 = 8.;
const ROW_HOVER: u32 = 0xf6f8fb;
/// Gap between [`RowSurface::Desk`] rows so neighbouring capsules never touch.
const DESK_ROW_GAP: f32 = 2.;
/// Scroll `px_1` (4×2) + row `mx_1` (4×2).
const ROW_H_INSET: f32 = 16.;
const ROW_PAD_RIGHT: f32 = 4.;
const ICON_SLOT: f32 = 16.;
const STATUS_SLOT: f32 = 16.;
const ROW_GAP: f32 = 4.;

/// What the tree sits on, which decides its hover / selected fills.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowSurface {
    /// Inside a white island (main Changes).
    Island,
    /// Clear on the frosted desk (Diff tree): the workspace sidebar's translucent capsules.
    Desk,
}

impl RowSurface {
    fn hover(self) -> Rgba {
        match self {
            RowSurface::Island => rgb(ROW_HOVER),
            RowSurface::Desk => theme::sidebar_row_hover(),
        }
    }
}

fn row_base(id: ElementId, depth: u32, surface: RowSurface, pane_width: f32) -> Stateful<Div> {
    div()
        .id(id)
        .mx_1()
        .when(surface == RowSurface::Desk, |d| d.mb(px(DESK_ROW_GAP)))
        .w(px((pane_width - ROW_H_INSET).max(0.)))
        .min_w(px(0.))
        .h(px(22.))
        .pl(px(ROW_INDENT_BASE + depth as f32 * 12.))
        .pr_1()
        .rounded(px(ROW_RADIUS))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap_1()
}

/// Max width for the basename before the trailing chrome (status / ±).
///
/// GPUI's `text_ellipsis` + `whitespace_nowrap` caches the first (often
/// indefinite) text measure and never truncates afterwards — so we ellipsize
/// the string ourselves with [`LineWrapper::truncate_line`].
fn name_max_width(pane_width: f32, depth: u32, after_name: f32) -> Pixels {
    let row_w = (pane_width - ROW_H_INSET).max(0.);
    let pl = ROW_INDENT_BASE + depth as f32 * 12.;
    // icon + gap before name; `after_name` covers gaps + status + ± for files.
    let used = pl + ROW_PAD_RIGHT + ICON_SLOT + ROW_GAP + after_name;
    px((row_w - used).max(0.))
}

fn digit_count(mut n: u32) -> usize {
    if n == 0 {
        return 1;
    }
    let mut d = 0;
    while n > 0 {
        n /= 10;
        d += 1;
    }
    d
}

/// Space after the name: gaps + status letter + `+N −M` (mono advance estimate).
fn file_after_name(add: u32, del: u32) -> f32 {
    // Plex Mono / Menlo at 12px — slightly wide so the `…` is never clipped.
    const MONO_ADV: f32 = 8.;
    let stats_chars = 2 + digit_count(add) + digit_count(del); // "+"/ "−" + digits
    ROW_GAP + STATUS_SLOT + ROW_GAP + stats_chars as f32 * MONO_ADV
}

fn ellipsize(text: &str, max_width: Pixels, weight: FontWeight, cx: &App) -> SharedString {
    let shared: SharedString = text.to_owned().into();
    if max_width <= px(0.) {
        return SharedString::new_static("…");
    }
    let mut face = font(appearance::ui_font(cx));
    face.weight = weight;
    let mut runs = vec![TextRun {
        len: shared.len(),
        font: face.clone(),
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    cx.text_system()
        .line_wrapper(face, appearance::ui_text(cx, 12.))
        .truncate_line(shared, max_width, "…", &mut runs)
}

fn name_cell(label: SharedString, color: Rgba, weight: Option<FontWeight>, cx: &App) -> Div {
    div()
        .w(px(0.))
        .flex_1()
        .min_w(px(0.))
        .h(px(16.))
        .overflow_hidden()
        .ui_label_size(12., cx)
        .when_some(weight, |d, w| d.font_weight(w))
        .text_color(color)
        .child(label)
}

pub fn dir_row(
    id: impl Into<ElementId>,
    depth: u32,
    name: impl Into<SharedString>,
    collapsed: bool,
    surface: RowSurface,
    pane_width: f32,
    cx: &App,
) -> Stateful<Div> {
    let name = name.into();
    let label = ellipsize(
        name.as_ref(),
        name_max_width(pane_width, depth, 0.),
        FontWeight::MEDIUM,
        cx,
    );
    row_base(id.into(), depth, surface, pane_width)
        .cursor_pointer()
        .hover(move |d| d.bg(surface.hover()))
        .when(surface == RowSurface::Desk, |d| {
            d.active(|d| d.bg(theme::sidebar_row_selected()))
        })
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .size(theme::ICON_SIZE_SM)
                        .path(if collapsed {
                            "folder.svg"
                        } else {
                            "folder_open.svg"
                        })
                        .text_color(theme::muted()),
                ),
        )
        .child(name_cell(
            label,
            theme::muted(),
            Some(FontWeight::MEDIUM),
            cx,
        ))
}

pub fn file_row(
    id: impl Into<ElementId>,
    depth: u32,
    path: &ChangedPath,
    selected: bool,
    surface: RowSurface,
    mono: SharedString,
    pane_width: f32,
    cx: &App,
) -> Stateful<Div> {
    let name = path.file_name().to_string();
    let status = path.status;
    let add = path.additions;
    let del = path.deletions;
    let after = file_after_name(add, del);
    let label = ellipsize(
        &name,
        name_max_width(pane_width, depth, after),
        FontWeight::NORMAL,
        cx,
    );
    // Solid accent selected fill kills status chroma — match sidebar icons: white on blue.
    // The desk's translucent capsule keeps every colour as-is.
    let on_accent = selected && surface == RowSurface::Island;
    let on_selected = theme::on_sidebar_selected();
    let name_color = if on_accent {
        on_selected
    } else {
        theme::text()
    };
    let status_color = if on_accent {
        on_selected
    } else {
        match status {
            PathStatus::Add => rgb(0x1a7f4b),
            PathStatus::Delete => rgb(0xb42318),
            PathStatus::Modify => rgb(0x9a6700),
        }
    };
    let add_color = if on_accent {
        on_selected
    } else {
        rgb(0x1a7f4b)
    };
    let del_color = if on_accent {
        on_selected
    } else {
        rgb(0xb42318)
    };
    let selected_fill = match surface {
        RowSurface::Island => theme::sidebar_selected(),
        RowSurface::Desk => theme::sidebar_row_selected(),
    };

    row_base(id.into(), depth, surface, pane_width)
        .when(selected, |d| d.bg(selected_fill))
        .when(!selected, |d| {
            d.hover(move |d| d.bg(surface.hover()))
                .when(surface == RowSurface::Desk, |d| {
                    d.active(|d| d.bg(theme::sidebar_row_selected()))
                })
        })
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                // Material icons are multi-colour: `img` keeps native fills, where
                // `svg().text_color()` would flatten them to one mask colour.
                .child(
                    img(file_icon_for_path(&path.path))
                        .size(theme::ICON_SIZE_SM)
                        .object_fit(ObjectFit::Contain),
                ),
        )
        .child(name_cell(label, name_color, None, cx))
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .font_family(mono.clone())
                .code_label_size(px(12.), mono.clone(), cx)
                .text_color(status_color)
                .child(status.letter()),
        )
        .child(
            div()
                .h(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .font_family(mono.clone())
                .code_label_size(px(12.), mono, cx)
                .gap_1()
                .child(div().text_color(add_color).child(format!("+{add}")))
                .child(div().text_color(del_color).child(format!("−{del}"))),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_max_width_shrinks_with_depth_and_trailing() {
        let shallow = f32::from(name_max_width(280., 0, 0.));
        let deep = f32::from(name_max_width(280., 3, 0.));
        let with_stats = f32::from(name_max_width(280., 0, 80.));
        assert!(shallow > deep);
        assert!(shallow > with_stats);
        assert!(with_stats > 0.);
    }

    #[test]
    fn name_max_width_zero_when_pane_too_narrow() {
        assert_eq!(f32::from(name_max_width(20., 0, 0.)), 0.);
    }
}

//! Label + colored pill metadata rows, ported from BeadsViewer's issue detail.

use gpui::{
    AnyElement, App, ClipboardItem, Div, FontWeight, IntoElement, SharedString, Stateful, div,
    prelude::*, px, rgb,
};

use super::appearance::UiTextSize;
use super::theme;
use super::tooltip::Tooltip;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: &'static str,
    pub value: String,
}

pub fn row(items: Vec<Item>, cx: &App) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .children(items.into_iter().map(|item| render_item(item, cx)))
}

/// Compact semantic badge used for statuses and other short values.
/// Keep the palette intentionally soft so the text remains the primary signal.
pub fn capsule(value: impl Into<String>, cx: &App) -> Div {
    let value = value.into();
    let (background, foreground) = capsule_colors(&value);
    div()
        .flex_none()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgb(background))
        .ui_text_size(12., cx)
        .line_height(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(foreground))
        .child(value)
}

/// The id doubles as a copy affordance, so it is the one value rendered as an
/// interactive capsule instead of a plain one.
fn copyable_capsule(value: impl Into<String>, cx: &App) -> Stateful<Div> {
    let value = value.into();
    let copied = value.clone();
    capsule(value.clone(), cx)
        .id(SharedString::from(format!("copy-metadata-{value}")))
        .debug_selector(move || format!("metadata-copy-{value}"))
        .cursor_pointer()
        .hover(|capsule| capsule.bg(theme::line()))
        .active(|capsule| capsule.bg(theme::element_active()))
        .tooltip(Tooltip::text("Click to copy", None))
        .on_click(move |_, _, cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
        })
}

fn is_copyable(label: &str) -> bool {
    label == "ID"
}

fn render_item(item: Item, cx: &App) -> Div {
    let value: AnyElement = if is_copyable(item.label) {
        copyable_capsule(item.value, cx).into_any_element()
    } else {
        capsule(item.value, cx).into_any_element()
    };

    div()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap_1()
        .ui_text_size(12., cx)
        .font_weight(FontWeight::NORMAL)
        .child(
            div()
                .flex_none()
                .text_color(theme::faint())
                .child(item.label),
        )
        .child(value)
}

fn capsule_colors(value: &str) -> (u32, u32) {
    match value.to_ascii_lowercase().as_str() {
        "open" | "opened" | "ready" | "running" | "pending" => (0xdbeafe, 0x1d4ed8),
        "closed" | "merged" | "done" | "completed" | "success" => (0xdcfce7, 0x166534),
        "blocked" | "failed" | "canceled" => (0xffe4e6, 0x9f1239),
        _ => (0xf1f3f6, 0x4b5563),
    }
}

#[cfg(test)]
mod tests {
    use super::{capsule_colors, is_copyable};

    #[test]
    fn only_the_id_is_offered_as_a_copy_target() {
        assert!(is_copyable("ID"));
        assert!(!is_copyable("Status"));
        assert!(!is_copyable("Pipeline"));
    }

    #[test]
    fn merge_request_states_and_pipelines_read_as_green_or_red() {
        assert_eq!(capsule_colors("merged"), capsule_colors("closed"));
        assert_eq!(capsule_colors("success"), capsule_colors("closed"));
        assert_eq!(capsule_colors("running"), capsule_colors("open"));
        assert_eq!(capsule_colors("pending"), capsule_colors("open"));
        assert_eq!(capsule_colors("canceled"), capsule_colors("failed"));
        assert_eq!(capsule_colors("can_be_merged"), (0xf1f3f6, 0x4b5563));
    }
}

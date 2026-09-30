//! Label + colored pill metadata rows, ported from BeadsViewer's issue detail.

use gpui::{
    AnyElement, App, ClipboardItem, Div, FontWeight, IntoElement, SharedString, Stateful, div,
    prelude::*, px,
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
    let palette = theme::software_palette();
    let colors = capsule_colors(&value, palette.feedback);
    div()
        .flex_none()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(colors.background)
        .border_1()
        .border_color(colors.border)
        .ui_text_size(12., cx)
        .line_height(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors.foreground)
        .child(value)
}

/// The id doubles as a copy affordance, so it is the one value rendered as an
/// interactive capsule instead of a plain one.
fn copyable_capsule(value: impl Into<String>, cx: &App) -> Stateful<Div> {
    let value = value.into();
    let copied = value.clone();
    let palette = theme::software_palette();
    capsule(value.clone(), cx)
        .id(SharedString::from(format!("copy-metadata-{value}")))
        .debug_selector(move || format!("metadata-copy-{value}"))
        .cursor_pointer()
        // Grow the left edge 1→2→4px, compensating padding so the ID stays put.
        .hover(move |capsule| {
            capsule
                .bg(palette.metadata.copy_hover)
                .border_l_2()
                .pl(px(4.))
        })
        .active(move |capsule| {
            capsule
                .bg(palette.metadata.copy_pressed)
                .border_l_4()
                .pl(px(2.))
        })
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
                .text_color(theme::software_palette().metadata.label)
                .child(item.label),
        )
        .child(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Info,
    Success,
    Error,
    Warning,
    Neutral,
}

fn tone_for(value: &str) -> Tone {
    match value.to_ascii_lowercase().as_str() {
        "open" | "opened" | "ready" | "running" | "pending" => Tone::Info,
        "merged" | "done" | "completed" | "success" => Tone::Success,
        "failed" | "canceled" => Tone::Error,
        "blocked" | "warning" | "conflict" | "conflicts" => Tone::Warning,
        _ => Tone::Neutral,
    }
}

fn capsule_colors(value: &str, colors: theme::FeedbackColors) -> theme::StatusColors {
    match tone_for(value) {
        Tone::Info => colors.info,
        Tone::Success => colors.success,
        Tone::Error => colors.error,
        Tone::Warning => colors.warning,
        Tone::Neutral => colors.neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::{Tone, is_copyable, tone_for};

    #[test]
    fn only_the_id_is_offered_as_a_copy_target() {
        assert!(is_copyable("ID"));
        assert!(!is_copyable("Status"));
        assert!(!is_copyable("Pipeline"));
    }

    #[test]
    fn merge_request_states_and_pipelines_keep_contextual_meaning() {
        assert_eq!(tone_for("merged"), Tone::Success);
        assert_eq!(tone_for("success"), Tone::Success);
        assert_eq!(tone_for("closed"), Tone::Neutral);
        assert_eq!(tone_for("running"), Tone::Info);
        assert_eq!(tone_for("pending"), Tone::Info);
        assert_eq!(tone_for("canceled"), Tone::Error);
        assert_eq!(tone_for("blocked"), Tone::Warning);
        assert_eq!(tone_for("can_be_merged"), Tone::Neutral);
    }
}

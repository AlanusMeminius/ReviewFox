//! Shared MR content; callers own discovery/Entry state and scrolling.
use gpui::{AnyElement, App, Div, FontWeight, div, prelude::*, px};

use super::{appearance::UiTextSize, metadata, selectable_markdown, theme};
use crate::gitlab::MergeRequestCheckState;

pub struct Content<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub metadata: Vec<metadata::Item>,
    pub notice: Option<AnyElement>,
}

pub fn render(id: String, content: Content<'_>, cx: &mut App) -> Div {
    div()
        .min_w(px(0.))
        .flex_none()
        .flex()
        .flex_col()
        .gap_1p5()
        .ui_text_size(12., cx)
        .text_color(theme::software_palette().metadata.label)
        .child(
            div()
                .ui_text_size(14., cx)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::software_palette().metadata.text)
                .child(content.title.to_owned()),
        )
        .child(metadata::row(content.metadata, cx))
        .children(content.notice)
        .when_some(
            content.description.filter(|s| !s.trim().is_empty()),
            |d, description| d.child(selectable_markdown::view(id, description.to_owned(), cx)),
        )
}

/// Metadata shared by both surfaces. Identity belongs to each caller's context:
/// the inbox already shows it in its list; an opened MR Entry still needs it.
pub fn context_items(
    source: &str,
    target: &str,
    merge_status: Option<&str>,
    checks: Option<&MergeRequestCheckState>,
) -> Vec<metadata::Item> {
    let mut items = vec![metadata::Item {
        label: "Branches",
        value: format!("{source} → {target}"),
    }];
    if let Some(status) = merge_status.filter(|s| !s.is_empty()) {
        items.push(metadata::Item {
            label: "Merge",
            value: status.to_owned(),
        });
    }
    if let Some(checks) = checks {
        if let Some(pipeline) = &checks.pipeline_status {
            items.push(metadata::Item {
                label: "Pipeline",
                value: pipeline.clone(),
            });
        }
        if let Some(label) = &checks.approvals_label {
            items.push(metadata::Item {
                label: "Approvals",
                value: label.clone(),
            });
        } else if checks.approved == Some(true) {
            items.push(metadata::Item {
                label: "Approvals",
                value: "approved".into(),
            });
        }
    }
    items
}

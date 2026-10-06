//! Shared MR content; callers own discovery/Entry state and scrolling.
use gpui::{AnyElement, App, Div, FontWeight, div, prelude::*, px};

use super::{appearance::UiTextSize, metadata, selectable_markdown, theme};
use crate::gitlab::MergeRequestCheckState;

pub struct Content<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub metadata: Vec<metadata::Item>,
    pub checks: Vec<metadata::Item>,
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
        .child(match content.notice {
            Some(notice) => div().min_h(px(24.)).flex().items_center().child(notice),
            None => div().h(px(24.)).flex_none().child(
                div()
                    .id(gpui::SharedString::from(format!("{id}-checks")))
                    .size_full()
                    .overflow_x_scroll()
                    .child(div().h_full().flex().items_center().gap_3().children(
                        content.checks.into_iter().map(|item| {
                            metadata::row(vec![item], cx)
                                .flex_none()
                                .whitespace_nowrap()
                        }),
                    )),
            ),
        })
        .when_some(
            content.description.filter(|s| !s.trim().is_empty()),
            |d, description| d.child(selectable_markdown::view(id, description.to_owned(), cx)),
        )
}

/// Metadata shared by both surfaces. Identity belongs to each caller's context:
/// the inbox already shows it in its list; an opened MR Entry still needs it.
pub fn context_items(source: &str, target: &str) -> Vec<metadata::Item> {
    vec![metadata::Item {
        label: "Branches",
        value: format!("{source} → {target}"),
    }]
}

/// Checks occupy a separate row so async completion never changes branch layout.
pub fn check_items(
    merge_status: Option<&str>,
    checks: Option<&MergeRequestCheckState>,
) -> Vec<metadata::Item> {
    let mut items = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, Window};

    struct Harness {
        loading: bool,
        width: f32,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let notice = self.loading.then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().size(px(16.)))
                    .child("Loading checks…")
                    .into_any_element()
            });
            let checks = if self.loading {
                Vec::new()
            } else {
                vec![
                    metadata::Item {
                        label: "Merge",
                        value: "can_be_merged".into(),
                    },
                    metadata::Item {
                        label: "Pipeline",
                        value: "success".into(),
                    },
                    metadata::Item {
                        label: "Approvals",
                        value: "2/2".into(),
                    },
                ]
            };
            div()
                .w(px(self.width))
                .flex()
                .flex_col()
                .child(render(
                    "fixture".into(),
                    Content {
                        title: "MR title",
                        description: None,
                        metadata: context_items("feature", "main"),
                        checks,
                        notice,
                    },
                    cx,
                ))
                .child(
                    div()
                        .id("body-position")
                        .debug_selector(|| "body-position".into())
                        .h(px(20.)),
                )
        }
    }

    #[gpui::test]
    fn checks_completion_keeps_body_position_at_narrow_and_wide_widths(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.set_global(crate::ui::appearance::resolve(&Default::default(), &[])));
        let (view, cx) = cx.add_window_view(|_, _| Harness {
            loading: true,
            width: 600.,
        });
        for width in [600., 216.] {
            view.update(cx, |view, cx| {
                view.width = width;
                view.loading = true;
                cx.notify();
            });
            cx.run_until_parked();
            let before = cx.debug_bounds("body-position").unwrap();
            view.update(cx, |view, cx| {
                view.loading = false;
                cx.notify();
            });
            cx.run_until_parked();
            let after = cx.debug_bounds("body-position").unwrap();
            assert_eq!(
                before.origin.y, after.origin.y,
                "checks must not shift the body at width {width}"
            );
        }
    }
}

use gpui::{App, ClickEvent, ElementId, Rgba, SharedString, Window, div, prelude::*, px, svg};

use crate::ui::{appearance, theme};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Settings nav sidebar shell: fixed 200px, 10px padding, sitting clear on the
/// frosted desk; the content island beside it is the only boundary.
/// Focus, key context and actions live on the caller's wrapper.
#[derive(IntoElement)]
pub struct SettingsNav {
    items: Vec<NavItem>,
}

impl SettingsNav {
    pub const WIDTH: f32 = 200.;

    pub fn new(items: Vec<NavItem>) -> Self {
        Self { items }
    }
}

impl RenderOnce for SettingsNav {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(Self::WIDTH))
            .h_full()
            .p(px(10.))
            .bg(theme::sidebar())
            .overflow_hidden()
            .children(self.items)
    }
}

/// Zed `TreeViewItem`: a 28px row. Pages carry a chevron; sections sit behind
/// a 22px indent column holding a 1px guide line.
#[derive(IntoElement)]
pub struct NavItem {
    id: ElementId,
    label: SharedString,
    /// `Some(expanded)` for a page, `None` for a section.
    page: Option<bool>,
    selected: bool,
    focused: bool,
    on_click: Option<ClickHandler>,
    on_toggle: Option<ClickHandler>,
}

impl NavItem {
    pub fn page(id: impl Into<ElementId>, label: impl Into<SharedString>, expanded: bool) -> Self {
        Self::build(id.into(), label.into(), Some(expanded))
    }

    pub fn section(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::build(id.into(), label.into(), None)
    }

    fn build(id: ElementId, label: SharedString, page: Option<bool>) -> Self {
        Self {
            id,
            label,
            page,
            selected: false,
            focused: false,
            on_click: None,
            on_toggle: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Keyboard focus is on this row: `border_focused` border.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Chevron click on a page row (does not also fire `on_click`).
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Selection is the white capsule alone (colour-only boundaries); the
        // border only ever shows keyboard focus.
        let border = if self.focused {
            theme::border_focused()
        } else {
            Rgba {
                a: 0.,
                ..theme::line()
            }
        };
        let label_color = if self.selected {
            theme::text()
        } else {
            theme::muted()
        };
        let label = div()
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .text_color(label_color)
            .child(self.label);

        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h(px(28.))
            .gap_2()
            .pl(px(2.))
            .pr_1()
            // Same capsule as the main window's sidebar rows.
            .rounded_lg()
            .border_1()
            .border_color(border)
            .when(self.selected, |row| row.bg(theme::capsule()))
            .when(!self.selected, |row| {
                row.hover(|row| row.bg(theme::settings_nav_hover()))
            })
            .cursor_pointer()
            .font_family(appearance::ui_font(cx))
            .text_size(px(14.))
            .when_some(self.on_click, |row, handler| {
                row.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .map(|row| match self.page {
                Some(expanded) => row
                    .child(
                        div()
                            .id("toggle")
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(16.))
                            .rounded(px(4.))
                            .hover(|chevron| chevron.bg(theme::element_active()))
                            .when_some(self.on_toggle, |chevron, handler| {
                                chevron.on_click(move |event, window, cx| {
                                    cx.stop_propagation();
                                    handler(event, window, cx);
                                })
                            })
                            .child(
                                svg()
                                    .size(px(14.))
                                    .path(if expanded {
                                        "chevron_down.svg"
                                    } else {
                                        "chevron_right.svg"
                                    })
                                    .text_color(theme::muted()),
                            ),
                    )
                    .child(label),
                None => row
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .justify_center()
                            .w(px(22.))
                            .h_full()
                            .child(div().w(px(1.)).h_full().bg(Rgba {
                                a: 0.5,
                                ..theme::line()
                            })),
                    )
                    .child(label),
            })
    }
}

use gpui::{
    App, ClickEvent, ElementId, FocusHandle, Hsla, Pixels, SharedString, Window, div, prelude::*,
    px, svg, transparent_black,
};

use crate::ui::appearance::{self, UiTextSize};
use crate::ui::theme;
use crate::ui::tooltip::Tooltip;

/// Zed `ButtonStyle`, cut down to what Settings uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonStyle {
    #[default]
    Subtle,
    Outlined,
    #[allow(dead_code)] // Part of the component set; no Settings page uses it yet.
    Tinted(TintColor),
}

#[allow(dead_code)] // Only reachable through ButtonStyle::Tinted, unused so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TintColor {
    Success,
    Error,
}

/// Zed `ButtonSize::Medium` (28px, 8px padding) / `Default` (22px, 4px).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Medium,
    #[default]
    Default,
}

impl ButtonSize {
    fn height(self) -> Pixels {
        match self {
            ButtonSize::Medium => px(28.),
            ButtonSize::Default => px(22.),
        }
    }

    fn padding_x(self) -> Pixels {
        match self {
            ButtonSize::Medium => px(8.),
            ButtonSize::Default => px(4.),
        }
    }
}

struct Colors {
    background: Hsla,
    border: Hsla,
    hover: Hsla,
    active: Hsla,
}

impl ButtonStyle {
    fn colors(self, palette: theme::ControlColors) -> Colors {
        let (background, border, hover, active) = match self {
            ButtonStyle::Subtle => (
                transparent_black(),
                transparent_black(),
                palette.hover.into(),
                palette.pressed.into(),
            ),
            ButtonStyle::Outlined => (
                transparent_black(),
                palette.outline.into(),
                palette.hover.into(),
                palette.pressed.into(),
            ),
            // Zed darkens the tint on hover; the tint's own border color is that step.
            ButtonStyle::Tinted(TintColor::Success) => (
                theme::success_background().into(),
                theme::success_border().into(),
                theme::success_border().into(),
                theme::success_border().into(),
            ),
            ButtonStyle::Tinted(TintColor::Error) => (
                theme::error_background().into(),
                theme::error_border().into(),
                theme::error_border().into(),
                theme::error_border().into(),
            ),
        };
        Colors {
            background,
            border,
            hover,
            active,
        }
    }
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Settings button: label with an optional leading or trailing icon, or icon only.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<SharedString>,
    icon_color: Option<Hsla>,
    icon_end: bool,
    hint: Option<SharedString>,
    style: ButtonStyle,
    size: ButtonSize,
    small_label: bool,
    tooltip: Option<SharedString>,
    tab_index: Option<isize>,
    focus_handle: Option<FocusHandle>,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::build(id.into(), Some(label.into()), None)
    }

    /// Square icon button (Zed `IconButton`); give it a [`Self::tooltip`].
    pub fn icon_only(id: impl Into<ElementId>, icon: impl Into<SharedString>) -> Self {
        Self::build(id.into(), None, Some(icon.into()))
    }

    fn build(id: ElementId, label: Option<SharedString>, icon: Option<SharedString>) -> Self {
        Self {
            id,
            label,
            icon,
            icon_color: None,
            icon_end: false,
            hint: None,
            style: ButtonStyle::default(),
            size: ButtonSize::default(),
            small_label: false,
            tooltip: None,
            tab_index: None,
            focus_handle: None,
            disabled: false,
            on_click: None,
        }
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Leading icon, an asset path such as `"undo.svg"`.
    pub fn start_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Trailing icon (Zed `IconPosition::End`), e.g. a picker's chevron.
    pub fn end_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self.icon_end = true;
        self
    }

    /// Muted 12px text after the label, e.g. `not installed`.
    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Icon tint; muted by default.
    #[allow(dead_code)] // Part of the component set; no Settings page uses it yet.
    pub fn icon_color(mut self, color: impl Into<Hsla>) -> Self {
        self.icon_color = Some(color.into());
        self
    }

    /// 12px label instead of 14px (Zed `LabelSize::Small`).
    pub fn small_label(mut self) -> Self {
        self.small_label = true;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Makes the button a Tab stop; keyboard focus shows a `border_focused` border.
    /// While focused, Enter / Space (no modifiers) run [`Self::on_click`]: GPUI's
    /// div turns their key-up into a `ClickEvent::Keyboard` for focused elements.
    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = Some(index);
        self
    }

    /// Like [`Self::tab_index`], with a handle the owner keeps, so it can
    /// focus the button (e.g. when a popover it opened closes). The handle
    /// carries the tab index.
    pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }

    /// Faint, no hover, and [`Self::on_click`] never runs.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = theme::software_palette();
        let colors = self.style.colors(palette.control);
        let height = self.size.height();
        let icon_only = self.label.is_none();
        let (text_color, icon_color): (Hsla, Hsla) = if self.disabled {
            (
                palette.settings.text_disabled.into(),
                palette.settings.text_disabled.into(),
            )
        } else {
            (
                palette.text.primary.into(),
                self.icon_color
                    .unwrap_or_else(|| palette.text.secondary.into()),
            )
        };
        let on_click = self.on_click.filter(|_| !self.disabled);
        let focusable = self.tab_index.is_some() || self.focus_handle.is_some();
        let small_label = self.small_label;
        let icon = self.icon.map(|icon| {
            svg()
                .size(px(14.))
                .flex_none()
                .path(icon)
                .text_color(icon_color)
        });
        let (start_icon, end_icon) = if self.icon_end {
            (None, icon)
        } else {
            (icon, None)
        };

        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(height)
            .map(|button| {
                if icon_only {
                    button.w(height).justify_center()
                } else {
                    button.px(self.size.padding_x())
                }
            })
            .rounded(px(4.))
            // Always 1px so focus / outline never shifts the layout.
            .border_1()
            .border_color(colors.border)
            .bg(colors.background)
            .font_family(appearance::ui_font(cx))
            .ui_text_size(if self.small_label { 12. } else { 14. }, cx)
            .text_color(text_color)
            .when(!self.disabled, |button| {
                button
                    .hover(|button| button.bg(colors.hover))
                    .active(|button| button.bg(colors.active))
            })
            .when(focusable, |button| {
                button.focus(|button| button.border_color(palette.settings.focus_border))
            })
            .when_some(self.tab_index, |button, index| button.tab_index(index))
            .when_some(self.focus_handle, |button, handle| {
                button.track_focus(&handle)
            })
            .when_some(self.tooltip, |button, text| {
                button.tooltip(Tooltip::text(text, None))
            })
            .when_some(on_click, |button, handler| {
                button
                    .cursor_pointer()
                    .on_click(move |event, window, cx| handler(event, window, cx))
            })
            .children(start_icon)
            .when_some(self.label, |button, label| {
                button.child(
                    div()
                        .ui_label_size(if small_label { 12. } else { 14. }, cx)
                        .child(label),
                )
            })
            .when_some(self.hint, |button, hint| {
                button.child(
                    div()
                        .ui_text_size(12., cx)
                        .text_color(palette.control.hint)
                        .child(hint),
                )
            })
            .children(end_icon)
    }
}

//! The one tooltip every control shares: a dark pill with a label and, when the
//! control has one, its keyboard shortcut in a dimmer tone.

use gpui::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::FluentBuilder, rgb,
};

use super::appearance::{self, UiTextSize};
use super::theme;

pub struct Tooltip {
    label: SharedString,
    shortcut: Option<SharedString>,
}

impl Tooltip {
    /// A `.tooltip(..)` builder for `label`, with an optional shortcut.
    pub fn text(
        label: impl Into<SharedString>,
        shortcut: Option<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let label = label.into();
        move |_, cx| {
            let label = label.clone();
            let shortcut = shortcut.clone();
            cx.new(|_| Tooltip { label, shortcut }).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x273142))
            .font_family(appearance::ui_font(cx))
            .ui_text_size(12., cx)
            .text_color(theme::white())
            .flex()
            .items_center()
            .gap_2()
            .child(self.label.clone())
            .when_some(self.shortcut.clone(), |pill, shortcut| {
                pill.child(div().text_color(theme::faint()).child(shortcut))
            })
    }
}

/// `key` behind the platform's command modifier: `⌘,` on macOS, `Ctrl+,` elsewhere.
pub fn cmd(key: &str) -> SharedString {
    if cfg!(target_os = "macos") {
        format!("⌘{key}").into()
    } else {
        format!("Ctrl+{key}").into()
    }
}

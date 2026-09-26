#[cfg(target_os = "macos")]
mod app_icon;
mod app_view;
mod diff_window;
mod file_tree;
mod scrollbar;
mod splitter;
mod theme;

use gpui::{
    App, AppContext, Application, AssetSource, Bounds, KeyBinding, Menu, MenuItem, SharedString,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions,
    actions, point, px, size,
};
use std::borrow::Cow;

use app_view::AppView;
use crate::git::BranchBrowser;
use crate::workspace_store;

actions!(app, [Quit]);

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "sidebar_title.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/sidebar_title.svg") as &'static [u8],
            )),
            "diff_title.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/diff_title.svg") as &'static [u8],
            )),
            "branch.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/branch.svg") as &'static [u8],
            )),
            "folder.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/folder.svg") as &'static [u8],
            )),
            "folder_open.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/folder_open.svg") as &'static [u8],
            )),
            "export.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/export.svg") as &'static [u8],
            )),
            _ => None,
        })
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

pub fn run() {
    let boot = restore_last();

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        app_icon::set_app_icon();

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "ReviewFox".into(),
            items: vec![MenuItem::action("Quit", Quit)],
        }]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);

        let boot = boot.clone();
        let _main = cx
            .open_window(
                WindowOptions {
                    focus: true,
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("ReviewFox".into()),
                        appears_transparent: true,
                        traffic_light_position: traffic_light_position(),
                        ..Default::default()
                    }),
                    window_decorations: Some(WindowDecorations::Client),
                    window_background: window_background_appearance(),
                    ..Default::default()
                },
                move |_, cx| cx.new(|cx| AppView::new(boot, cx)),
            )
            .expect("open main window");

        // Diff opens via Open Diff on the main Changes chrome.
        cx.activate(true);
    });
}

/// Restore via `last` → BranchBrowser::open_workspace. Failure already drops the entry.
fn restore_last() -> Option<BranchBrowser> {
    let store = workspace_store::load();
    let Some(entry) = workspace_store::last_entry(&store).cloned() else {
        if store.last.is_some() {
            let mut store = store;
            store.last = None;
            workspace_store::save(&store);
        }
        return None;
    };
    BranchBrowser::open_workspace(&entry).ok()
}

#[cfg(target_os = "macos")]
fn traffic_light_position() -> Option<gpui::Point<gpui::Pixels>> {
    Some(point(
        px(theme::TRAFFIC_LIGHT_LEFT_INSET),
        px(theme::TRAFFIC_LIGHT_TOP_INSET),
    ))
}

#[cfg(not(target_os = "macos"))]
fn traffic_light_position() -> Option<gpui::Point<gpui::Pixels>> {
    None
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn window_background_appearance() -> WindowBackgroundAppearance {
    WindowBackgroundAppearance::Blurred
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn window_background_appearance() -> WindowBackgroundAppearance {
    WindowBackgroundAppearance::Opaque
}

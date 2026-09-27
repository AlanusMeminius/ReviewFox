#[cfg(target_os = "macos")]
mod app_icon;
mod app_view;
mod current_repo;
mod gitlab_connection;
mod diff_window;
mod file_tree;
mod settings_window;
mod text_field;
#[cfg(target_os = "macos")]
mod mac_column_vibrancy;
mod metadata;
mod scrollbar;
mod splitter;
mod theme;
mod window_controls;
mod window_geometry;

use gpui::{
    App, AppContext, Application, AssetSource, KeyBinding, Menu, MenuItem, SharedString,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowHandle,
    WindowOptions, actions, point, px,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use app_view::AppView;
use crate::git::BranchBrowser;
use crate::reqwest_client::ReqwestClient;
use crate::window_geometry_store;
use crate::workspace_store;
use gitlab_connection::GitLabConnection;
use settings_window::SettingsView;
use text_field::{
    Backspace, Copy, Cut, Delete, End, Home, Left, Paste, Right, SelectAll, SelectLeft,
    SelectRight, ShowCharacterPalette,
};

actions!(app, [Quit, OpenSettings]);

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
            "gitlab.svg" => Some(Cow::Borrowed(
                include_bytes!("../../assets/gitlab.svg") as &'static [u8],
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
    let geometry = window_geometry_store::load();
    let restore_diff = if geometry.diff_open {
        geometry.diff_reopen.clone()
    } else {
        None
    };

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        app_icon::set_app_icon();

        let http_client =
            ReqwestClient::user_agent(concat!("ReviewFox/", env!("CARGO_PKG_VERSION")))
                .expect("HTTP client");
        cx.set_http_client(Arc::new(http_client));

        let settings_window: Rc<RefCell<Option<WindowHandle<SettingsView>>>> =
            Rc::new(RefCell::new(None));
        let gitlab_connection = Rc::new(RefCell::new(GitLabConnection::default()));

        cx.on_action(|_: &Quit, cx| {
            window_geometry_store::begin_quit();
            window_geometry_store::flush();
            cx.quit();
        });
        cx.on_action({
            let settings_window = settings_window.clone();
            let gitlab_connection = gitlab_connection.clone();
            move |_: &OpenSettings, cx| {
                settings_window::open_or_focus_settings(
                    &mut settings_window.borrow_mut(),
                    gitlab_connection.clone(),
                    cx,
                );
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("cmd-comma", OpenSettings, None),
            KeyBinding::new("ctrl-comma", OpenSettings, None),
            KeyBinding::new("backspace", Backspace, Some("TextField")),
            KeyBinding::new("delete", Delete, Some("TextField")),
            KeyBinding::new("left", Left, Some("TextField")),
            KeyBinding::new("right", Right, Some("TextField")),
            KeyBinding::new("shift-left", SelectLeft, Some("TextField")),
            KeyBinding::new("shift-right", SelectRight, Some("TextField")),
            KeyBinding::new("cmd-a", SelectAll, Some("TextField")),
            KeyBinding::new("ctrl-a", SelectAll, Some("TextField")),
            KeyBinding::new("cmd-v", Paste, Some("TextField")),
            KeyBinding::new("ctrl-v", Paste, Some("TextField")),
            KeyBinding::new("cmd-c", Copy, Some("TextField")),
            KeyBinding::new("ctrl-c", Copy, Some("TextField")),
            KeyBinding::new("cmd-x", Cut, Some("TextField")),
            KeyBinding::new("ctrl-x", Cut, Some("TextField")),
            KeyBinding::new("home", Home, Some("TextField")),
            KeyBinding::new("end", End, Some("TextField")),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("TextField")),
        ]);
        cx.set_menus(vec![Menu {
            name: "ReviewFox".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ],
        }]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                window_geometry_store::begin_quit();
                window_geometry_store::flush();
                cx.quit();
            }
        })
        .detach();

        let geometry = window_geometry_store::snapshot();
        let bounds = window_geometry::resolve_bounds(
            geometry.main.as_ref(),
            window_geometry::main_default_size(),
            cx,
        );

        let boot = boot.clone();
        let gitlab_connection = gitlab_connection.clone();
        let restore_diff = restore_diff.clone();
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
                move |_, cx| {
                    cx.new(|cx| AppView::new(boot, gitlab_connection, restore_diff, cx))
                },
            )
            .expect("open main window");

        // Diff opens via Open Diff on the main Changes chrome, or boot restore.
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

#[cfg(target_os = "macos")]
pub(crate) fn window_background_appearance() -> WindowBackgroundAppearance {
    // Column frost is a DIY NSVisualEffectView; avoid full-window Blurred.
    WindowBackgroundAppearance::Transparent
}

#[cfg(target_os = "windows")]
pub(crate) fn window_background_appearance() -> WindowBackgroundAppearance {
    WindowBackgroundAppearance::Blurred
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn window_background_appearance() -> WindowBackgroundAppearance {
    WindowBackgroundAppearance::Opaque
}

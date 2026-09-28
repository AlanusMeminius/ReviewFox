#[cfg(target_os = "macos")]
mod app_icon;
mod appearance;
mod app_view;
mod entry_chrome;
mod diff;
mod diff_window;
mod file_tree;
mod file_tree_rows;
mod gitlab_connection;
mod icon_button;
#[cfg(target_os = "macos")]
mod mac_column_vibrancy;
mod markdown;
mod metadata;
mod scrollbar;
mod selectable_markdown;
mod settings;
mod splitter;
mod text_field;
mod theme;
mod tooltip;
mod window_controls;
mod window_geometry;

use gpui::{
    App, AppContext, Application, AssetSource, KeyBinding, Menu, MenuItem, SharedString,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions,
    actions, point, px,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::git::BranchBrowser;
use crate::reqwest_client::ReqwestClient;
use crate::window_geometry_store;
use crate::workspace_store;
use app_view::AppView;
use gitlab_connection::GitLabConnection;
use text_field::{
    Backspace, Confirm, Copy, Cut, Delete, End, Home, Left, Paste, Right, SelectAll, SelectLeft,
    SelectRight, ShowCharacterPalette,
};

actions!(app, [Quit, OpenSettings]);

struct Assets;

/// Icons are Lucide (ISC, lucide.dev), 24 viewBox, stroke 2. GPUI draws each as a
/// mask tinted by `text_color`, so the stroke colour in the file doesn't matter.
macro_rules! icon_assets {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../assets/", $name)) as &[u8])),*]
    };
}

const ICON_ASSETS: &[(&str, &[u8])] = icon_assets![
    "sidebar_title.svg",
    "diff_title.svg",
    "branch.svg",
    "folder.svg",
    "folder_open.svg",
    "gitlab.svg",
    "export.svg",
    "gear.svg",
    "check.svg",
    "warning.svg",
    "undo.svg",
    "refresh.svg",
    "chevron_right.svg",
    "chevron_down.svg",
    "arrow_up.svg",
    "arrow_down.svg",
    "unfold_vertical.svg",
    "fold_vertical.svg",
    "pilcrow.svg",
    "minus.svg",
    "plus.svg",
    "search.svg",
    "pin.svg",
    "pin_off.svg",
    "trash.svg",
    "square_minus.svg",
    "square_plus.svg",
    "diff.svg",
    "chevrons_up_down.svg",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICON_ASSETS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
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
        appearance::init(cx);
        // Compile highlight queries off the UI thread before the first open.
        cx.background_spawn(async {
            crate::syntax::warm();
            let _ = theme::syntax_colors();
        })
        .detach();
        #[cfg(target_os = "macos")]
        app_icon::set_app_icon();

            let http_client =
                ReqwestClient::user_agent(concat!("ReviewFox/", env!("CARGO_PKG_VERSION")))
                    .expect("HTTP client");
            cx.set_http_client(Arc::new(http_client));

            let gitlab_connection = Rc::new(RefCell::new(GitLabConnection::default()));
            settings::init(gitlab_connection.clone(), cx);

            cx.on_action(|_: &Quit, cx| {
                window_geometry_store::begin_quit();
                window_geometry_store::flush();
                cx.quit();
            });
            // Keys, gear and menu open without a target; error links call
            // `settings::open_or_focus_settings` with one.
            cx.on_action(|_: &OpenSettings, cx| settings::open_or_focus_settings(None, cx));
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("cmd-comma", OpenSettings, None),
                KeyBinding::new("ctrl-comma", OpenSettings, None),
                KeyBinding::new("backspace", Backspace, Some("TextField")),
                KeyBinding::new("delete", Delete, Some("TextField")),
                KeyBinding::new("enter", Confirm, Some("TextField")),
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
            cx.bind_keys(settings::key_bindings());
            cx.bind_keys(diff_window::key_bindings());
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

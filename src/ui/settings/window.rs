use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable, Global, KeyBinding,
    Render, ScrollHandle, Subscription, Window, WindowControlArea, WindowDecorations,
    WindowHandle, actions, div, point, prelude::*, px, size,
};

use std::cell::RefCell;
use std::rc::Rc;

use crate::gitlab::SettingsTarget;
use crate::settings_store::{self, DEFAULT_BASE_URL};

use super::nav::{NavItem, SettingsNav};
use super::nav_tree::{NavEntry, NavPage, NavState};
use super::token_row::{self, CardStatus, KeychainError, TokenRow};
use super::{Button, ButtonSize, ButtonStyle, ConfiguredCard, SectionHeader, SettingRow};
use crate::ui::gitlab_connection::{self, GitLabConnection};
#[cfg(target_os = "macos")]
use crate::ui::mac_column_vibrancy::ColumnVibrancy;
use crate::ui::scrollbar;
use crate::ui::text_field::{TextField, TextFieldEvent, TextFieldStyle};
use crate::ui::theme;
use crate::ui::window_controls::window_controls;

actions!(
    settings,
    [
        CloseSettings,
        FocusNextControl,
        FocusPrevControl,
        NavUp,
        NavDown,
        NavExpand,
        NavCollapse,
    ]
);

/// Key context on the whole Settings view.
const CONTEXT: &str = "Settings";
/// Key context on the nav tree (only while it has focus).
const NAV_CONTEXT: &str = "SettingsNav";
const CONTENT_SCROLL_ID: &str = "settings-content-sb";

#[cfg(target_os = "macos")]
const CLOSE_KEY: &str = "cmd-w";
#[cfg(not(target_os = "macos"))]
const CLOSE_KEY: &str = "ctrl-w";

/// Tab order: nav, GitLab URL, then the token input or the token card's buttons.
const TAB_NAV: isize = 0;
const TAB_URL: isize = 1;
const TAB_TOKEN: isize = 2;

const URL_TITLE: &str = "GitLab URL";
const URL_DESCRIPTION: &str = "Your self-hosted GitLab address. Leave empty for gitlab.com.";
const URL_PLACEHOLDER: &str = "https://gitlab.com";
const RESET_TO_DEFAULT: &str = "Reset to Default";

/// Settings-window bindings, scoped to [`CONTEXT`] / [`NAV_CONTEXT`].
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("escape", CloseSettings, Some(CONTEXT)),
        KeyBinding::new(CLOSE_KEY, CloseSettings, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNextControl, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevControl, Some(CONTEXT)),
        KeyBinding::new("up", NavUp, Some(NAV_CONTEXT)),
        KeyBinding::new("down", NavDown, Some(NAV_CONTEXT)),
        KeyBinding::new("right", NavExpand, Some(NAV_CONTEXT)),
        KeyBinding::new("left", NavCollapse, Some(NAV_CONTEXT)),
    ]
}

/// Level-2 nav entries; each renders one block of its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    GitLab,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::GitLab => "GitLab",
        }
    }
}

/// The nav tree. A new page or section is a new entry here plus its render arm.
const PAGES: &[NavPage<Section>] = &[NavPage {
    title: "Accounts",
    sections: &[Section::GitLab],
    expanded: true,
}];

/// The nav entry of `section`.
fn nav_entry(section: Section) -> NavEntry {
    PAGES
        .iter()
        .enumerate()
        .find_map(|(page, p)| {
            let index = p.sections.iter().position(|&s| s == section)?;
            Some(NavEntry::Section {
                page,
                section: index,
            })
        })
        .expect("every Section is listed in PAGES")
}

pub struct SettingsView {
    focus: FocusHandle,
    base_url: Entity<TextField>,
    /// Normalized base URL as saved in `settings.json` (the default when unset).
    saved_base_url: String,
    pat: Entity<TextField>,
    /// A token is stored in the keychain (cached; the keychain is only read on open).
    has_saved_token: bool,
    keychain_error: Option<KeychainError>,
    gitlab_connection: Rc<RefCell<GitLabConnection>>,
    nav_focus: FocusHandle,
    nav: NavState,
    /// Last nav interaction came from the keyboard: show the focus border.
    nav_keyboard: bool,
    content_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    #[cfg(target_os = "macos")]
    window_vibrancy: Option<ColumnVibrancy>,
}

impl SettingsView {
    pub fn new(
        gitlab_connection: Rc<RefCell<GitLabConnection>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let saved_base_url = settings_store::effective_base_url(&settings_store::load_file());
        let base_url = cx.new(|cx| {
            TextField::new(URL_PLACEHOLDER, false, cx)
                .with_style(TextFieldStyle::Settings)
                .tab_index(TAB_URL)
        });
        base_url.update(cx, |field, cx| field.set_content(saved_base_url.clone(), cx));

        let pat = cx.new(|cx| {
            TextField::new(token_row::PLACEHOLDER, true, cx)
                .with_style(TextFieldStyle::Settings)
                .tab_index(TAB_TOKEN)
        });
        let has_saved_token = settings_store::load_pat().is_some_and(|p| !p.trim().is_empty());

        let base_url_focus = base_url.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe(&base_url, |view, _, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Confirm => view.commit_base_url(cx),
            }),
            cx.on_blur(&base_url_focus, window, |view, _, cx| view.commit_base_url(cx)),
            // The token commits on Enter only, never on blur.
            cx.subscribe(&pat, |view, _, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Confirm => view.commit_token(cx),
            }),
        ];

        // The scroll handle lives in a global registry and outlives the window.
        let (content_scroll, _) = scrollbar::vertical(CONTENT_SCROLL_ID, cx);
        content_scroll.set_offset(point(px(0.), px(0.)));

        let view = Self {
            focus: cx.focus_handle(),
            base_url,
            saved_base_url,
            pat,
            has_saved_token,
            keychain_error: None,
            gitlab_connection,
            nav_focus: cx.focus_handle().tab_index(TAB_NAV).tab_stop(true),
            nav: NavState::new(PAGES),
            nav_keyboard: false,
            content_scroll,
            _subscriptions: subscriptions,
            #[cfg(target_os = "macos")]
            window_vibrancy: None,
        };
        // Re-check on open (the token may have been revoked), unless a check
        // is already in flight.
        if !matches!(*view.gitlab_connection.borrow(), GitLabConnection::Checking) {
            view.refresh_connection(cx);
        }
        view
    }

    /// Re-verify the saved URL + token through the shared connection, so the
    /// main window sees the same state.
    fn refresh_connection(&self, cx: &mut Context<Self>) {
        gitlab_connection::spawn_refresh_connection(
            self.gitlab_connection.clone(),
            cx.entity().downgrade(),
            cx,
        );
    }

    /// Blur / Enter on the URL field: normalize, save, show the normalized
    /// value, and re-verify when a token exists and the URL changed.
    fn commit_base_url(&mut self, cx: &mut Context<Self>) {
        let raw = self.base_url.read(cx).content().to_string();
        let normalized = settings_store::normalize_base_url(&raw);
        if raw != normalized {
            self.base_url
                .update(cx, |field, cx| field.set_content(normalized.clone(), cx));
        }
        if normalized == self.saved_base_url {
            return;
        }

        let mut file = settings_store::load_file();
        file.gitlab_base_url = (normalized != DEFAULT_BASE_URL).then(|| normalized.clone());
        settings_store::save_file(&file);
        self.saved_base_url = normalized;
        if self.has_saved_token {
            self.refresh_connection(cx);
        }
        cx.notify();
    }

    fn reset_base_url(&mut self, cx: &mut Context<Self>) {
        self.base_url
            .update(cx, |field, cx| field.set_content(DEFAULT_BASE_URL, cx));
        self.commit_base_url(cx);
    }

    /// Enter in the token field: store it (whatever verify later says), then verify.
    fn commit_token(&mut self, cx: &mut Context<Self>) {
        let token = self.pat.read(cx).content().trim().to_string();
        if token.is_empty() {
            return;
        }
        match settings_store::save_pat(&token) {
            Ok(()) => {
                self.has_saved_token = true;
                self.keychain_error = None;
                self.pat.update(cx, |field, cx| field.set_content("", cx));
                self.refresh_connection(cx);
            }
            Err(e) => self.keychain_error = Some(KeychainError::Save(e.to_string())),
        }
        cx.notify();
    }

    /// `Reset Token`: delete the keychain entry now and go back to the input.
    fn reset_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // After a failed save there is nothing stored; just dismiss the error.
        if self.has_saved_token {
            match settings_store::clear_pat() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => {
                    self.keychain_error = Some(KeychainError::Clear(e.to_string()));
                    cx.notify();
                    return;
                }
            }
        }
        self.has_saved_token = false;
        self.keychain_error = None;
        gitlab_connection::set_no_pat(&self.gitlab_connection);
        self.pat.update(cx, |field, cx| field.set_content("", cx));
        let pat_focus = self.pat.read(cx).focus_handle(cx);
        window.focus(&pat_focus);
        cx.notify();
    }
}

// Nav tree and window keyboard.
impl SettingsView {
    /// `Open Settings` from an error: select Accounts › GitLab, scroll to it
    /// and focus the field to fix. The token input only takes focus while the
    /// row shows it; behind a card, focus goes to the nav (nothing in content).
    fn open_target(&mut self, target: SettingsTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.nav.select(nav_entry(Section::GitLab));
        self.nav_keyboard = false;
        self.reveal_selected();
        let field = match target {
            SettingsTarget::GitLabUrl => Some(&self.base_url),
            SettingsTarget::GitLabToken => {
                let row = token_row::token_row(
                    self.has_saved_token,
                    &self.gitlab_connection.borrow(),
                    self.keychain_error.as_ref(),
                );
                matches!(row, TokenRow::Input).then_some(&self.pat)
            }
        };
        let focus = match field {
            Some(field) => field.read(cx).focus_handle(cx),
            None => self.nav_focus.clone(),
        };
        window.focus(&focus);
        cx.notify();
    }

    /// Scroll the content to the selected entry: a page to the top, a section
    /// to its header. Child 0 of the scroll column is the page title.
    fn reveal_selected(&self) {
        match self.nav.selected() {
            NavEntry::Page(_) => self.content_scroll.set_offset(point(px(0.), px(0.))),
            NavEntry::Section { section, .. } => {
                self.content_scroll.scroll_to_top_of_item(section + 1)
            }
        }
    }

    fn click_nav(&mut self, entry: NavEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.nav.select(entry);
        self.nav_keyboard = false;
        window.focus(&self.nav_focus);
        self.reveal_selected();
        cx.notify();
    }

    fn toggle_nav(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.nav.toggle(page) {
            self.reveal_selected();
        }
        cx.notify();
    }

    /// Arrow keys in the tree; `moved` is what the [`NavState`] move returned.
    fn nav_key(&mut self, moved: bool, cx: &mut Context<Self>) {
        self.nav_keyboard = true;
        if moved {
            self.reveal_selected();
        }
        cx.notify();
    }

    fn focus_control(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if next {
            window.focus_next();
        } else {
            window.focus_prev();
        }
        self.nav_keyboard = self.nav_focus.is_focused(window);
        cx.notify();
    }

    fn render_nav(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.nav.selected();
        let focused = self.nav_keyboard && self.nav_focus.is_focused(window);
        let items = self
            .nav
            .visible()
            .into_iter()
            .map(|entry| {
                let item = match entry {
                    NavEntry::Page(page) => NavItem::page(
                        ("settings-nav-page", page),
                        PAGES[page].title,
                        self.nav.is_expanded(page),
                    )
                    .on_toggle(cx.listener(move |view, _, _, cx| view.toggle_nav(page, cx))),
                    NavEntry::Section { page, section } => NavItem::section(
                        ElementId::Name(format!("settings-nav-{page}-{section}").into()),
                        PAGES[page].sections[section].title(),
                    ),
                };
                item.selected(entry == selected)
                    .focused(focused && entry == selected)
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.click_nav(entry, window, cx)),
                    )
            })
            .collect();

        div()
            .id("settings-nav")
            .key_context(NAV_CONTEXT)
            .track_focus(&self.nav_focus)
            .flex_none()
            .h_full()
            .on_action(cx.listener(|view, _: &NavUp, _, cx| {
                let moved = view.nav.select_prev();
                view.nav_key(moved, cx);
            }))
            .on_action(cx.listener(|view, _: &NavDown, _, cx| {
                let moved = view.nav.select_next();
                view.nav_key(moved, cx);
            }))
            .on_action(cx.listener(|view, _: &NavExpand, _, cx| {
                let moved = view.nav.expand();
                view.nav_key(moved, cx);
            }))
            .on_action(cx.listener(|view, _: &NavCollapse, _, cx| {
                let moved = view.nav.collapse();
                view.nav_key(moved, cx);
            }))
            .child(SettingsNav::new(items))
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = &PAGES[self.nav.selected().page()];
        let (scroll, scrollbar) = scrollbar::vertical(CONTENT_SCROLL_ID, cx);
        let sections: Vec<_> = page
            .sections
            .iter()
            .map(|&section| {
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .child(SectionHeader::new(section.title()))
                    .child(match section {
                        Section::GitLab => self.render_gitlab_section(cx),
                    })
            })
            .collect();

        div()
            .id("settings-island")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(theme::white())
            .rounded(px(theme::CHANGES_RADIUS))
            .overflow_hidden()
            // gpui clips the scrolling content to this rect, not to its radius, so
            // hold it one radius clear of the top and bottom and let the island
            // draw its own corners (same as the Diff window's island).
            .py(px(theme::CHANGES_RADIUS))
            .child(scrollbar::overlay_flex(
                div()
                    .id("settings-content")
                    .size_full()
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .px(px(32.))
                    // 24px from the island's top edge, counting its radius padding.
                    .pt(px(24. - theme::CHANGES_RADIUS))
                    .child(
                        div()
                            .flex_none()
                            .pt(px(8.))
                            .pb(px(12.))
                            .text_size(px(16.))
                            .text_color(theme::text())
                            .child(page.title),
                    )
                    .children(sections),
                scrollbar,
            ))
    }

    /// Accounts › GitLab: the URL row, then the token row.
    fn render_gitlab_section(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(self.render_url_row(cx))
            .child(self.render_token_row(cx))
            .into_any_element()
    }

    fn render_url_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let non_default = self.saved_base_url != DEFAULT_BASE_URL;
        SettingRow::new("settings-gitlab-url", URL_TITLE)
            .description(URL_DESCRIPTION)
            .when(non_default, |row| {
                row.title_action(
                    Button::icon_only("settings-gitlab-url-reset", "undo.svg")
                        .tooltip(RESET_TO_DEFAULT)
                        .on_click(cx.listener(|view, _, _, cx| view.reset_base_url(cx))),
                )
            })
            .control(self.base_url.clone())
    }

    fn render_token_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = token_row::token_row(
            self.has_saved_token,
            &self.gitlab_connection.borrow(),
            self.keychain_error.as_ref(),
        );
        let (status, label, can_retry) = match row {
            TokenRow::Input => {
                return SettingRow::new("settings-gitlab-token", token_row::TITLE)
                    .narrow()
                    .last(true)
                    .description(self.token_description())
                    .control(self.pat.clone())
                    .into_any_element();
            }
            TokenRow::Card {
                status,
                label,
                can_retry,
            } => (status, label, can_retry),
        };

        let mut card = ConfiguredCard::new(label);
        match status {
            CardStatus::Verifying => {}
            CardStatus::Connected => card = card.icon("check.svg", theme::success()),
            CardStatus::Failed => card = card.icon("warning.svg", theme::error()),
        }
        let mut tab_index = TAB_TOKEN;
        if can_retry {
            card = card.action(
                card_button("settings-gitlab-token-retry", token_row::RETRY, "refresh.svg")
                    .tab_index(tab_index)
                    .on_click(cx.listener(|view, _, _, cx| view.refresh_connection(cx))),
            );
            tab_index += 1;
        }
        card = card.action(
            card_button(
                "settings-gitlab-token-reset",
                token_row::RESET_TOKEN,
                "undo.svg",
            )
            .tab_index(tab_index)
            .on_click(cx.listener(|view, _, window, cx| view.reset_token(window, cx))),
        );

        // Like Zed's configured API key, the card stands in for the whole row
        // (last row of the section: 40px bottom padding, no divider).
        div()
            .id("settings-gitlab-token")
            .pt(px(16.))
            .pb(px(40.))
            .child(card)
            .into_any_element()
    }

    /// `Create one with the read_api scope in GitLab access tokens.`, the link
    /// opening `{base}/-/user_settings/personal_access_tokens` in the browser.
    fn token_description(&self) -> impl IntoElement {
        let url = token_row::access_tokens_url(&self.saved_base_url);
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .child(token_row::DESCRIPTION_BEFORE_LINK)
            .child(
                div()
                    .id("settings-gitlab-token-link")
                    .underline()
                    .cursor_pointer()
                    .hover(|link| link.text_color(theme::text()))
                    .child(token_row::DESCRIPTION_LINK)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(token_row::DESCRIPTION_AFTER_LINK)
    }
}

/// Bare drag band across the window: no title text (the page title already
/// heads the island), traffic lights on macOS, caption buttons on Windows.
fn render_titlebar(window: &Window) -> impl IntoElement {
    div()
        .id("settings-titlebar")
        .h(theme::TITLEBAR_HEIGHT)
        .flex_none()
        .flex()
        .child(
            div()
                .id("settings-titlebar-drag")
                .h_full()
                .flex_1()
                .min_w(px(0.))
                .window_control_area(WindowControlArea::Drag)
                .occlude(),
        )
        .children(window_controls(window))
}

/// Zed `ConfiguredApiCard` button: Subtle, Default size, small label, muted icon.
fn card_button(id: &'static str, label: &'static str, icon: &'static str) -> Button {
    Button::new(id, label)
        .style(ButtonStyle::Subtle)
        .size(ButtonSize::Default)
        .small_label()
        .start_icon(icon)
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        {
            // Full window: the island is inset on every side, so the gutters need
            // material under them too.
            ColumnVibrancy::ensure_synced_window(&mut self.window_vibrancy, window);
        }

        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            // The window's one translucent layer; the nav and the gutters stay
            // clear so it is not painted twice.
            .bg(theme::frost())
            .font_family(theme::UI_FONT)
            .text_color(theme::text())
            .child(render_titlebar(window))
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_row()
                    .gap(px(theme::CHANGES_INSET))
                    .pt(px(theme::CHANGES_TOP_INSET))
                    .pr(px(theme::CHANGES_INSET))
                    .pb(px(theme::CHANGES_INSET))
                    .child(self.render_nav(window, cx))
                    .child(self.render_content(cx)),
            )
            .on_action(cx.listener(|view, _: &CloseSettings, window, cx| {
                // Closing does not blur the field; keep an unsaved URL edit.
                view.commit_base_url(cx);
                window.remove_window();
            }))
            .on_action(cx.listener(|view, _: &FocusNextControl, window, cx| {
                view.focus_control(true, window, cx)
            }))
            .on_action(cx.listener(|view, _: &FocusPrevControl, window, cx| {
                view.focus_control(false, window, cx)
            }))
    }
}

/// The Settings window (at most one) and the connection it verifies through.
struct SettingsWindow {
    handle: Option<WindowHandle<SettingsView>>,
    gitlab_connection: Rc<RefCell<GitLabConnection>>,
}

impl Global for SettingsWindow {}

/// Call once at startup, before [`open_or_focus_settings`].
pub fn init(gitlab_connection: Rc<RefCell<GitLabConnection>>, cx: &mut App) {
    cx.set_global(SettingsWindow {
        handle: None,
        gitlab_connection,
    });
}

/// Open Settings, or focus it when already open. With a `target`
/// (`Open Settings` on an error) also select and focus the field to fix;
/// without one (keys, gear, menu) leave the window as it is.
pub fn open_or_focus_settings(target: Option<SettingsTarget>, cx: &mut App) {
    let (handle, gitlab_connection) = {
        let state = cx.global::<SettingsWindow>();
        (state.handle, state.gitlab_connection.clone())
    };
    if let Some(h) = handle
        && h
            .update(cx, |view, window, cx| {
                window.activate_window();
                if let Some(target) = target {
                    view.open_target(target, window, cx);
                }
            })
            .is_ok()
    {
        return;
    }

    let bounds = gpui::Bounds::centered(None, size(px(760.), px(520.)), cx);
    match cx.open_window(
        gpui::WindowOptions {
            focus: true,
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Settings".into()),
                appears_transparent: true,
                traffic_light_position: crate::ui::traffic_light_position(),
                ..Default::default()
            }),
            window_min_size: Some(size(px(640.), px(400.))),
            // Frosted desk like the main and Diff windows: nav on the material,
            // content on a white island.
            window_decorations: Some(WindowDecorations::Client),
            window_background: crate::ui::window_background_appearance(),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| SettingsView::new(gitlab_connection, window, cx));
            // Arrow keys work in the tree straight away; no focus border until used.
            let nav_focus = view.read(cx).nav_focus.clone();
            window.focus(&nav_focus);
            if let Some(target) = target {
                view.update(cx, |view, cx| view.open_target(target, window, cx));
            }
            view
        },
    ) {
        Ok(h) => cx.global_mut::<SettingsWindow>().handle = Some(h),
        Err(e) => eprintln!("failed to open settings: {e}"),
    }
}

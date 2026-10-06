//! Native implementation of the selected time-inbox prototype (variant A).
use super::*;
use crate::gitlab::inbox::{self, InboxMr, Period, Preview};
use chrono::Local;
use futures::{StreamExt, stream};

#[derive(Clone, Debug, PartialEq, Eq)]
struct InboxRepository {
    path: PathBuf,
    name: String,
}

#[derive(Clone)]
struct Row {
    repository: InboxRepository,
    project: String,
    mr: InboxMr,
}

impl Row {
    fn key(&self) -> (PathBuf, u64) {
        (self.repository.path.clone(), self.mr.iid)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Repositories,
    Period,
}

#[derive(Default)]
pub(super) struct Inbox {
    pub visible: bool,
    pub menu_open: bool,
    menu: Option<Menu>,
    menu_index: usize,
    repositories: Vec<InboxRepository>,
    excluded: HashSet<PathBuf>,
    period: Period,
    base: String,
    rows: Vec<Row>,
    selected: Option<(PathBuf, u64)>,
    preview: Option<Result<Preview, ErrorNote>>,
    errors: Vec<(String, ErrorNote)>,
    loading: bool,
    loaded: bool,
    loaded_day: Option<chrono::NaiveDate>,
    generation: u64,
    preview_generation: u64,
    list_task: Option<gpui::Task<()>>,
    preview_task: Option<gpui::Task<()>>,
    repo_bounds: Rc<Cell<Bounds<Pixels>>>,
    period_bounds: Rc<Cell<Bounds<Pixels>>>,
    detail_width: Option<f32>,
    resize_state: Rc<ResizeState>,
}

impl Inbox {
    fn finish_load(
        &mut self,
        generation: u64,
        results: Vec<(String, Result<Vec<Row>, ErrorNote>)>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading = false;
        for (name, result) in results {
            match result {
                Ok(rows) => self.rows.extend(rows),
                Err(error) => self.errors.push((name, error)),
            }
        }
        self.rows.sort_by(|a, b| {
            b.mr.updated_at
                .cmp(&a.mr.updated_at)
                .then_with(|| a.key().cmp(&b.key()))
        });
        if self.selected_row().is_none() {
            self.selected = self.rows.first().map(Row::key);
        }
        true
    }

    fn finish_preview(&mut self, generation: u64, result: Result<Preview, ErrorNote>) -> bool {
        if self.preview_generation != generation {
            return false;
        }
        self.preview = Some(result);
        true
    }

    fn selected_row(&self) -> Option<&Row> {
        self.rows
            .iter()
            .find(|r| self.selected.as_ref() == Some(&r.key()))
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
        self.menu_open = false;
    }

    fn toggle_menu(&mut self, menu: Menu) {
        self.menu = if self.menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        self.menu_open = self.menu.is_some();
        self.menu_index = 0;
    }
}

fn inbox_available_width(view: &AppView, window: &Window) -> f32 {
    let rail = if view.repos_collapsed {
        0.
    } else {
        view.sidebar_width + theme::CHANGES_SHADOW_GAP
    };
    (f32::from(window.viewport_size().width)
        - rail
        - theme::left_island_inset(!view.repos_collapsed)
        - theme::CHANGES_INSET
        - theme::CHANGES_SHADOW_GAP)
        .max(0.)
}

fn inbox_detail_width(requested: Option<f32>, available: f32) -> f32 {
    let minimum = 240_f32.min(available / 2.);
    requested
        .unwrap_or(available / 2.)
        .clamp(minimum, available - minimum)
}

impl AppView {
    fn inbox_resize_handler(&self, cx: &Context<Self>) -> splitter::ResizeHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |raw, window, cx| {
            view.update(cx, |this, cx| {
                let width = inbox_detail_width(
                    Some(raw - theme::CHANGES_INSET - theme::CHANGES_SHADOW_GAP / 2.),
                    inbox_available_width(this, window),
                );
                this.inbox.detail_width = Some(width);
                cx.notify();
            })
            .ok();
        })
    }

    pub(super) fn show_inbox(&mut self, cx: &mut Context<Self>) {
        self.refresh_store();
        self.inbox.visible = true;
        self.branch_picker = None;
        self.mr_picker = None;
        self.repo_menu = None;
        self.commit_menu = None;
        self.inbox.close_menu();
        let base = settings_store::effective_base_url(&settings_store::load_file());
        let repositories: Vec<_> = workspace_store::sorted_workspaces(&self.store.workspaces)
            .into_iter()
            .filter(|r| gitlab::repo_matches_settings_host(&r.path, &base))
            .map(|r| InboxRepository {
                name: Repository::new(r.path.clone()).display_name(),
                path: r.path,
            })
            .collect();
        let changed = self.inbox.repositories != repositories || self.inbox.base != base;
        self.inbox.repositories = repositories;
        self.inbox.base = base;
        if changed || !self.inbox.loaded || self.inbox.loaded_day != Some(Local::now().date_naive())
        {
            self.refresh_inbox(cx);
        }
        cx.notify();
    }

    fn refresh_inbox(&mut self, cx: &mut Context<Self>) {
        self.inbox.generation = self.inbox.generation.wrapping_add(1);
        self.inbox.preview_generation = self.inbox.preview_generation.wrapping_add(1);
        self.inbox.list_task = None;
        self.inbox.preview_task = None;
        self.inbox.preview = None;
        self.inbox.rows.clear();
        self.inbox.errors.clear();
        self.inbox.loaded = true;
        self.inbox.loaded_day = Some(Local::now().date_naive());
        let (scroll, _) = scrollbar::vertical("inbox-list-sb", cx);
        scroll.set_offset(gpui::point(px(0.), px(0.)));
        let repos: Vec<_> = self
            .inbox
            .repositories
            .iter()
            .filter(|r| !self.inbox.excluded.contains(&r.path))
            .cloned()
            .collect();
        self.inbox.loading = !repos.is_empty();
        if repos.is_empty() {
            self.inbox.selected = None;
            cx.notify();
            return;
        }
        let generation = self.inbox.generation;
        let base = self.inbox.base.clone();
        let pat = settings_store::load_pat().unwrap_or_default();
        let after = self.inbox.period.start(Local::now());
        let http = cx.http_client();
        self.inbox.list_task = Some(cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    stream::iter(repos.into_iter().map(|repo| {
                        let http = http.clone();
                        let base = base.clone();
                        let pat = pat.clone();
                        async move {
                            let result = async {
                                if pat.trim().is_empty() {
                                    return Err(ErrorNote::new(
                                        gitlab::format_list_merge_requests_error(
                                            &gitlab::ListMergeRequestsError::MissingPat,
                                        ),
                                        Some(SettingsTarget::GitLabToken),
                                    ));
                                }
                                let identity = match gitlab::resolve_project(
                                    http.clone(),
                                    &base,
                                    &pat,
                                    &repo.path,
                                )
                                .await
                                {
                                    ResolveProjectResult::Ok(p) => p,
                                    ResolveProjectResult::Err(e) => {
                                        return Err(ErrorNote::resolve_project(&e));
                                    }
                                };
                                inbox::list(http, &base, &pat, &identity.path_with_namespace, after)
                                    .await
                                    .map(|mrs| {
                                        mrs.into_iter()
                                            .map(|mr| Row {
                                                repository: repo.clone(),
                                                project: identity.path_with_namespace.clone(),
                                                mr,
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                    .map_err(|e| {
                                        ErrorNote::new(
                                            gitlab::format_list_merge_requests_error(&e),
                                            e.settings_fix(),
                                        )
                                    })
                            }
                            .await;
                            (repo.name, result)
                        }
                    }))
                    .buffer_unordered(4)
                    .collect::<Vec<_>>()
                    .await
                })
                .await;
            this.update(cx, |this, cx| {
                if !this.inbox.finish_load(generation, results) {
                    return;
                }
                this.load_inbox_preview(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn select_inbox_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.inbox.rows.get(index) else {
            return;
        };
        if self.inbox.selected.as_ref() != Some(&row.key()) {
            self.inbox.selected = Some(row.key());
            self.load_inbox_preview(cx);
        }
        cx.notify();
    }

    fn load_inbox_preview(&mut self, cx: &mut Context<Self>) {
        self.inbox.preview_generation = self.inbox.preview_generation.wrapping_add(1);
        self.inbox.preview_task = None;
        self.inbox.preview = None;
        let (scroll, _) = scrollbar::vertical("inbox-detail-sb", cx);
        scroll.set_offset(gpui::point(px(0.), px(0.)));
        let Some(row) = self.inbox.selected_row().cloned() else {
            return;
        };
        let generation = self.inbox.preview_generation;
        let base = self.inbox.base.clone();
        let pat = settings_store::load_pat().unwrap_or_default();
        let http = cx.http_client();
        self.inbox.preview_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    inbox::preview(http, &base, &pat, &row.project, row.mr.iid)
                        .await
                        .map_err(|e| {
                            ErrorNote::new(
                                gitlab::format_fetch_merge_request_error(&e),
                                e.settings_fix(),
                            )
                        })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.inbox.finish_preview(generation, result) {
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn open_inbox_mr(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.inbox.selected_row().cloned() else {
            return;
        };
        // Open exactly this Repository; do not briefly restore its remembered MR.
        let Some(workspace) = self
            .store
            .workspaces
            .iter()
            .find(|w| w.path == row.repository.path)
            .cloned()
        else {
            return;
        };
        match BranchBrowser::open_workspace(&workspace) {
            Ok(browser) => {
                self.clear_uncommitted();
                self.state = MainState::Ready(LoadedBrowser::install(browser));
                self.branch_picker = None;
                self.mr_picker = None;
                self.inbox.visible = false;
                self.inbox.close_menu();
                self.select_mr(row.mr.summary(), cx);
            }
            Err(e) => {
                self.inbox
                    .errors
                    .push((row.repository.name, ErrorNote::plain(e.0)));
                self.refresh_store();
                cx.notify();
            }
        }
    }

    pub(super) fn handle_inbox_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if let Some(menu) = self.inbox.menu {
            let len = match menu {
                Menu::Repositories => self.inbox.repositories.len(),
                Menu::Period => 5,
            };
            match key {
                "escape" => self.inbox.close_menu(),
                "up" => self.inbox.menu_index = self.inbox.menu_index.saturating_sub(1),
                "down" => {
                    self.inbox.menu_index = (self.inbox.menu_index + 1).min(len.saturating_sub(1))
                }
                "enter" | "space" => self.choose_inbox_option(self.inbox.menu_index, cx),
                _ => {}
            }
        } else {
            let index = self
                .inbox
                .rows
                .iter()
                .position(|r| Some(r.key()) == self.inbox.selected)
                .unwrap_or(0);
            match key {
                "up" => self.select_inbox_row(index.saturating_sub(1), cx),
                "down" => self
                    .select_inbox_row((index + 1).min(self.inbox.rows.len().saturating_sub(1)), cx),
                "enter" => self.open_inbox_mr(cx),
                _ => {}
            }
        }
        if matches!(key, "up" | "down") {
            if self.inbox.menu_open {
                let (scroll, _) = scrollbar::vertical("inbox-filter-menu-sb", cx);
                let header_rows = usize::from(self.inbox.menu == Some(Menu::Period));
                scroll.scroll_to_item(self.inbox.menu_index + header_rows);
            } else if let Some(index) = self
                .inbox
                .rows
                .iter()
                .position(|r| Some(r.key()) == self.inbox.selected)
            {
                let (scroll, _) = scrollbar::vertical("inbox-list-sb", cx);
                scroll.scroll_to_item(self.inbox.errors.len() + index);
            }
        }
        cx.notify();
    }

    fn choose_inbox_option(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.inbox.menu {
            Some(Menu::Repositories) => {
                if let Some(repo) = self.inbox.repositories.get(index) {
                    if !self.inbox.excluded.remove(&repo.path) {
                        self.inbox.excluded.insert(repo.path.clone());
                    }
                }
                self.refresh_inbox(cx);
            }
            Some(Menu::Period) => {
                if let Some(period) = Period::ALL.get(index) {
                    self.inbox.period = *period;
                }
                self.inbox.close_menu();
                self.refresh_inbox(cx);
            }
            None => {}
        }
        cx.notify();
    }
}

pub(super) fn render_filters(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let count = view
        .inbox
        .repositories
        .iter()
        .filter(|r| !view.inbox.excluded.contains(&r.path))
        .count();
    let label = if count == view.inbox.repositories.len() {
        format!("仓库 · 全部 {count} 个")
    } else {
        format!("仓库 · {count} 个")
    };
    div()
        .flex()
        .items_center()
        .gap(px(theme::CHROME_GAP))
        .map(super::super::titlebar::consume_control_mouse_events)
        .child(
            filter_pill(
                "inbox-repos-filter",
                label,
                view.inbox.repo_bounds.clone(),
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.focus.focus(window);
                this.inbox.toggle_menu(Menu::Repositories);
                cx.notify();
            })),
        )
        .child(
            filter_pill(
                "inbox-period-filter",
                view.inbox.period.label().into(),
                view.inbox.period_bounds.clone(),
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.focus.focus(window);
                this.inbox.toggle_menu(Menu::Period);
                cx.notify();
            })),
        )
}

fn filter_pill(
    id: &'static str,
    label: String,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    cx: &App,
) -> gpui::Stateful<Div> {
    let palette = theme::software_palette();
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .relative()
        .h(theme::TOGGLE_SIZE)
        .px_3()
        .rounded_full()
        .bg(palette.control.pill)
        .hover(|d| d.bg(palette.control.pill_hover))
        .active(|d| d.bg(palette.control.pressed))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .ui_text_size(12., cx)
        .text_color(palette.text.primary)
        .child(
            canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .child(label)
        .child(
            svg()
                .path("chevron_down.svg")
                .size(px(12.))
                .text_color(palette.text.secondary),
        )
}

pub(super) fn render_menu(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(menu) = view.inbox.menu else {
        return div().into_any_element();
    };
    let palette = theme::software_palette();
    let bounds = match menu {
        Menu::Repositories => view.inbox.repo_bounds.get(),
        Menu::Period => view.inbox.period_bounds.get(),
    };
    let (scroll, sb) = scrollbar::vertical("inbox-filter-menu-sb", cx);
    let options: Vec<(String, bool)> = match menu {
        Menu::Repositories => view
            .inbox
            .repositories
            .iter()
            .map(|r| (r.name.clone(), !view.inbox.excluded.contains(&r.path)))
            .collect(),
        Menu::Period => Period::ALL
            .iter()
            .map(|p| (p.label().into(), view.inbox.period == *p))
            .collect(),
    };
    let toolbar_height = if menu == Menu::Repositories { 36. } else { 0. };
    let heading_height = if menu == Menu::Period { 34. } else { 0. };
    let height = (options.len() as f32 * 38. + 8. + heading_height + toolbar_height).min(360.);
    anchored()
        .position(gpui::point(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height + px(4.),
        ))
        .anchor(Corner::TopLeft)
        .snap_to_window()
        .child(
            div()
                .id("inbox-filter-menu")
                .flex()
                .flex_col()
                .occlude()
                .w(px(260.))
                .h(px(height))
                .p_1()
                .rounded(px(theme::CHANGES_RADIUS))
                .bg(palette.surface.popover)
                .shadow(theme::picker_shadow())
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    // A trigger click owns its toggle; closing on its mouse-down
                    // would make the following click reopen the same menu.
                    if !this.inbox.repo_bounds.get().contains(&event.position)
                        && !this.inbox.period_bounds.get().contains(&event.position)
                    {
                        this.inbox.close_menu();
                        cx.notify();
                    }
                }))
                .when(menu == Menu::Repositories, |d| {
                    d.child(
                        div()
                            .h(px(36.))
                            .flex_none()
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(
                                [
                                    ("inbox-repos-all", "全选", true),
                                    ("inbox-repos-clear", "清空", false),
                                ]
                                .into_iter()
                                .map(|(id, label, select_all)| {
                                    div()
                                        .id(id)
                                        .debug_selector(move || id.to_owned())
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .ui_text_size(12., cx)
                                        .text_color(palette.text.primary)
                                        .bg(palette.feedback.neutral.background)
                                        .hover(|d| d.bg(palette.control.pill_hover))
                                        .active(|d| d.bg(palette.control.pressed))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if select_all {
                                                this.inbox.excluded.clear();
                                            } else {
                                                this.inbox.excluded = this
                                                    .inbox
                                                    .repositories
                                                    .iter()
                                                    .map(|r| r.path.clone())
                                                    .collect();
                                            }
                                            this.refresh_inbox(cx);
                                            cx.notify();
                                        }))
                                        .child(label)
                                }),
                            ),
                    )
                })
                .child(scrollbar::overlay_flex(
                    div()
                        .id("inbox-filter-options")
                        .size_full()
                        .track_scroll(&scroll)
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(menu == Menu::Period, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .px_2()
                                    .py_2()
                                    .ui_text_size(11., cx)
                                    .text_color(palette.text.secondary)
                                    .child("按更新时间 · 本地自然日"),
                            )
                        })
                        .children(options.into_iter().enumerate().map(
                            |(index, (label, checked))| {
                                div()
                                    .id(("inbox-filter-option", index))
                                    .h(px(34.))
                                    .w(px(252.))
                                    .flex_none()
                                    .px_3()
                                    .rounded_md()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .ui_text_size(12., cx)
                                    .text_color(palette.text.primary)
                                    .cursor_pointer()
                                    .when(index == view.inbox.menu_index, |d| {
                                        d.bg(palette.control.selected)
                                    })
                                    .hover(|d| {
                                        d.bg(if index == view.inbox.menu_index {
                                            palette.control.selected_hover
                                        } else {
                                            palette.control.hover
                                        })
                                    })
                                    .active(|d| {
                                        d.bg(if index == view.inbox.menu_index {
                                            palette.control.selected_pressed
                                        } else {
                                            palette.control.pressed
                                        })
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.inbox.menu_index = index;
                                        this.choose_inbox_option(index, cx);
                                    }))
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(checked, |d| {
                                                d.child(
                                                    div()
                                                        .size(px(5.))
                                                        .rounded_full()
                                                        .bg(palette.text.primary),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(label),
                                    )
                            },
                        )),
                    sb,
                )),
        )
        .into_any_element()
}

pub(super) fn render(
    view: &AppView,
    window: &Window,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let palette = theme::software_palette();
    let (scroll, sb) = scrollbar::vertical("inbox-list-sb", cx);
    let available = inbox_available_width(view, window);
    let detail_width = inbox_detail_width(view.inbox.detail_width, available);
    let row_width = (available - detail_width - 8.).max(0.);
    let mut children = Vec::new();
    for (index, row) in view.inbox.rows.iter().enumerate() {
        let selected = view.inbox.selected.as_ref() == Some(&row.key());
        children.push(
            div()
                .id(("inbox-mr", index))
                // Keep ellipsis and both rounded ends inside the island, including margins.
                .w(px(row_width))
                .tooltip(super::super::tooltip::Tooltip::text(
                    format!(
                        "{} · !{} · {}",
                        row.repository.name, row.mr.iid, row.mr.author.username
                    ),
                    None,
                ))
                .mx_1()
                .my_0p5()
                .px_3()
                .py_2()
                .rounded_lg()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap_0p5()
                .cursor_pointer()
                .when(selected, |d| d.bg(palette.metadata.range))
                .hover(|d| {
                    d.bg(if selected {
                        palette.metadata.range_hover
                    } else {
                        palette.metadata.row_hover
                    })
                })
                .active(|d| {
                    d.bg(if selected {
                        palette.metadata.range_pressed
                    } else {
                        palette.metadata.row_pressed
                    })
                })
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.focus.focus(window);
                    this.select_inbox_row(index, cx);
                    if event.click_count() >= 2 {
                        this.open_inbox_mr(cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w(px(0.))
                        .ui_text_size(11., cx)
                        .text_color(palette.metadata.label)
                        .child(status_badge(row.mr.status(), cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(format!(
                                    "{} · !{} · {}",
                                    row.repository.name, row.mr.iid, row.mr.author.username
                                )),
                        )
                        .child(
                            div().flex_none().child(
                                row.mr
                                    .updated_at
                                    .with_timezone(&Local)
                                    .format("%m/%d %H:%M")
                                    .to_string(),
                            ),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .ui_text_size(14., cx)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(palette.metadata.text)
                        .child(row.mr.title.clone()),
                )
                .into_any_element(),
        );
    }
    let body = div()
        .id("inbox-list-scroll")
        .size_full()
        .track_scroll(&scroll)
        .overflow_y_scroll()
        .when(view.inbox.loading, |d| {
            d.child(
                div()
                    .p_4()
                    .child(loading_row("inbox-loading", "Loading merge requests…", cx)),
            )
        })
        .children(
            view.inbox
                .errors
                .iter()
                .enumerate()
                .map(|(i, (repo, error))| {
                    div()
                        .px_4()
                        .py_2()
                        .child(
                            div()
                                .ui_text_size(12., cx)
                                .text_color(palette.text.primary)
                                .child(repo.clone()),
                        )
                        .child(render_error_note(("inbox-error", i), error, true, cx))
                }),
        )
        .when(!view.inbox.loading && view.inbox.rows.is_empty(), |d| {
            d.child(empty_message(
                if view.inbox.repositories.is_empty() {
                    "没有已加入的 GitLab 仓库。请添加与 Settings Base URL 匹配的仓库。"
                } else if view
                    .inbox
                    .repositories
                    .iter()
                    .all(|r| view.inbox.excluded.contains(&r.path))
                {
                    "尚未选择仓库。请从标题栏选择一个或多个仓库。"
                } else if !view.inbox.errors.is_empty() {
                    "未能加载 MR。请检查上方错误后重试。"
                } else {
                    "这段时间没有 MR。试试扩大时间范围。"
                },
                cx,
            ))
        })
        .children(children);
    div()
        .absolute()
        .inset_0()
        .flex()
        .pl(px(theme::left_island_inset(!view.repos_collapsed)))
        .pr(px(theme::CHANGES_INSET))
        .pt(px(theme::CHANGES_TOP_INSET))
        .pb(px(theme::CHANGES_INSET))
        .child(
            island("inbox-list")
                .flex_1()
                .child(
                    div()
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .ui_text_size(14., cx)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette.text.primary)
                                .child(format!("Merge requests · {}", view.inbox.rows.len())),
                        )
                        .child(
                            IconButton::new(
                                "inbox-refresh",
                                "refresh.svg",
                                "Refresh merge requests",
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inbox.loaded = false;
                                this.show_inbox(cx);
                            })),
                        ),
                )
                .child(
                    div()
                        .px_4()
                        .pb_2()
                        .ui_text_size(11., cx)
                        .text_color(palette.text.secondary)
                        .child("按更新时间分组 · 最近更新在前"),
                )
                .child(scrollbar::overlay_flex(body, sb))
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .ui_text_size(11., cx)
                        .text_color(palette.text.secondary)
                        .child("单击预览 · 双击或 Enter 进入 MR"),
                ),
        )
        .child(splitter::handle(
            "inbox-detail-resize",
            Axis::HorizontalTrailing,
            view.inbox_resize_handler(cx),
            view.inbox.resize_state.clone(),
            true,
        ))
        .child(
            div()
                .w(px(inbox_detail_width(
                    view.inbox.detail_width,
                    inbox_available_width(view, window),
                )))
                .flex_none()
                .h_full()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(render_detail(view, cx)),
        )
}

fn island(id: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .h_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(px(theme::CHANGES_RADIUS))
        .bg(theme::software_palette().surface.island)
}

fn empty_message(message: &'static str, cx: &App) -> impl IntoElement {
    div()
        .p_4()
        .ui_text_size(13., cx)
        .text_color(theme::software_palette().text.secondary)
        .child(message)
}

fn status_badge(status: &str, cx: &App) -> impl IntoElement {
    let feedback = theme::software_palette().feedback;
    let colors = match status {
        "opened" | "success" => feedback.success,
        "closed" | "failed" => feedback.error,
        "merged" => feedback.info,
        "running" | "pending" => feedback.warning,
        _ => feedback.neutral,
    };
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .ui_text_size(11., cx)
        .bg(colors.background)
        .text_color(colors.foreground)
        .child(status.to_owned())
}

fn render_detail(view: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(row) = view.inbox.selected_row() else {
        return island("inbox-detail")
            .flex_1()
            .child(empty_message("选择一条 MR 查看详情", cx))
            .into_any_element();
    };
    let palette = theme::software_palette();
    let (scroll, sb) = scrollbar::vertical("inbox-detail-sb", cx);
    let preview = view.inbox.preview.as_ref().and_then(|p| p.as_ref().ok());
    let items = mr_detail::context_items(&row.mr.source_branch, &row.mr.target_branch);
    let checks = mr_detail::check_items(
        preview.and_then(|p| p.merge_status.as_deref()),
        preview.map(|p| &p.checks),
    );
    let notice = match &view.inbox.preview {
        None => {
            Some(loading_row("inbox-preview-loading", "Loading checks…", cx).into_any_element())
        }
        Some(Err(error)) => Some(render_error_note("inbox-preview-error", error, false, cx)),
        Some(Ok(_)) => None,
    };
    let content = div()
        .id("inbox-detail-scroll")
        .size_full()
        .px_3()
        .py_2()
        .track_scroll(&scroll)
        .overflow_y_scroll()
        .child(mr_detail::render(
            format!(
                "inbox-desc-{}-{}",
                row.repository.path.display(),
                row.mr.iid
            ),
            mr_detail::Content {
                title: &row.mr.title,
                description: row.mr.description.as_deref(),
                metadata: items,
                checks,
                notice,
            },
            cx,
        ));
    island("inbox-detail")
        .flex_1()
        .child(scrollbar::overlay_flex(content, sb))
        .child(
            div().p_3().flex().items_center().child(
                div()
                    .id("inbox-open-mr")
                    .px_3()
                    .py_2()
                    .rounded_full()
                    .cursor_pointer()
                    .bg(palette.control.accent)
                    .text_color(palette.control.on_accent)
                    .ui_text_size(12., cx)
                    .hover(|d| d.opacity(0.9))
                    .active(|d| d.opacity(0.8))
                    .on_click(cx.listener(|this, _, _, cx| this.open_inbox_mr(cx)))
                    .child("进入 MR 界面 →"),
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(repository: &str, iid: u64, updated: &str) -> Row {
        Row {
            repository: InboxRepository {
                path: PathBuf::from(repository),
                name: repository.into(),
            },
            project: format!("group/{repository}"),
            mr: serde_json::from_value(serde_json::json!({
                "iid":iid,"title":"An MR", "source_branch":"feature", "target_branch":"main",
                "state":"opened", "author":{"username":"reviewer"},"updated_at":updated
            }))
            .unwrap(),
        }
    }

    #[test]
    fn inbox_split_keeps_both_panes_visible_when_dragged_or_window_shrinks() {
        assert_eq!(inbox_detail_width(None, 1000.), 500.);
        assert_eq!(inbox_detail_width(Some(-200.), 1000.), 240.);
        assert_eq!(inbox_detail_width(Some(2000.), 1000.), 760.);
        assert_eq!(inbox_detail_width(Some(760.), 600.), 360.);
        assert_eq!(inbox_detail_width(Some(760.), 400.), 200.);
    }

    #[test]
    fn aggregate_keeps_same_iid_in_different_repositories_and_partial_errors() {
        let a = row("a", 1, "2026-10-06T01:00:00Z");
        let b = row("b", 1, "2026-10-06T02:00:00Z");
        let selected = a.key();
        let mut inbox = Inbox {
            selected: Some(selected.clone()),
            generation: 2,
            ..Default::default()
        };
        assert!(inbox.finish_load(
            2,
            vec![
                ("a".into(), Ok(vec![a])),
                ("b".into(), Ok(vec![b])),
                ("c".into(), Err(ErrorNote::plain("Unavailable"))),
            ]
        ));
        assert_eq!(inbox.rows.len(), 2);
        assert_eq!(inbox.rows[0].repository.name, "b");
        assert_eq!(inbox.selected, Some(selected));
        assert_eq!(inbox.selected_row().unwrap().repository.name, "a");
        assert_eq!(inbox.errors[0].0, "c");
    }

    #[test]
    fn old_list_and_preview_cannot_replace_new_filter_or_selection() {
        let mut inbox = Inbox {
            generation: 8,
            preview_generation: 12,
            loading: true,
            ..Default::default()
        };
        assert!(!inbox.finish_load(
            7,
            vec![("a".into(), Ok(vec![row("a", 1, "2026-10-06T01:00:00Z")]))]
        ));
        assert!(inbox.loading);
        assert!(inbox.rows.is_empty());
        assert!(!inbox.finish_preview(11, Err(ErrorNote::plain("old failure"))));
        assert!(inbox.preview.is_none());
        assert!(inbox.finish_load(
            8,
            vec![("b".into(), Ok(vec![row("b", 2, "2026-10-06T02:00:00Z")]))]
        ));
        assert_eq!(inbox.selected, Some((PathBuf::from("b"), 2)));
        assert!(inbox.finish_preview(12, Err(ErrorNote::plain("current failure"))));
        assert!(matches!(inbox.preview, Some(Err(_))));
    }
}

#[cfg(test)]
mod menu_layout_tests {
    use super::*;

    fn fixture(cx: &mut Context<AppView>) -> AppView {
        AppView {
            focus: cx.focus_handle(),
            repos_collapsed: false,
            state: MainState::Empty,
            inbox: mr_inbox::Inbox::default(),
            mr_activation_generation: 0,
            store: WorkspaceStore::default(),
            diff_window: None,
            branch_picker: None,
            mr_picker: None,
            branch_toggle_bounds: Rc::new(Cell::new(Bounds::default())),
            mr_toggle_bounds: Rc::new(Cell::new(Bounds::default())),
            mr_entry: None,
            uncommitted_generation: 0,
            uncommitted_paths_pending: false,
            uncommitted_scan_error: None,
            empty_mr: false,
            pending_kind_restore: false,
            repo_menu: None,
            commit_menu: None,
            activation_sub: None,
            bounds_sub: None,
            collapsed_dirs: HashSet::new(),
            tree_path_fingerprint: Vec::new(),
            sidebar_width: splitter::default_sidebar_width(),
            sidebar_resize_state: Rc::new(ResizeState::with_drag_latch()),
            files_width: splitter::default_files_width(),
            files_resize_state: Rc::new(ResizeState::default()),
            head_meta_height: splitter::MIN_HEAD_META_HEIGHT,
            head_meta_height_user_set: false,
            head_meta_resize_state: Rc::new(ResizeState::default()),
            mr_detail_height: splitter::DEFAULT_MR_DETAIL_HEIGHT,
            mr_detail_height_user_set: false,
            mr_detail_resize_state: Rc::new(ResizeState::default()),
            #[cfg(target_os = "macos")]
            window_vibrancy: None,
        }
    }

    struct StageHarness {
        view: gpui::Entity<AppView>,
    }
    impl Render for StageHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let left = self.view.read(cx).sidebar_width + theme::CHANGES_SHADOW_GAP;
            let stage = self
                .view
                .update(cx, |view, cx| render(view, window, cx).into_any_element());
            div().size_full().child(
                div()
                    .absolute()
                    .left(px(left))
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .child(stage),
            )
        }
    }

    #[gpui::test]
    fn inbox_splitter_drag_resizes_only_inbox(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(appearance::resolve(&Default::default(), &[])));
        let view = cx.new(fixture);
        let (_, cx) = cx.add_window_view(|_, _| StageHarness { view: view.clone() });
        cx.simulate_resize(gpui::size(px(1000.), px(700.)));
        cx.run_until_parked();
        let before = cx.debug_bounds("inbox-detail").unwrap();
        let list = cx.debug_bounds("inbox-list").unwrap();
        assert!((f32::from(before.size.width - list.size.width)).abs() < 1.);
        let start = gpui::point(
            before.origin.x - px(theme::CHANGES_SHADOW_GAP / 2.),
            px(300.),
        );
        let end = gpui::point(start.x - px(50.), start.y);
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseMoveEvent {
            position: end,
            pressed_button: Some(MouseButton::Left),
            modifiers: Default::default(),
        });
        cx.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position: end,
            modifiers: Default::default(),
            click_count: 1,
        });
        cx.run_until_parked();
        let after = cx.debug_bounds("inbox-detail").unwrap();
        assert!(
            (f32::from(after.size.width - before.size.width) - 50.).abs() < 1.,
            "drag must grow detail by 50px: {before:?} → {after:?}"
        );
        assert_eq!(
            cx.read(|cx| view.read(cx).files_width),
            splitter::default_files_width()
        );
    }

    struct MenuHarness {
        view: gpui::Entity<AppView>,
    }
    impl Render for MenuHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let filters = self
                .view
                .update(cx, |view, cx| render_filters(view, cx).into_any_element());
            div()
                .size_full()
                .child(div().absolute().left(px(40.)).top(px(24.)).child(filters))
                .child(self.view.update(cx, |view, cx| {
                    deferred(render_menu(view, cx)).into_any_element()
                }))
        }
    }

    #[gpui::test]
    fn inbox_filter_menus_have_visible_options(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(appearance::resolve(&Default::default(), &[])));
        let view = cx.new(fixture);
        let (_, cx) = cx.add_window_view(|_, _| MenuHarness { view: view.clone() });
        cx.run_until_parked();
        for menu in [Menu::Period, Menu::Repositories] {
            view.update(cx, |view, cx| {
                view.inbox.menu = Some(menu);
                view.inbox.repositories = (0..3)
                    .map(|i| InboxRepository {
                        path: PathBuf::from(format!("/fixture/repo-{i}")),
                        name: format!("Fixture repository {i}"),
                    })
                    .collect();
                cx.notify();
            });
            cx.run_until_parked();
            let (scroll, _) = cx.update(|_, cx| scrollbar::vertical("inbox-filter-menu-sb", cx));
            assert!(
                scroll.bounds().size.height >= px(100.),
                "dropdown options are clipped: scroll viewport is {:?}",
                scroll.bounds()
            );
            let trigger = cx
                .debug_bounds(if menu == Menu::Period {
                    "inbox-period-filter"
                } else {
                    "inbox-repos-filter"
                })
                .expect("trigger rendered");
            assert_eq!(
                scroll.bounds().origin.x - px(4.),
                trigger.origin.x,
                "dropdown outer edge must align with capsule outer edge"
            );
            let first_index = usize::from(menu == Menu::Period);
            let option = scroll
                .bounds_for_item(first_index)
                .expect("first option must be laid out");
            assert!(option.size.height >= px(30.));
            let second = scroll
                .bounds_for_item(first_index + 1)
                .expect("second option must be laid out");
            assert!(
                second.origin.y - option.bottom() >= px(4.),
                "adjacent selected and hovered rows need visible spacing"
            );
            if menu == Menu::Repositories {
                let all = cx
                    .debug_bounds("inbox-repos-all")
                    .expect("select-all button rendered");
                let clear = cx
                    .debug_bounds("inbox-repos-clear")
                    .expect("clear button rendered");
                assert_eq!(
                    all.origin.y, clear.origin.y,
                    "bulk buttons share the top row"
                );
                assert!(all.right() < clear.origin.x);
                assert!(
                    all.bottom() <= scroll.bounds().origin.y,
                    "bulk actions stay above the scrolling repository options"
                );
                assert_eq!(all.origin.x, scroll.bounds().origin.x + px(8.));
            }
            assert!(
                scroll.bounds().intersects(&option),
                "first option must be visible"
            );
            let count = if menu == Menu::Period { 5 } else { 3 };
            let last = scroll
                .bounds_for_item(first_index + count - 1)
                .expect("last option must be laid out");
            assert!(
                scroll.bounds().intersects(&last),
                "last option must be visible"
            );
            assert!(
                scroll.bounds_for_item(first_index + count).is_none(),
                "menu must render its own option set after switching"
            );
        }
    }
}

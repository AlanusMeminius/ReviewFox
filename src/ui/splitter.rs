//! Dual-axis pane resize: BeadsViewer interaction shell, Zed row-height semantics.
//! See `docs/adr/0004-dual-axis-splitter.md`.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Size, Window,
    canvas, div, prelude::*, px,
};

use super::theme;

pub type ResizeHandler = Rc<dyn Fn(f32, &mut Window, &mut App)>;

#[derive(Default)]
pub struct ResizeState {
    active: Cell<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Vertical rule; size = pointer x (leading column width).
    HorizontalLeading,
    /// Vertical rule; size = viewport.width − x (trailing column width).
    HorizontalTrailing,
    /// Horizontal rule; size = viewport.height − y (south pane height).
    Vertical,
    /// Horizontal rule; size = pointer y (north pane height from viewport top).
    VerticalNorth,
}

pub const MIN_SIDEBAR_WIDTH: f32 = 140.;
pub const MAX_SIDEBAR_WIDTH: f32 = 360.;
pub const MIN_COMMITS_WIDTH: f32 = 280.;
pub const MIN_FILES_WIDTH: f32 = 200.;
pub const MAX_FILES_WIDTH: f32 = 480.;
/// Hit-target thickness for resize handles (ADR 0004).
pub const HANDLE_WIDTH: f32 = 5.;
/// Floor for Diff dual-pane (L|gutter|R) when clamping the file-tree column.
pub const MIN_DIFF_CONTENT_WIDTH: f32 = 400.;

pub const DEFAULT_HEAD_META_HEIGHT: f32 = 120.;
pub const MIN_HEAD_META_HEIGHT: f32 = 72.;
pub const MIN_FILE_TREE_HEIGHT: f32 = 100.;

pub const DEFAULT_MR_DETAIL_HEIGHT: f32 = 120.;
pub const MIN_MR_DETAIL_HEIGHT: f32 = 72.;
pub const MIN_COMMIT_LIST_HEIGHT: f32 = 100.;

pub fn default_sidebar_width() -> f32 {
    f32::from(theme::SIDEBAR_WIDTH)
}

pub fn default_files_width() -> f32 {
    f32::from(theme::FILES_WIDTH)
}

/// Changes floats over commits — sidebar only needs to leave a readable commits strip.
pub fn clamp_sidebar_width(requested: f32, available: f32) -> f32 {
    let maximum = MAX_SIDEBAR_WIDTH
        .min((available - MIN_COMMITS_WIDTH).max(MIN_SIDEBAR_WIDTH));
    requested.clamp(MIN_SIDEBAR_WIDTH, maximum)
}

/// Floating Changes width: leave `MIN_COMMITS_WIDTH` readable left of the capsule
/// (+ inset + shadow clearance).
pub fn clamp_files_width(requested: f32, available: f32, sidebar_width: f32) -> f32 {
    let stage = (available - sidebar_width).max(0.);
    let clear = theme::CHANGES_INSET + theme::CHANGES_SHADOW_GAP;
    let maximum = MAX_FILES_WIDTH
        .min((stage - MIN_COMMITS_WIDTH - clear).max(MIN_FILES_WIDTH));
    requested.clamp(MIN_FILES_WIDTH, maximum)
}

/// Diff window tree|dual only — same sidebar min/max, no main commits reservation.
pub fn clamp_diff_tree_width(requested: f32, available: f32) -> f32 {
    let maximum = MAX_SIDEBAR_WIDTH
        .min((available - MIN_DIFF_CONTENT_WIDTH).max(MIN_SIDEBAR_WIDTH));
    requested.clamp(MIN_SIDEBAR_WIDTH, maximum)
}

pub fn clamp_height(requested: f32, available: f32) -> f32 {
    let max_by_pct = available * 0.4;
    let max_by_tree = (available - MIN_FILE_TREE_HEIGHT).max(MIN_HEAD_META_HEIGHT);
    let maximum = max_by_pct.min(max_by_tree);
    requested.clamp(MIN_HEAD_META_HEIGHT, maximum)
}

/// MR detail (north of commit list): min floor, 40% cap, leave room for commits.
pub fn clamp_mr_detail_height(requested: f32, available: f32) -> f32 {
    let max_by_pct = available * 0.4;
    let max_by_list = (available - MIN_COMMIT_LIST_HEIGHT).max(MIN_MR_DETAIL_HEIGHT);
    let maximum = max_by_pct.min(max_by_list);
    requested.clamp(MIN_MR_DETAIL_HEIGHT, maximum)
}

/// Map pointer → raw pane size in window space (handlers re-clamp with sibling widths).
/// Vertical is clamped here (no sibling). Not `window.bounds()` — that is screen-global.
pub fn size_at_pointer(axis: Axis, position: Point<Pixels>, viewport: Size<Pixels>) -> f32 {
    match axis {
        Axis::HorizontalLeading => f32::from(position.x),
        Axis::HorizontalTrailing => f32::from(viewport.width - position.x),
        Axis::Vertical => clamp_height(
            f32::from(viewport.height - position.y),
            f32::from(viewport.height),
        ),
        Axis::VerticalNorth => f32::from(position.y),
    }
}

pub fn handle(
    id: &'static str,
    axis: Axis,
    on_resize: ResizeHandler,
    resize_state: Rc<ResizeState>,
) -> impl IntoElement {
    let down_state = resize_state.clone();
    let move_state = resize_state.clone();
    let up_state = resize_state;
    let mut el = div()
        .id(id)
        .flex_none()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let down_state = down_state.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, _, _, _| {
                        if event.button == MouseButton::Left && bounds.contains(&event.position) {
                            down_state.active.set(true);
                        }
                    });

                    let move_state = move_state.clone();
                    let on_resize = on_resize.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, _, window, cx| {
                        if !move_state.active.get() {
                            return;
                        }
                        let size = size_at_pointer(axis, event.position, window.viewport_size());
                        on_resize(size, window, cx);
                    });

                    let up_state = up_state.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, _, _, _| {
                        if event.button == MouseButton::Left {
                            up_state.active.set(false);
                        }
                    });
                },
            )
            .size_full(),
        );

    el = match axis {
        // Leading sits over column vibrancy on macOS — leave clear.
        Axis::HorizontalLeading => el
            .w(px(HANDLE_WIDTH))
            .h_full()
            .cursor_col_resize(),
        // Trailing is the Changes capsule's left-edge hit target — leave clear
        // so the soft cast shadow is not covered by an opaque strip.
        Axis::HorizontalTrailing => el
            .w(px(HANDLE_WIDTH))
            .h_full()
            .cursor_col_resize(),
        // Vertical sits between white panes inside the capsule; fill so a
        // Transparent window root doesn't punch through the 5px seam.
        Axis::Vertical => el
            .h(px(HANDLE_WIDTH))
            .w_full()
            .bg(theme::white())
            .cursor_row_resize(),
        // MR detail ↔ commits: leave clear so the capsule shadow is not
        // propped up by an opaque white strip.
        Axis::VerticalNorth => el
            .h(px(HANDLE_WIDTH))
            .w_full()
            .cursor_row_resize(),
    };
    el
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_clamp_keeps_commits_strip() {
        assert_eq!(clamp_sidebar_width(100., 1280.), 160.);
        assert_eq!(clamp_sidebar_width(220., 1280.), 220.);
        assert_eq!(clamp_sidebar_width(500., 1280.), 360.);
        // 720 − 280 commits = 440, but MAX_SIDEBAR caps at 360
        assert_eq!(clamp_sidebar_width(400., 720.), 360.);
        // tight window: 400 − 280 = 120 → floor at MIN_SIDEBAR
        assert_eq!(clamp_sidebar_width(220., 400.), 160.);
    }

    #[test]
    fn files_clamp_keeps_commits_strip_beside_float() {
        assert_eq!(clamp_files_width(100., 1280., 220.), 200.);
        assert_eq!(clamp_files_width(280., 1280., 220.), 280.);
        assert_eq!(clamp_files_width(600., 1280., 220.), 480.);
        // stage 500 − 280 − 8 − 8 = 204 → clamp stays 204 (above MIN_FILES)
        assert_eq!(clamp_files_width(400., 720., 220.), 204.);
    }

    #[test]
    fn diff_tree_clamp_keeps_dual_pane() {
        assert_eq!(clamp_diff_tree_width(100., 1280.), 160.);
        assert_eq!(clamp_diff_tree_width(200., 1280.), 200.);
        assert_eq!(clamp_diff_tree_width(500., 1280.), 360.);
        // 560 − 400 content = 160 max
        assert_eq!(clamp_diff_tree_width(300., 560.), 160.);
    }

    #[test]
    fn height_clamp_keeps_tree_and_pct_cap() {
        assert_eq!(clamp_height(40., 800.), 72.);
        assert_eq!(clamp_height(120., 800.), 120.);
        assert_eq!(clamp_height(400., 800.), 320.);
        assert_eq!(clamp_height(150., 200.), 80.);
    }

    #[test]
    fn trailing_size_is_distance_to_viewport_right() {
        use gpui::{point, px, size};
        let viewport = size(px(1280.), px(800.));
        let at_files_edge = point(px(1000.), px(400.));
        assert_eq!(
            size_at_pointer(Axis::HorizontalTrailing, at_files_edge, viewport),
            280.
        );
    }

    #[test]
    fn vertical_size_uses_viewport_not_screen_origin() {
        use gpui::{point, px, size};

        let viewport = size(px(1280.), px(800.));
        let at_handle = point(px(1000.), px(680.));
        assert_eq!(size_at_pointer(Axis::Vertical, at_handle, viewport), 120.);
        let screen_origin_y = 200.;
        let poisoned = (screen_origin_y + 800.) - 680.;
        assert!(poisoned > 120.);
        assert_ne!(clamp_height(poisoned, 800.), 120.);
    }

    #[test]
    fn vertical_north_size_is_distance_from_viewport_top() {
        use gpui::{point, px, size};

        let viewport = size(px(1280.), px(800.));
        let at_handle = point(px(400.), px(156.));
        assert_eq!(
            size_at_pointer(Axis::VerticalNorth, at_handle, viewport),
            156.
        );
    }

    #[test]
    fn mr_detail_height_clamp_keeps_commit_list() {
        assert_eq!(clamp_mr_detail_height(40., 800.), 72.);
        assert_eq!(clamp_mr_detail_height(120., 800.), 120.);
        assert_eq!(clamp_mr_detail_height(400., 800.), 320.);
        assert_eq!(clamp_mr_detail_height(150., 200.), 80.);
    }
}

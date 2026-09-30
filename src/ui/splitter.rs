//! Dual-axis pane resize: BeadsViewer interaction shell, Zed row-height semantics.
//! See `docs/adr/0004-dual-axis-splitter.md`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{
    App, Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Size,
    Window, canvas, div, fill, point, prelude::*, px, size,
};

use super::theme;

pub type ResizeHandler = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// How many splitter handles currently own a press-drag. Selectable text surfaces
/// consult this so a resize gesture does not paint a false selection.
static RESIZE_DRAG_COUNT: AtomicUsize = AtomicUsize::new(0);

/// True while any splitter handle is being dragged.
pub fn is_resizing() -> bool {
    RESIZE_DRAG_COUNT.load(Ordering::Relaxed) > 0
}

fn begin_resize_drag() {
    RESIZE_DRAG_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn end_resize_drag() {
    let _ = RESIZE_DRAG_COUNT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_sub(1))
    });
}

/// Present only on a handle whose caller may swallow the rest of a drag.
#[derive(Default)]
struct DragLatch {
    /// A nudge already opened the pane; moves do nothing until mouseup.
    consumed: Cell<bool>,
    /// `size_at_pointer` at mousedown.
    origin: Cell<Option<f32>>,
}

#[derive(Default)]
pub struct ResizeState {
    active: Cell<bool>,
    /// Pointer is over this handle's hit strip (chrome reads this for hover fill).
    hovered: Cell<bool>,
    latch: Option<Rc<DragLatch>>,
}

impl ResizeState {
    /// Opt in to [`Self::consume_drag`]: other splitters leave the latch off.
    pub fn with_drag_latch() -> Self {
        Self {
            latch: Some(Rc::new(DragLatch::default())),
            ..Self::default()
        }
    }

    /// Ignore further moves until mouseup.
    pub fn consume_drag(&self) {
        if let Some(latch) = &self.latch {
            latch.consumed.set(true);
        }
    }

    pub fn drag_consumed(&self) -> bool {
        self.latch
            .as_ref()
            .is_some_and(|latch| latch.consumed.get())
    }

    /// `size_at_pointer` at the mousedown that started this drag.
    pub fn drag_origin(&self) -> Option<f32> {
        self.latch.as_ref().and_then(|latch| latch.origin.get())
    }

    fn note_drag_start(&self, origin: f32) {
        if let Some(latch) = &self.latch {
            latch.consumed.set(false);
            latch.origin.set(Some(origin));
        }
    }

    fn note_drag_end(&self) {
        if let Some(latch) = &self.latch {
            latch.consumed.set(false);
            latch.origin.set(None);
        }
    }
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
/// Hit-target thickness for most resize handles (ADR 0004).
pub const HANDLE_WIDTH: f32 = 5.;
/// Short stadium painted at the seam center when chrome is on (paint-only).
pub const CAPSULE_THICKNESS: f32 = 3.;
pub const CAPSULE_LENGTH: f32 = 24.;
/// Stadium corner radius = half the short side.
pub const CAPSULE_RADIUS: f32 = CAPSULE_THICKNESS / 2.;

const CAPSULE_IDLE_ALPHA: f32 = 0.35;
const CAPSULE_HOVER_ALPHA: f32 = 0.55;
const CAPSULE_DRAG_ALPHA: f32 = 0.72;

/// Floor for Diff dual-pane (L|gutter|R) when clamping the file-tree column.
pub const MIN_DIFF_CONTENT_WIDTH: f32 = 400.;

pub const MIN_HEAD_META_HEIGHT: f32 = 72.;
pub const MIN_FILE_TREE_HEIGHT: f32 = 100.;

pub const DEFAULT_MR_DETAIL_HEIGHT: f32 = 120.;
pub const MIN_MR_DETAIL_HEIGHT: f32 = 72.;
/// Floor left for the commit island below the MR detail splitter.
pub const MIN_COMMIT_LIST_HEIGHT: f32 = 100.;

pub fn default_sidebar_width() -> f32 {
    f32::from(theme::SIDEBAR_WIDTH)
}

pub fn default_files_width() -> f32 {
    f32::from(theme::FILES_WIDTH)
}

/// Initial MR detail height: half the island column (clamped).
pub fn default_mr_detail_height(available: f32) -> f32 {
    clamp_mr_detail_height(available * 0.5, available)
}

/// Initial head-meta height: 40% of the Changes island (clamped).
pub fn default_head_meta_height(available: f32) -> f32 {
    clamp_height(available * 0.4, available)
}

/// Changes floats over commits — sidebar only needs to leave a readable commits strip.
pub fn clamp_sidebar_width(requested: f32, available: f32) -> f32 {
    let maximum = MAX_SIDEBAR_WIDTH.min((available - MIN_COMMITS_WIDTH).max(MIN_SIDEBAR_WIDTH));
    requested.clamp(MIN_SIDEBAR_WIDTH, maximum)
}

/// Floating Changes width: leave `MIN_COMMITS_WIDTH` for the Commit capsule,
/// plus the right inset and the Commit|Changes gap.
///
/// `sidebar_width == 0` means the rail is collapsed: the left inset is inside
/// the stage. When the rail is open, that inset is the seam outside the stage
/// (`CHANGES_SHADOW_GAP`), so it is subtracted from `available` instead.
pub fn clamp_files_width(requested: f32, available: f32, sidebar_width: f32) -> f32 {
    let rail_open = sidebar_width > 0.;
    let rail = if rail_open {
        theme::CHANGES_SHADOW_GAP
    } else {
        0.
    };
    let stage = (available - sidebar_width - rail).max(0.);
    let left_inset = if rail_open { 0. } else { theme::CHANGES_INSET };
    let clear = left_inset + theme::CHANGES_INSET + theme::CHANGES_SHADOW_GAP;
    let maximum = MAX_FILES_WIDTH.min((stage - MIN_COMMITS_WIDTH - clear).max(MIN_FILES_WIDTH));
    requested.clamp(MIN_FILES_WIDTH, maximum)
}

/// Diff window tree|dual only — same sidebar min/max, no main commits reservation.
pub fn clamp_diff_tree_width(requested: f32, available: f32) -> f32 {
    let maximum =
        MAX_SIDEBAR_WIDTH.min((available - MIN_DIFF_CONTENT_WIDTH).max(MIN_SIDEBAR_WIDTH));
    requested.clamp(MIN_SIDEBAR_WIDTH, maximum)
}

pub fn clamp_height(requested: f32, available: f32) -> f32 {
    let max_by_pct = available * 0.4;
    let max_by_tree = (available - MIN_FILE_TREE_HEIGHT).max(MIN_HEAD_META_HEIGHT);
    let maximum = max_by_pct.min(max_by_tree).max(MIN_HEAD_META_HEIGHT);
    requested.clamp(MIN_HEAD_META_HEIGHT, maximum)
}

/// MR detail (north of commit list): min floor, leave [`MIN_COMMIT_LIST_HEIGHT`] for commits.
/// No percentage cap — a first open defaults to half via [`default_mr_detail_height`].
pub fn clamp_mr_detail_height(requested: f32, available: f32) -> f32 {
    let maximum = (available - MIN_COMMIT_LIST_HEIGHT).max(MIN_MR_DETAIL_HEIGHT);
    requested.clamp(MIN_MR_DETAIL_HEIGHT, maximum)
}

/// A collapsible pane snaps shut below this width; there is no state between 0 and it.
pub const COLLAPSE_THRESHOLD: f32 = 160.;
/// A collapsed leading rail opens once the pointer has moved this far toward open.
pub const REVEAL_NUDGE: f32 = 8.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collapse {
    Hidden,
    Width(f32),
}

/// Collapsible pane beside a sibling that keeps `floor` of `available`. No max:
/// the pane takes what it asks for, shrinks when the sibling would drop below
/// its floor, and hides once it would be under `threshold`.
pub fn resolve_collapsible(requested: f32, available: f32, floor: f32, threshold: f32) -> Collapse {
    let width = requested.min(available - floor);
    if width < threshold {
        Collapse::Hidden
    } else {
        Collapse::Width(width)
    }
}

/// One move of a leading rail that can snap shut and nudge back open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RailDrag {
    Stay,
    Reveal,
    Show(f32),
    Hide,
}

/// Open rail: follow the pointer, cap at [`MAX_SIDEBAR_WIDTH`], hide below
/// [`COLLAPSE_THRESHOLD`]. Collapsed rail: stay put until the pointer has moved
/// [`REVEAL_NUDGE`] toward open, then reveal at the remembered width.
pub fn leading_rail_drag(
    raw: f32,
    origin: f32,
    shown: bool,
    available: f32,
    floor: f32,
) -> RailDrag {
    if !shown {
        if raw - origin < REVEAL_NUDGE {
            RailDrag::Stay
        } else {
            RailDrag::Reveal
        }
    } else {
        match resolve_collapsible(raw, available, floor, COLLAPSE_THRESHOLD) {
            Collapse::Hidden => RailDrag::Hide,
            Collapse::Width(width) => RailDrag::Show(width.min(MAX_SIDEBAR_WIDTH)),
        }
    }
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

fn capsule_size(axis: Axis) -> (f32, f32) {
    match axis {
        Axis::HorizontalLeading | Axis::HorizontalTrailing => (CAPSULE_THICKNESS, CAPSULE_LENGTH),
        Axis::Vertical | Axis::VerticalNorth => (CAPSULE_LENGTH, CAPSULE_THICKNESS),
    }
}

fn capsule_bounds(axis: Axis, seam: Bounds<Pixels>) -> Bounds<Pixels> {
    let (cw, ch) = capsule_size(axis);
    let origin = point(
        seam.origin.x + (seam.size.width - px(cw)) / 2.,
        seam.origin.y + (seam.size.height - px(ch)) / 2.,
    );
    Bounds {
        origin,
        size: size(px(cw), px(ch)),
    }
}

fn capsule_alpha(state: &ResizeState) -> f32 {
    if state.active.get() {
        CAPSULE_DRAG_ALPHA
    } else if state.hovered.get() {
        CAPSULE_HOVER_ALPHA
    } else {
        CAPSULE_IDLE_ALPHA
    }
}

/// Dual-axis resize hit strip. `chrome` paints a short center stadium; default off
/// keeps the strip invisible for any caller that does not opt in.
pub fn handle(
    id: &'static str,
    axis: Axis,
    on_resize: ResizeHandler,
    resize_state: Rc<ResizeState>,
    chrome: bool,
) -> impl IntoElement {
    let down_state = resize_state.clone();
    let move_state = resize_state.clone();
    let hover_state = resize_state.clone();
    let up_state = resize_state.clone();
    let paint_state = resize_state;
    let hit = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if chrome {
                window.paint_quad(
                    fill(
                        capsule_bounds(axis, bounds),
                        theme::splitter_capsule(capsule_alpha(&paint_state)),
                    )
                    .corner_radii(px(CAPSULE_RADIUS)),
                );
            }

            let down_state = down_state.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, _, window, _| {
                if event.button == MouseButton::Left && bounds.contains(&event.position) {
                    down_state.note_drag_start(size_at_pointer(
                        axis,
                        event.position,
                        window.viewport_size(),
                    ));
                    // `replace` returns the previous value — count only transitions.
                    if !down_state.active.replace(true) {
                        begin_resize_drag();
                        window.refresh();
                    }
                }
            });

            let move_state = move_state.clone();
            let hover_state = hover_state.clone();
            let on_resize = on_resize.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, _, window, cx| {
                let hovering = bounds.contains(&event.position);
                if hover_state.hovered.get() != hovering {
                    hover_state.hovered.set(hovering);
                    window.refresh();
                }
                if !move_state.active.get() {
                    return;
                }
                let size = size_at_pointer(axis, event.position, window.viewport_size());
                on_resize(size, window, cx);
            });

            let up_state = up_state.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, _, window, _| {
                if event.button == MouseButton::Left && up_state.active.replace(false) {
                    up_state.note_drag_end();
                    end_resize_drag();
                    window.refresh();
                }
            });
        },
    )
    .size_full();

    let mut el = div().id(id).flex_none().relative().child(hit);

    el = match axis {
        // Leading chrome fills the frost gap between the rail and the island,
        // same width as Commit|Changes. Without chrome, keep a slim strip.
        Axis::HorizontalLeading => {
            let w = if chrome {
                theme::CHANGES_SHADOW_GAP
            } else {
                HANDLE_WIDTH
            };
            el.w(px(w)).h_full().cursor_col_resize()
        }
        // Trailing chrome fills the frost gap between Commit and Changes islands.
        // Without chrome, keep a slim clear strip (soft cast must stay uncovered).
        Axis::HorizontalTrailing => {
            let w = if chrome {
                theme::CHANGES_SHADOW_GAP
            } else {
                HANDLE_WIDTH
            };
            el.w(px(w)).h_full().cursor_col_resize()
        }
        // Without chrome, fill white so a Transparent root doesn't punch through.
        // With chrome, the strip stays clear — only the center stadium paints.
        Axis::Vertical => {
            let el = el.h(px(HANDLE_WIDTH)).w_full().cursor_row_resize();
            if chrome { el } else { el.bg(theme::white()) }
        }
        // MR detail ↔ Commit island: clear hit strip doubles as the frost gap.
        Axis::VerticalNorth => el.h(px(theme::CHANGES_INSET)).w_full().cursor_row_resize(),
    };
    el
}

/// A collapsed trailing pane's handle, parked in the parent's right
/// [`theme::CHANGES_INSET`] so the pane can be dragged back out. The parent must be
/// `relative` and inset by exactly that much; the capsule centres on its height.
pub fn parked_handle(
    id: &'static str,
    on_resize: ResizeHandler,
    resize_state: Rc<ResizeState>,
) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(-theme::CHANGES_INSET))
        .w(px(theme::CHANGES_INSET))
        .child(handle(
            id,
            Axis::HorizontalTrailing,
            on_resize,
            resize_state,
            true,
        ))
}

/// A collapsed leading rail's handle, parked in the parent's left gutter so the
/// rail can be dragged back open. The parent must be `relative`.
pub fn parked_leading_handle(
    id: &'static str,
    on_resize: ResizeHandler,
    resize_state: Rc<ResizeState>,
) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(px(theme::CHANGES_SHADOW_GAP))
        .child(handle(
            id,
            Axis::HorizontalLeading,
            on_resize,
            resize_state,
            true,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_clamp_keeps_commits_strip() {
        assert_eq!(clamp_sidebar_width(100., 1280.), 140.);
        assert_eq!(clamp_sidebar_width(220., 1280.), 220.);
        assert_eq!(clamp_sidebar_width(500., 1280.), 360.);
        // 720 − 280 commits = 440, but MAX_SIDEBAR caps at 360
        assert_eq!(clamp_sidebar_width(400., 720.), 360.);
        // tight window: 400 − 280 = 120 → floor at MIN_SIDEBAR
        assert_eq!(clamp_sidebar_width(220., 400.), 140.);
    }

    #[test]
    fn files_clamp_keeps_commits_strip_beside_float() {
        assert_eq!(clamp_files_width(100., 1280., 220.), 200.);
        assert_eq!(clamp_files_width(280., 1280., 220.), 280.);
        assert_eq!(clamp_files_width(600., 1280., 220.), 480.);
        // stage (720−220−12 rail) − 280 − right − gap = 184 → floors at MIN_FILES
        assert_eq!(clamp_files_width(400., 720., 220.), 200.);
        // collapsed rail: left inset is back inside the stage
        assert_eq!(clamp_files_width(600., 1280., 0.), 480.);
        assert_eq!(clamp_files_width(400., 400., 0.), 200.);
    }

    #[test]
    fn diff_tree_clamp_keeps_dual_pane() {
        assert_eq!(clamp_diff_tree_width(100., 1280.), 140.);
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
        assert_eq!(clamp_mr_detail_height(400., 800.), 400.);
        assert_eq!(clamp_mr_detail_height(750., 800.), 700.);
        assert_eq!(clamp_mr_detail_height(150., 200.), 100.);
    }

    #[test]
    fn mr_detail_default_is_half_the_column() {
        assert_eq!(default_head_meta_height(800.), 320.);
        assert_eq!(default_head_meta_height(100.), 72.);
        assert_eq!(default_mr_detail_height(800.), 400.);
        assert_eq!(default_mr_detail_height(200.), 100.);
        assert_eq!(default_mr_detail_height(100.), 72.);
    }

    #[test]
    fn collapsible_follows_request_above_threshold() {
        assert_eq!(
            resolve_collapsible(300., 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Width(300.)
        );
        assert_eq!(
            resolve_collapsible(160., 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Width(160.)
        );
    }

    #[test]
    fn collapsible_hides_below_threshold() {
        assert_eq!(
            resolve_collapsible(159., 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Hidden
        );
        assert_eq!(
            resolve_collapsible(0., 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Hidden
        );
    }

    #[test]
    fn collapsible_width_stops_at_the_diff_floor() {
        // 1200 − 400 floor leaves at most 800; no other maximum.
        assert_eq!(
            resolve_collapsible(1000., 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Width(800.)
        );
        assert_eq!(
            resolve_collapsible(700., 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Width(700.)
        );
    }

    #[test]
    fn narrow_room_shrinks_collapsible_then_hides_it() {
        // 268 wanted, 400 + 200 room: shrinks to 200 first.
        assert_eq!(
            resolve_collapsible(268., 600., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Width(200.)
        );
        // 400 + 160 room: still at the threshold.
        assert_eq!(
            resolve_collapsible(268., 560., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Width(160.)
        );
        // Below 400 + 160: yields entirely.
        assert_eq!(
            resolve_collapsible(268., 559., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Hidden
        );
        assert_eq!(
            resolve_collapsible(268., 300., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD),
            Collapse::Hidden
        );
    }

    #[test]
    fn force_show_drops_the_floor() {
        // 300 room cannot fit 400 + 160; a zero floor keeps the last width.
        assert_eq!(
            resolve_collapsible(268., 300., 0., COLLAPSE_THRESHOLD),
            Collapse::Width(268.)
        );
        assert_eq!(
            resolve_collapsible(268., 200., 0., COLLAPSE_THRESHOLD),
            Collapse::Width(200.)
        );
        // A forced show drops the snap threshold too: a 100px stage still shows,
        // clamped to the room, instead of staying hidden.
        assert_eq!(
            resolve_collapsible(268., 100., 0., 0.),
            Collapse::Width(100.)
        );
    }

    #[test]
    fn collapsible_stays_hidden_until_threshold() {
        let at = |pointer| {
            resolve_collapsible(pointer, 1200., MIN_DIFF_CONTENT_WIDTH, COLLAPSE_THRESHOLD)
        };
        assert_eq!(at(40.), Collapse::Hidden);
        assert_eq!(at(159.), Collapse::Hidden);
        assert_eq!(at(161.), Collapse::Width(161.));
        assert_eq!(at(240.), Collapse::Width(240.));
    }

    #[test]
    fn capsule_chrome_tokens_are_stadium_shaped() {
        assert_eq!(CAPSULE_RADIUS, CAPSULE_THICKNESS / 2.);
        assert!(theme::CHANGES_SHADOW_GAP > HANDLE_WIDTH);
        assert!(theme::CHANGES_SHADOW_GAP >= CAPSULE_THICKNESS + 2.);
        assert_eq!(CAPSULE_THICKNESS, 3.);
        assert_eq!(CAPSULE_LENGTH, 24.);
    }

    #[test]
    fn leading_rail_drag_snaps_and_nudges() {
        let floor = MIN_COMMITS_WIDTH;
        assert_eq!(
            leading_rail_drag(160., 0., true, 1280., floor),
            RailDrag::Show(160.)
        );
        assert_eq!(
            leading_rail_drag(188., 0., true, 1280., floor),
            RailDrag::Show(188.)
        );
        assert_eq!(
            leading_rail_drag(159., 0., true, 1280., floor),
            RailDrag::Hide
        );
        assert_eq!(
            leading_rail_drag(500., 0., true, 1280., floor),
            RailDrag::Show(MAX_SIDEBAR_WIDTH)
        );
        // 400 − 280 commits = 120, under the snap threshold.
        assert_eq!(
            leading_rail_drag(200., 0., true, 400., floor),
            RailDrag::Hide
        );
        assert_eq!(
            leading_rail_drag(7., 0., false, 1280., floor),
            RailDrag::Stay
        );
        assert_eq!(
            leading_rail_drag(8., 0., false, 1280., floor),
            RailDrag::Reveal
        );
    }

    #[test]
    fn capsule_alpha_follows_resize_state() {
        let state = ResizeState::default();
        assert_eq!(capsule_alpha(&state), CAPSULE_IDLE_ALPHA);
        state.hovered.set(true);
        assert_eq!(capsule_alpha(&state), CAPSULE_HOVER_ALPHA);
        state.active.set(true);
        assert_eq!(capsule_alpha(&state), CAPSULE_DRAG_ALPHA);
    }
}

//! `DualPaneElement`: old pane | gutter | new pane as one GPUI Element. Prepaint
//! takes the bounds (so `view_h` is this frame's), builds the Viewport and a
//! paint list for the visible rows only; paint draws that list and registers
//! the mouse listeners, which go through Viewport hit tests. See
//! docs/diffview-architecture.md §3–§5 and docs/dual-pane-diff.md §3.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, Entity,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Rgba,
    ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, Window, fill, font, point, px,
    relative, rgb, size,
};

use super::layout::{Layout, LineKind, Row};
use super::pane::DualPane;
use super::trace::{self, FrameStats};
use super::viewport::{Viewport, route_wheel, snap};
use crate::domain::Side;
use crate::ui::scrollbar::{self, ThumbGeom};
use crate::ui::theme;

const LN_FONT_PX: f32 = 10.;
/// Wider than Menlo/Consolas at 10px (~6px) so a digit is never clipped.
const LN_DIGIT_PX: f32 = 8.;
/// Line-number inset from the code side of its column.
const LN_PAD: f32 = 4.;
const BRIDGE_COL: f32 = 24.;
/// Code text inset from the pane's inner edge.
const TEXT_PAD: f32 = 12.;
/// Left bar on a commented line; the text moves right by the same amount.
const COMMENT_BAR: f32 = 2.;
const SEAM_H: f32 = 2.;
const DRAFTING_BG: u32 = 0xdbe4ff;

/// State that changes without touching Layout or the shaped-line cache.
pub(super) struct Decorations {
    pub drafting: Option<(Side, u32)>,
}

/// Shaped text and line number per `(side, visual row)`. Cleared on Layout
/// rebuild and font size change; evicted outside visible ± one screen.
#[derive(Default)]
pub(super) struct ShapeCache {
    rows: HashMap<(Side, u32), RowShape>,
}

struct RowShape {
    text: Option<ShapedLine>,
    label: Option<ShapedLine>,
}

impl ShapeCache {
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    fn retain(&mut self, keep: &[Range<usize>; 2]) {
        self.rows.retain(|&(side, row), _| keep[side_ix(side)].contains(&(row as usize)));
    }
}

/// Scrollbar interaction state for both sides (auto-hide like `scrollbar.rs`).
#[derive(Default)]
pub(super) struct BarState {
    pub visible: bool,
    pub hovered: [bool; 2],
    /// Side being dragged and the grab offset inside the thumb.
    pub drag: Option<(Side, f32)>,
}

impl BarState {
    fn shown(&self, side: Side) -> bool {
        self.visible || self.hovered[side_ix(side)] || self.drag.is_some_and(|(s, _)| s == side)
    }
}

/// Horizontal split of the element, in window pixels.
#[derive(Clone, Copy)]
pub(super) struct Geom {
    pub bounds: Bounds<Pixels>,
    pub old: Bounds<Pixels>,
    pub gutter: Bounds<Pixels>,
    pub new: Bounds<Pixels>,
    pub ln_w: f32,
}

impl Geom {
    pub fn new(bounds: Bounds<Pixels>, ln_w: f32, scale: f32) -> Self {
        let x0 = snap(f32::from(bounds.left()), scale);
        let x1 = snap(f32::from(bounds.right()), scale);
        let y0 = snap(f32::from(bounds.top()), scale);
        let y1 = snap(f32::from(bounds.bottom()), scale);
        let gutter_w = ln_w * 2. + BRIDGE_COL;
        let pane_w = snap(((x1 - x0 - gutter_w) / 2.).max(0.), scale);
        let rect = |a: f32, b: f32| {
            Bounds::from_corners(point(px(a), px(y0)), point(px(b.max(a)), px(y1)))
        };
        let g0 = x0 + pane_w;
        let g1 = (g0 + gutter_w).min(x1);
        Self {
            bounds: rect(x0, x1),
            old: rect(x0, g0),
            gutter: rect(g0, g1),
            new: rect(g1, x1),
            ln_w,
        }
    }

    pub fn top(&self) -> f32 {
        f32::from(self.bounds.top())
    }

    pub fn pane(&self, side: Side) -> Bounds<Pixels> {
        match side {
            Side::Old => self.old,
            Side::New => self.new,
        }
    }

    /// Scrollbar track: old on the outer left, new on the outer right (ADR-0003).
    pub fn track(&self, side: Side) -> Bounds<Pixels> {
        let pane = self.pane(side);
        let h = (f32::from(pane.size.height) - scrollbar::PAD * 2.).max(0.);
        let x = match side {
            Side::Old => f32::from(pane.left()),
            Side::New => f32::from(pane.right()) - scrollbar::TRACK_WIDTH,
        };
        Bounds::new(
            point(px(x), pane.top() + px(scrollbar::PAD)),
            size(px(scrollbar::TRACK_WIDTH), px(h)),
        )
    }
}

/// Thumb geometry for a side whose top is `top` of `max_top` in a `view_h` pane.
pub(super) fn thumb_for(view_h: f32, max_top: f32, top: f32) -> Option<ThumbGeom> {
    ThumbGeom::from_metrics(px(view_h), px(max_top), px(-top))
}

/// Side top that puts the thumb's top edge at `thumb_top` in the track.
pub(super) fn top_at(geom: &ThumbGeom, thumb_top: f32) -> f32 {
    -f32::from(geom.offset_for(px(thumb_top)))
}

/// Width a code pane needs to show a line `line_w` wide without clipping:
/// inset, comment bar room, the text, and the same inset after it.
pub(super) fn text_extent(line_w: f32) -> f32 {
    TEXT_PAD + COMMENT_BAR + line_w + TEXT_PAD
}

pub(super) fn line_number_digits(layout: &Layout) -> u32 {
    let mut digits = 0u32;
    let mut n = layout.max_line_number().max(1);
    while n > 0 {
        digits += 1;
        n /= 10;
    }
    digits.max(2)
}

pub(super) fn ln_col_width(digits: u32) -> f32 {
    digits as f32 * LN_DIGIT_PX + LN_PAD
}

struct RowPaint {
    y0: f32,
    y1: f32,
    kind_bg: Rgba,
    bg: Rgba,
    commented: bool,
    text: Option<ShapedLine>,
    /// Word-mark runs, x relative to the text origin; one rect per run.
    marks: Vec<(f32, f32)>,
    label: Option<ShapedLine>,
}

struct SideFrame {
    rows: Vec<RowPaint>,
    /// Hatched intervals, window y.
    gaps: Vec<(f32, f32)>,
    /// Seam hairlines, window y; `true` when the seam row exists (the gutter
    /// column draws those too).
    seams: Vec<(f32, bool)>,
    empty_seam: Option<f32>,
    thumb: Option<Thumb>,
    /// Widest shaped text among this frame's rows.
    widest: f32,
    /// Horizontal scroll of the code text, device-pixel snapped. Set after
    /// the frame is built, once `widest` has fed the clamp.
    x_offset: f32,
}

struct Thumb {
    rect: Bounds<Pixels>,
    shown: bool,
    dragging: bool,
}

/// A visible bridge in window coordinates.
struct WinBridge {
    kind: LineKind,
    x_l: f32,
    x_r: f32,
    y_l0: f32,
    y_l1: f32,
    y_r0: f32,
    y_r1: f32,
}

/// Everything paint needs, built in prepaint.
pub struct Frame {
    geom: Geom,
    row_h: f32,
    hitbox: Hitbox,
    code: [Hitbox; 2],
    tracks: [Option<Hitbox>; 2],
    sides: [SideFrame; 2],
    bridges: Vec<WinBridge>,
    /// Omission separator joins: (old y, new y), window.
    waves: Vec<(f32, f32)>,
    /// Frame-trace numbers (zeros unless `REVIEWFOX_FRAME_TRACE=1`).
    pub(super) stats: FrameStats,
}

pub(super) struct FrameInput<'a> {
    pub layout: &'a Layout,
    pub vp: &'a Viewport<'a>,
    pub geom: Geom,
    pub row_h: f32,
    pub font_px: f32,
    pub scale: f32,
    pub decorations: Decorations,
    pub bars: &'a BarState,
}

/// Shape and place the visible rows of both sides; evict far cache entries.
pub(super) fn build_frame(
    input: FrameInput<'_>,
    shapes: &mut ShapeCache,
    hitboxes: (Hitbox, [Hitbox; 2], [Option<Hitbox>; 2]),
    window: &mut Window,
) -> Frame {
    let FrameInput {
        layout,
        vp,
        geom,
        row_h,
        font_px,
        scale,
        decorations,
        bars,
    } = input;
    let t_build = trace::start();
    let mut stats = FrameStats::default();
    let top = geom.top();
    let view_h = f32::from(geom.bounds.size.height);
    let screen = (view_h / row_h).ceil() as usize;
    let mut keep = [0..0, 0..0];
    let sides = [Side::Old, Side::New].map(|side| {
        let rows = layout.side(side);
        let visible = vp.visible_rows(side);
        stats.rows[side_ix(side)] = visible.len();
        keep[side_ix(side)] = visible.start.saturating_sub(screen)..visible.end + screen;
        let side_top = vp.top(side);
        let y_of = |r: usize| snap(top + r as f32 * row_h - side_top, scale);
        let drafting = decorations
            .drafting
            .and_then(|(s, ln)| (s == side).then_some(ln));
        let mut out = Vec::with_capacity(visible.len());
        let mut widest = 0f32;
        for i in visible {
            let Some(row) = rows.row(i) else { continue };
            let shape = shapes
                .rows
                .entry((side, i as u32))
                .or_insert_with(|| {
                    let t = trace::start();
                    let shape = shape_row(layout, side, row, font_px, window);
                    stats.shaped += 1;
                    stats.shape += trace::since(t);
                    shape
                });
            let (kind, commented, drafting_here, marks) = match row {
                Row::Line(l) => {
                    let marks = match (layout.mark_runs(side, l), &shape.text) {
                        (Some(runs), Some(text)) => mark_spans(&runs, text),
                        _ => Vec::new(),
                    };
                    (
                        Some(l.kind),
                        rows.has_comment(l.ln),
                        drafting == Some(l.ln),
                        marks,
                    )
                }
                Row::Omit(_) => (None, false, false, Vec::new()),
            };
            let kind_bg = kind_bg(kind);
            if let Some(text) = &shape.text {
                widest = widest.max(f32::from(text.width));
            }
            out.push(RowPaint {
                y0: y_of(i),
                y1: y_of(i + 1),
                kind_bg,
                bg: if drafting_here { rgb(DRAFTING_BG) } else { kind_bg },
                commented,
                text: shape.text.clone(),
                marks,
                label: shape.label.clone(),
            });
        }
        let n = rows.rows();
        let thumb = thumb_for(view_h, vp.max_top(side), side_top).map(|g| {
            let track = geom.track(side);
            let inset = (scrollbar::TRACK_WIDTH - scrollbar::THUMB_WIDTH) / 2.;
            let rect = Bounds::new(
                point(track.left() + px(inset), track.top() + g.thumb_top),
                size(px(scrollbar::THUMB_WIDTH), g.thumb_height),
            );
            Thumb {
                rect,
                shown: bars.shown(side),
                dragging: bars.drag.is_some_and(|(s, _)| s == side),
            }
        });
        SideFrame {
            rows: out,
            gaps: vp
                .gaps(side)
                .into_iter()
                .map(|(a, b)| (snap(top + a, scale), snap(top + b, scale)))
                .collect(),
            seams: vp
                .visible_seams(side)
                .iter()
                .map(|&r| (y_of(r as usize), (r as usize) < n))
                .collect(),
            empty_seam: rows
                .is_empty()
                .then(|| snap(top + vp.empty_seam(side), scale)),
            thumb,
            widest,
            x_offset: 0.,
        }
    });
    shapes.retain(&keep);

    let x_l = f32::from(geom.old.right());
    let x_r = f32::from(geom.new.left());
    let (bridges, waves) = if x_r - x_l >= 4. {
        let y = |v: f32| snap(top + v, scale);
        (
            vp.bridges()
                .into_iter()
                .map(|p| WinBridge {
                    kind: p.kind,
                    x_l,
                    x_r,
                    y_l0: y(p.y_l0),
                    y_l1: y(p.y_l1),
                    y_r0: y(p.y_r0),
                    y_r1: y(p.y_r1),
                })
                .collect(),
            vp.omit_links()
                .into_iter()
                .map(|(l, r)| (top + l, top + r))
                .collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };

    let (hitbox, code, tracks) = hitboxes;
    stats.build = trace::since(t_build);
    Frame {
        geom,
        row_h,
        hitbox,
        code,
        tracks,
        sides,
        bridges,
        waves,
        stats,
    }
}

fn shape_row(layout: &Layout, side: Side, row: Row<'_>, font_px: f32, window: &mut Window) -> RowShape {
    let Row::Line(line) = row else {
        return RowShape {
            text: None,
            label: None,
        };
    };
    let text = layout.side(side).text(line);
    RowShape {
        text: (!text.is_empty()).then(|| {
            shape(
                window,
                SharedString::from(text.to_string()),
                theme::MONO_FONT,
                font_px,
                theme::text(),
            )
        }),
        label: Some(shape(
            window,
            SharedString::from(line.ln.to_string()),
            theme::line_number_font(),
            LN_FONT_PX,
            theme::faint(),
        )),
    }
}

fn shape(window: &mut Window, text: SharedString, family: &'static str, font_px: f32, color: Rgba) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: font(family),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(font_px), &[run], None)
}

/// x spans of highlight runs (byte ranges into the line text).
fn mark_spans(runs: &[(usize, usize)], text: &ShapedLine) -> Vec<(f32, f32)> {
    runs.iter()
        .map(|&(a, b)| (a.min(text.len()), b.min(text.len())))
        .filter(|(a, b)| b > a)
        .map(|(a, b)| (f32::from(text.x_for_index(a)), f32::from(text.x_for_index(b))))
        .collect()
}

fn side_ix(side: Side) -> usize {
    match side {
        Side::Old => 0,
        Side::New => 1,
    }
}

fn kind_bg(kind: Option<LineKind>) -> Rgba {
    match kind {
        Some(LineKind::Replace) => theme::mod_bg(),
        Some(LineKind::Insert) => theme::add_bg(),
        Some(LineKind::Delete) => theme::del_bg(),
        Some(LineKind::Equal) | None => theme::white(),
    }
}

fn seam_color(side: Side) -> Rgba {
    match side {
        Side::Old => theme::add_bg(),
        Side::New => theme::del_bg(),
    }
}

fn hline(x0: f32, x1: f32, y: f32, h: f32) -> Bounds<Pixels> {
    Bounds::from_corners(point(px(x0), px(y)), point(px(x1.max(x0)), px(y + h)))
}

impl Frame {
    pub(super) fn widest(&self, side: Side) -> f32 {
        self.sides[side_ix(side)].widest
    }

    /// Code text / word mark shift for `side` (already clamped and snapped).
    pub(super) fn set_x_offset(&mut self, side: Side, x: f32) {
        self.sides[side_ix(side)].x_offset = x;
    }

    fn paint(&self, window: &mut Window, cx: &mut App) {
        let geom = self.geom;
        window.paint_quad(fill(geom.bounds, theme::white()));
        for side in [Side::Old, Side::New] {
            self.paint_code(side, window, cx);
        }
        window.with_content_mask(Some(ContentMask { bounds: geom.gutter }), |window| {
            self.paint_gutter(window, cx);
        });
        paint_omit_waves(window, geom, &self.waves);
        for frame in &self.sides {
            let Some(thumb) = frame.thumb.as_ref().filter(|t| t.shown) else {
                continue;
            };
            let color = if thumb.dragging {
                scrollbar::THUMB_ACTIVE
            } else {
                scrollbar::THUMB_IDLE
            };
            window.paint_quad(
                fill(thumb.rect, rgb(color)).corner_radii(px(scrollbar::THUMB_WIDTH / 2.)),
            );
        }
    }

    fn paint_code(&self, side: Side, window: &mut Window, cx: &mut App) {
        let pane = self.geom.pane(side);
        let frame = &self.sides[side_ix(side)];
        let (x0, x1) = (f32::from(pane.left()), f32::from(pane.right()));
        let row_h = px(self.row_h);
        let mark_h = (self.row_h - 1.).max(1.);
        // Clipped to the code pane: text scrolled left slides under the pane
        // edge (and the gutter never sees it).
        window.with_content_mask(Some(ContentMask { bounds: pane }), |window| {
            for row in &frame.rows {
                // Row backgrounds span the pane and do not scroll; text and
                // word marks move by the side's `x_offset`.
                window.paint_quad(fill(hline(x0, x1, row.y0, row.y1 - row.y0), row.bg));
                let mut text_x = x0 + TEXT_PAD - frame.x_offset;
                if row.commented {
                    text_x += COMMENT_BAR;
                }
                for &(a, b) in &row.marks {
                    let rect = hline(text_x + a, text_x + b, row.y0 + (self.row_h - mark_h) / 2., mark_h);
                    window.paint_quad(fill(rect, theme::mod_chg()).corner_radii(px(2.)));
                }
                if let Some(text) = &row.text {
                    text.paint(point(px(text_x), px(row.y0)), row_h, window, cx).ok();
                }
                // Row marker, pinned to the pane edge above scrolled text.
                if row.commented {
                    window.paint_quad(fill(
                        hline(x0, x0 + COMMENT_BAR, row.y0, row.y1 - row.y0),
                        theme::accent(),
                    ));
                }
            }
            paint_gaps(window, pane, &frame.gaps);
            if let Some(y) = frame.empty_seam {
                let mut stroke = PathBuilder::stroke(px(2.));
                stroke.move_to(point(px(x0 + 8.), px(y)));
                stroke.line_to(point(px(x1 - 8.), px(y)));
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, rgb(0x8aa0b8));
                }
            }
            for &(y, _) in &frame.seams {
                window.paint_quad(fill(hline(x0, x1, y, SEAM_H), seam_color(side)));
            }
        });
    }

    fn paint_gutter(&self, window: &mut Window, cx: &mut App) {
        let g = self.geom.gutter;
        let ln_w = self.geom.ln_w;
        window.paint_quad(fill(g, theme::white()));
        let cols = [
            (f32::from(g.left()), f32::from(g.left()) + ln_w),
            (f32::from(g.right()) - ln_w, f32::from(g.right())),
        ];
        for side in [Side::Old, Side::New] {
            let (c0, c1) = cols[side_ix(side)];
            let frame = &self.sides[side_ix(side)];
            for row in &frame.rows {
                window.paint_quad(fill(hline(c0, c1, row.y0, row.y1 - row.y0), row.kind_bg));
            }
            for &(y, in_rows) in &frame.seams {
                if in_rows {
                    window.paint_quad(fill(hline(c0, c1, y, SEAM_H), seam_color(side)));
                }
            }
        }
        paint_bridges(window, &self.bridges, ln_w);
        let row_h = px(self.row_h);
        for side in [Side::Old, Side::New] {
            let (c0, c1) = cols[side_ix(side)];
            for row in &self.sides[side_ix(side)].rows {
                let Some(label) = &row.label else { continue };
                // Old numbers hug the gutter's inner edge from the left column's
                // right; new numbers start at the right column's left.
                let x = match side {
                    Side::Old => c1 - LN_PAD - f32::from(label.width),
                    Side::New => c0 + LN_PAD,
                };
                label.paint(point(px(x), px(row.y0)), row_h, window, cx).ok();
            }
        }
    }
}

fn paint_bridges(window: &mut Window, placed: &[WinBridge], ln_w: f32) {
    for bridge in placed {
        let parallel = (bridge.y_l0 - bridge.y_r0).abs() < 1. && (bridge.y_l1 - bridge.y_r1).abs() < 1.;
        let mut path = PathBuilder::fill();
        if parallel {
            path.move_to(point(px(bridge.x_l), px(bridge.y_l0)));
            path.line_to(point(px(bridge.x_r), px(bridge.y_r0)));
            path.line_to(point(px(bridge.x_r), px(bridge.y_r1)));
            path.line_to(point(px(bridge.x_l), px(bridge.y_l1)));
        } else {
            pinch_bezier(&mut path, bridge, ln_w);
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, kind_bg(Some(bridge.kind)));
        }
    }
}

/// Full row on the long side, cubic Bézier through the center gutter, then a 2px
/// hairline across the short side's line-number column so it meets the code hairline.
fn pinch_bezier(path: &mut PathBuilder, bridge: &WinBridge, ln_w: f32) {
    let x_l = bridge.x_l;
    let x_r = bridge.x_r;
    let mid_l = x_l + ln_w;
    let mid_r = x_r - ln_w;
    let mw = (mid_r - mid_l).max(8.);
    match bridge.kind {
        LineKind::Delete => {
            let y0 = bridge.y_l0;
            let y1 = bridge.y_l1;
            let top = bridge.y_r0;
            let bot = bridge.y_r0 + 2.;
            path.move_to(point(px(x_l), px(y0)));
            path.line_to(point(px(mid_l), px(y0)));
            path.cubic_bezier_to(
                point(px(mid_r), px(top)),
                point(px(mid_l + mw * 0.45), px(y0)),
                point(px(mid_r - mw * 0.45), px(top)),
            );
            path.line_to(point(px(x_r), px(top)));
            path.line_to(point(px(x_r), px(bot)));
            path.line_to(point(px(mid_r), px(bot)));
            path.cubic_bezier_to(
                point(px(mid_l), px(y1)),
                point(px(mid_r - mw * 0.45), px(bot)),
                point(px(mid_l + mw * 0.45), px(y1)),
            );
            path.line_to(point(px(x_l), px(y1)));
        }
        LineKind::Insert => {
            let top = bridge.y_l0;
            let bot = bridge.y_l0 + 2.;
            let y0 = bridge.y_r0;
            let y1 = bridge.y_r1;
            path.move_to(point(px(x_l), px(top)));
            path.line_to(point(px(mid_l), px(top)));
            path.cubic_bezier_to(
                point(px(mid_r), px(y0)),
                point(px(mid_l + mw * 0.45), px(top)),
                point(px(mid_r - mw * 0.45), px(y0)),
            );
            path.line_to(point(px(x_r), px(y0)));
            path.line_to(point(px(x_r), px(y1)));
            path.line_to(point(px(mid_r), px(y1)));
            path.cubic_bezier_to(
                point(px(mid_l), px(bot)),
                point(px(mid_r - mw * 0.45), px(y1)),
                point(px(mid_l + mw * 0.45), px(bot)),
            );
            path.line_to(point(px(x_l), px(bot)));
        }
        _ => {
            // Hold the full block through each line-number column. The cubic
            // only runs between the columns, from the top of the left block
            // to the top of the right block (and bottom to bottom).
            path.move_to(point(px(x_l), px(bridge.y_l0)));
            path.line_to(point(px(mid_l), px(bridge.y_l0)));
            path.cubic_bezier_to(
                point(px(mid_r), px(bridge.y_r0)),
                point(px(mid_l + mw * 0.45), px(bridge.y_l0)),
                point(px(mid_r - mw * 0.45), px(bridge.y_r0)),
            );
            path.line_to(point(px(x_r), px(bridge.y_r0)));
            path.line_to(point(px(x_r), px(bridge.y_r1)));
            path.line_to(point(px(mid_r), px(bridge.y_r1)));
            path.cubic_bezier_to(
                point(px(mid_l), px(bridge.y_l1)),
                point(px(mid_r - mw * 0.45), px(bridge.y_r1)),
                point(px(mid_l + mw * 0.45), px(bridge.y_l1)),
            );
            path.line_to(point(px(x_l), px(bridge.y_l1)));
        }
    }
}

fn paint_omit_waves(window: &mut Window, geom: Geom, folds: &[(f32, f32)]) {
    if folds.is_empty() {
        return;
    }
    let bounds = geom.bounds;
    let x0 = f32::from(bounds.left());
    let x1 = f32::from(bounds.right());
    let gutter_l = f32::from(geom.old.right());
    let gutter_r = f32::from(geom.new.left());
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for &(y_l, y_r) in folds {
            // Bend only in the gap between the line-number columns. A slope
            // across the digits cuts through them when the folds are far apart.
            let path = joined_wave(x0, x1, y_l, gutter_l + geom.ln_w, gutter_r - geom.ln_w, y_r);
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(0xb5b5b5));
            }
        }
    });
}

const WAVE_PERIOD: f32 = 16.;
const WAVE_AMP: f32 = 3.5;
const WAVE_STEP: f32 = 2.;

fn wave_y(x: f32, base: f32, crest_at: Option<f32>) -> f32 {
    let phase = match crest_at {
        Some(lock) => (x - lock) / WAVE_PERIOD * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2,
        None => x / WAVE_PERIOD * std::f32::consts::TAU,
    };
    base + phase.sin() * WAVE_AMP
}

fn trace_wave(
    path: &mut PathBuilder,
    x0: f32,
    x1: f32,
    base: f32,
    crest_at: Option<f32>,
    first_move: bool,
) {
    if x1 < x0 {
        return;
    }
    let mut x = x0;
    let mut moved = !first_move;
    loop {
        let xx = x.min(x1);
        let p = point(px(xx), px(wave_y(xx, base, crest_at)));
        if moved {
            path.line_to(p);
        } else {
            path.move_to(p);
            moved = true;
        }
        if xx >= x1 - 0.01 {
            break;
        }
        x += WAVE_STEP;
    }
}

/// One stroke. Each side is a horizontal sine through its code and line numbers.
/// A height change is a cubic Bézier in the gap between the line-number columns.
/// Each sine meets that curve at a crest, so both tangents are horizontal, the
/// same way the change ribbons leave a flat edge.
fn joined_wave(x0: f32, x1: f32, y_l: f32, gap_l: f32, gap_r: f32, y_r: f32) -> PathBuilder {
    let mut path = PathBuilder::stroke(px(1.25));
    if x1 - x0 < 2. {
        return path;
    }
    let gap_l = gap_l.clamp(x0, x1);
    let gap_r = (gap_l + 8.).max(gap_r).min(x1);
    if (y_r - y_l).abs() < 0.5 {
        trace_wave(&mut path, x0, x1, y_l, None, true);
        return path;
    }
    trace_wave(&mut path, x0, gap_l, y_l, Some(gap_l), true);
    let y0 = y_l + WAVE_AMP;
    let y1 = y_r + WAVE_AMP;
    let dx = (gap_r - gap_l) * 0.45;
    path.cubic_bezier_to(
        point(px(gap_r), px(y1)),
        point(px(gap_l + dx), px(y0)),
        point(px(gap_r - dx), px(y1)),
    );
    if gap_r < x1 {
        trace_wave(&mut path, gap_r, x1, y_r, Some(gap_r), false);
    }
    path
}

fn paint_gaps(window: &mut Window, pane: Bounds<Pixels>, gaps: &[(f32, f32)]) {
    for &(y0, y1) in gaps {
        if y1 - y0 < 0.5 {
            continue;
        }
        let rect = hline(f32::from(pane.left()), f32::from(pane.right()), y0, y1 - y0);
        window.paint_quad(fill(rect, theme::gap_bg()));
        window.with_content_mask(Some(ContentMask { bounds: rect }), |window| {
            let width = f32::from(rect.size.width);
            let left = f32::from(rect.left());
            let top = f32::from(rect.top());
            let bottom = f32::from(rect.bottom());
            let mut y = top - width;
            while y < bottom {
                let mut stroke = PathBuilder::stroke(px(1.));
                stroke.move_to(point(px(left), px(y)));
                stroke.line_to(point(px(left + width), px(y + width)));
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, theme::faint());
                }
                y += 7.;
            }
        });
    }
}

/// The dual pane body. Holds only the entity; all state lives in `DualPane`.
pub struct DualPaneElement {
    pane: Entity<DualPane>,
}

pub fn dual_pane(pane: Entity<DualPane>) -> DualPaneElement {
    DualPaneElement { pane }
}

impl IntoElement for DualPaneElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for DualPaneElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Frame>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    /// `view_h` comes from these bounds, so the Viewport is this frame's.
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Frame> {
        self.pane
            .update(cx, |pane, cx| pane.prepaint_frame(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Option<Frame>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(frame) = frame.as_ref() else {
            return;
        };
        let t = trace::start();
        frame.paint(window, cx);
        for code in &frame.code {
            window.set_cursor_style(CursorStyle::PointingHand, code);
        }
        for track in frame.tracks.iter().flatten() {
            window.set_cursor_style(CursorStyle::Arrow, track);
        }
        register_listeners(&self.pane, frame, window);
        if t.is_some() {
            trace::frame(&frame.stats, trace::since(t));
        }
    }
}

/// Insert the element's hitboxes (prepaint only): the whole element for the
/// wheel, each code pane for clicks, each scrollbar track (when it scrolls).
pub(super) fn insert_hitboxes(
    geom: &Geom,
    tracks: [bool; 2],
    window: &mut Window,
) -> (Hitbox, [Hitbox; 2], [Option<Hitbox>; 2]) {
    let whole = window.insert_hitbox(geom.bounds, HitboxBehavior::Normal);
    let code = [Side::Old, Side::New].map(|side| window.insert_hitbox(geom.pane(side), HitboxBehavior::Normal));
    let tracks = [Side::Old, Side::New].map(|side| {
        tracks[side_ix(side)].then(|| window.insert_hitbox(geom.track(side), HitboxBehavior::Normal))
    });
    (whole, code, tracks)
}

fn register_listeners(pane: &Entity<DualPane>, frame: &Frame, window: &mut Window) {
    let geom = frame.geom;
    let hitbox = frame.hitbox.clone();
    let code = frame.code.clone();
    let tracks = frame.tracks.clone();

    let entity = pane.clone();
    let wheel_hitbox = hitbox.clone();
    let wheel_code = code.clone();
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !wheel_hitbox.should_handle_scroll(window) {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height());
        // Deltas are negative when the user scrolls down / right (AppKit
        // scrollingDelta, Windows wheel); flip to "content moves" signs.
        let (dx, dy) = route_wheel(-f32::from(delta.x), -f32::from(delta.y), event.modifiers.shift);
        if dy != 0. {
            entity.update(cx, |pane, cx| pane.scroll_by(dy, cx));
            cx.stop_propagation();
        } else if dx != 0. {
            // Horizontal input moves only the pane under the pointer; over
            // the gutter it does nothing.
            let side = [Side::Old, Side::New]
                .into_iter()
                .find(|&side| wheel_code[side_ix(side)].is_hovered(window));
            if let Some(side) = side {
                entity.update(cx, |pane, cx| pane.scroll_x_by(side, dx, cx));
            }
            cx.stop_propagation();
        }
    });

    let entity = pane.clone();
    let down_code = code.clone();
    let down_tracks = tracks.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        let y = f32::from(event.position.y) - geom.top();
        for side in [Side::Old, Side::New] {
            let ix = side_ix(side);
            if down_tracks[ix].as_ref().is_some_and(|t| t.is_hovered(window)) {
                let track = geom.track(side);
                let local = f32::from(event.position.y - track.top());
                entity.update(cx, |pane, cx| pane.press_track(side, local, cx));
                return;
            }
        }
        for side in [Side::Old, Side::New] {
            if down_code[side_ix(side)].is_hovered(window) {
                entity.update(cx, |pane, _| pane.press_row(side, y));
                return;
            }
        }
    });

    let entity = pane.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        let y = f32::from(event.position.y) - geom.top();
        let side = [Side::Old, Side::New]
            .into_iter()
            .find(|&side| code[side_ix(side)].is_hovered(window));
        entity.update(cx, |pane, cx| pane.release(side, y, cx));
    });

    let entity = pane.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let pos = event.position;
        let hovered = [Side::Old, Side::New]
            .map(|side| tracks[side_ix(side)].as_ref().is_some_and(|t| t.is_hovered(window)));
        let in_gutter = hitbox.is_hovered(window) && geom.gutter.contains(&pos);
        let y = f32::from(pos.y) - geom.top();
        let track_y = [Side::Old, Side::New].map(|side| f32::from(pos.y - geom.track(side).top()));
        entity.update(cx, |pane, cx| {
            pane.mouse_moved(hovered, in_gutter.then_some(y), track_y, cx)
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_drag_round_trips_the_side_top() {
        let (view_h, max_top) = (300., 2000.);
        for top in [0., 1., 640., 1999., 2000.] {
            let geom = thumb_for(view_h, max_top, top).unwrap();
            let back = top_at(&geom, f32::from(geom.thumb_top));
            assert!((back - top).abs() < 0.01, "top {top} came back as {back}");
        }
    }

    #[test]
    fn thumb_drag_clamps_to_the_side_travel() {
        let geom = thumb_for(300., 2000., 500.).unwrap();
        assert_eq!(top_at(&geom, -40.), 0.);
        assert_eq!(top_at(&geom, 10_000.), 2000.);
    }

    #[test]
    fn no_thumb_without_travel() {
        assert!(thumb_for(300., 0., 0.).is_none());
    }

    #[test]
    fn line_number_column_fits_the_digits() {
        assert_eq!(ln_col_width(2), 2. * LN_DIGIT_PX + LN_PAD);
        assert_eq!(ln_col_width(5), 5. * LN_DIGIT_PX + LN_PAD);
    }
}

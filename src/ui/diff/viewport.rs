//! Per-frame dual-pane geometry from a [`Layout`] and the one scroll parameter
//! `scroll_s`. Pure (no GPUI); cheap enough to rebuild every frame. Pixel values
//! are pane-relative: y = 0 is the top of the code panes. See
//! docs/dual-pane-diff.md §3.1 and docs/diffview-architecture.md §4.

use std::ops::Range;

use super::layout::{Bridge, Layout, LineKind, Row, ScrollKnot};
use crate::domain::{HunkJumpTarget, Side};

/// A bridge placed in pane pixels. `y_l*` are the old-side ends, `y_r*` the new.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedBridge {
    /// Index into [`Layout::bridges`].
    pub index: usize,
    pub kind: LineKind,
    pub y_l0: f32,
    pub y_l1: f32,
    pub y_r0: f32,
    pub y_r1: f32,
    /// `Some(side)`: culled. The other side's ends are both off the pane, so
    /// only `side`'s block is drawn, as a tab (§3.4).
    pub tab_side: Option<Side>,
}

impl PlacedBridge {
    pub fn ends(&self, side: Side) -> (f32, f32) {
        match side {
            Side::Old => (self.y_l0, self.y_l1),
            Side::New => (self.y_r0, self.y_r1),
        }
    }

    /// Vertical extent of what is drawn: both sides, or the tab's side.
    fn band(&self) -> (f32, f32) {
        match self.tab_side {
            Some(side) => self.ends(side),
            None => (self.y_l0.min(self.y_r0), self.y_l1.max(self.y_r1)),
        }
    }
}

/// Viewport position of the line held in place across a fold change (§3.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorCap {
    pub side: Side,
    pub ln: u32,
    pub view_y: f32,
}

pub struct Viewport<'a> {
    layout: &'a Layout,
    s: f32,
    view_h: f32,
    row_h: f32,
    anchor: f32,
    old_top: f32,
    new_top: f32,
}

impl<'a> Viewport<'a> {
    /// `scroll_s` is clamped to [`s_range`]; read the result back with [`Self::s`].
    pub fn new(layout: &'a Layout, scroll_s: f32, view_h: f32, row_h: f32) -> Self {
        let (lo, hi) = s_range(layout, view_h, row_h);
        let s = scroll_s.clamp(lo, hi);
        let anchor = anchor_of(view_h);
        let (old_y, new_y) = interp(s, &layout.knots, row_h);
        Self {
            layout,
            s,
            view_h,
            row_h,
            anchor,
            old_top: track(old_y, layout.old.rows(), anchor, row_h),
            new_top: track(new_y, layout.new.rows(), anchor, row_h),
        }
    }

    /// Snap both tops to the device pixel grid at `scale` (device px per
    /// logical px). Everything derived from the tops (rows, bridges, gaps,
    /// links, hit tests) follows.
    pub fn snapped(mut self, scale: f32) -> Self {
        self.old_top = snap(self.old_top, scale);
        self.new_top = snap(self.new_top, scale);
        self
    }

    pub fn s(&self) -> f32 {
        self.s
    }

    /// Largest top `side` can reach (its scrollbar travel).
    pub fn max_top(&self, side: Side) -> f32 {
        max_scroll(self.layout.side(side).rows(), self.anchor, self.row_h)
    }

    /// Scroll offset of `side`'s content (content y at the pane top).
    pub fn top(&self, side: Side) -> f32 {
        match side {
            Side::Old => self.old_top,
            Side::New => self.new_top,
        }
    }

    /// Rows of `side` that intersect the pane.
    pub fn visible_rows(&self, side: Side) -> Range<usize> {
        let n = self.layout.side(side).rows();
        if self.row_h <= 0. {
            return 0..0;
        }
        let top = self.top(side);
        let first = (top / self.row_h).floor().max(0.) as usize;
        let last = ((top + self.view_h) / self.row_h).ceil().max(0.) as usize;
        first.min(n)..last.min(n)
    }

    /// Pane y of an empty side's only seam.
    pub fn empty_seam(&self, side: Side) -> f32 {
        empty_seam(
            self.layout.side(other(side)).rows(),
            self.view_h,
            self.row_h,
        )
    }

    /// Seam rows of `side` whose hairline can be on screen.
    pub fn visible_seams(&self, side: Side) -> &'a [u32] {
        let seams = self.layout.side(side).seams();
        let rows = self.visible_rows(side);
        let a = seams.partition_point(|&s| (s as usize) < rows.start);
        let b = seams.partition_point(|&s| (s as usize) <= rows.end);
        &seams[a..b]
    }

    /// Hatched intervals of `side` (pane y), where the other side has rows
    /// this side does not.
    pub fn gaps(&self, side: Side) -> Vec<(f32, f32)> {
        let n = self.layout.side(side).rows();
        if n == 0 {
            return if self.view_h > 0. {
                vec![(0., self.view_h)]
            } else {
                Vec::new()
            };
        }
        let other_side = other(side);
        let own_top = self.top(side);
        let other_top = self.top(other_side);
        let covered = (-own_top, n as f32 * self.row_h - own_top);
        let mut out = Vec::new();
        for i in self.bridge_window() {
            let bridge = &self.layout.bridges[i];
            let from_other = match (bridge, side) {
                (Bridge::Insert { .. }, Side::Old) | (Bridge::Delete { .. }, Side::New) => true,
                _ => false,
            };
            if !from_other {
                continue;
            }
            let (from, to) = bridge.rows(other_side);
            let span = (
                from as f32 * self.row_h - other_top,
                to as f32 * self.row_h - other_top,
            );
            for (a, b) in subtract_span(span, covered) {
                let top = a.max(0.);
                let bot = b.min(self.view_h);
                if bot - top > 0.5 {
                    out.push((top, bot));
                }
            }
        }
        out
    }

    /// Bridges that can be on screen, placed in pane pixels.
    pub fn bridges(&self) -> Vec<PlacedBridge> {
        let off_pane =
            |(y0, y1): (f32, f32)| (y0 < 0. && y1 < 0.) || (y0 > self.view_h && y1 > self.view_h);
        self.bridge_window()
            .filter_map(|i| self.place(i))
            .filter_map(|mut p| {
                p.tab_side = match (off_pane(p.ends(Side::Old)), off_pane(p.ends(Side::New))) {
                    (false, false) => None,
                    (false, true) => Some(Side::Old),
                    (true, false) => Some(Side::New),
                    (true, true) => return None,
                };
                let (y0, y1) = p.band();
                (p.tab_side.is_none() || y1 > y0).then_some(p)
            })
            .collect()
    }

    /// Index of the first bridge whose drawn vertical band contains pane y `y`.
    pub fn bridge_at(&self, y: f32) -> Option<usize> {
        self.bridges().into_iter().find_map(|p| {
            let (y0, y1) = p.band();
            (y >= y0 && y <= y1.max(y0 + 2.)).then_some(p.index)
        })
    }

    /// Omission separator joins that can be on screen: `(old y, new y)` of each
    /// side's row middle, in pane pixels.
    pub fn omit_links(&self) -> Vec<(f32, f32)> {
        let old = self.layout.old.omits();
        let new = self.layout.new.omits();
        let top = |k: usize| {
            let l = old[k].row as f32 * self.row_h - self.old_top;
            let r = new[k].row as f32 * self.row_h - self.new_top;
            (l, r)
        };
        let mid = self.row_h / 2.;
        window(
            old.len().min(new.len()),
            |k| top(k).0.min(top(k).1),
            |k| top(k).0.max(top(k).1) + self.row_h,
            self.view_h,
        )
        .map(|k| (top(k).0 + mid, top(k).1 + mid))
        .collect()
    }

    /// What `side` shows at pane y `y`.
    pub fn hit(&self, side: Side, y: f32) -> Option<Row<'a>> {
        if self.row_h <= 0. {
            return None;
        }
        let r = ((y + self.top(side)) / self.row_h).floor();
        if r < 0. {
            return None;
        }
        self.layout.side(side).row(r as usize)
    }

    /// Row on the viewport anchor line, clamped into the side's rows.
    pub fn row_at_anchor(&self, side: Side) -> Option<usize> {
        row_at(
            self.layout.side(side).rows(),
            self.top(side),
            self.anchor,
            self.row_h,
        )
    }

    /// The line to hold in place across a fold change (§3.3). If the anchor
    /// is on an omission separator, the last line above it is held.
    pub fn capture_anchor(&self) -> Option<AnchorCap> {
        let hits = [Side::Old, Side::New].map(|side| (side, self.row_at_anchor(side)));
        let cap = |side: Side, row: u32, ln: u32| AnchorCap {
            side,
            ln,
            view_y: row as f32 * self.row_h - self.top(side),
        };
        for (side, hit) in hits {
            let Some(i) = hit else { continue };
            let rows = self.layout.side(side);
            if let Some(Row::Omit(_)) = rows.row(i) {
                return rows.line_above(i).map(|l| cap(side, l.row, l.ln));
            }
        }
        for (side, hit) in hits {
            if let Some(Row::Line(l)) = hit.and_then(|i| self.layout.side(side).row(i)) {
                return Some(cap(side, l.row, l.ln));
            }
        }
        None
    }

    /// Bridges near either side's visible rows. Bridges are monotone in rows on
    /// both sides, so the candidates are one contiguous index range.
    fn bridge_window(&self) -> Range<usize> {
        let bridges = &self.layout.bridges;
        if self.layout.old.is_empty() || self.layout.new.is_empty() {
            // An empty side's ends sit on its fixed seam, not in content rows.
            return 0..bridges.len();
        }
        let y = |rows: u32, side: Side| rows as f32 * self.row_h - self.top(side);
        window(
            bridges.len(),
            |i| {
                let b = &bridges[i];
                y(b.rows(Side::Old).0, Side::Old).min(y(b.rows(Side::New).0, Side::New))
            },
            |i| {
                let b = &bridges[i];
                y(b.rows(Side::Old).1, Side::Old).max(y(b.rows(Side::New).1, Side::New))
            },
            self.view_h,
        )
    }

    fn place(&self, index: usize) -> Option<PlacedBridge> {
        let old_n = self.layout.old.rows();
        let new_n = self.layout.new.rows();
        let content_y = |rows: u32, top: f32| rows as f32 * self.row_h - top;
        let edge_y = |rows: u32, n: usize, other_n: usize, top: f32| {
            if n == 0 {
                empty_seam(other_n, self.view_h, self.row_h)
            } else {
                content_y(rows, top)
            }
        };
        let (old_top, new_top) = (self.old_top, self.new_top);
        let (y_l0, y_l1, y_r0, y_r1) = match self.layout.bridges[index] {
            Bridge::Insert {
                old_seam,
                new_from,
                new_to,
                ..
            } => {
                if new_to <= new_from {
                    return None;
                }
                let seam = edge_y(old_seam, old_n, new_n, old_top);
                (
                    seam,
                    seam,
                    content_y(new_from, new_top),
                    content_y(new_to, new_top),
                )
            }
            Bridge::Delete {
                old_from,
                old_to,
                new_seam,
                ..
            } => {
                if old_to <= old_from {
                    return None;
                }
                let seam = edge_y(new_seam, new_n, old_n, new_top);
                (
                    content_y(old_from, old_top),
                    content_y(old_to, old_top),
                    seam,
                    seam,
                )
            }
            Bridge::Replace {
                old_from,
                old_to,
                new_from,
                new_to,
                ..
            } => {
                if old_to <= old_from && new_to <= new_from {
                    return None;
                }
                (
                    edge_y(old_from, old_n, new_n, old_top),
                    edge_y(old_to, old_n, new_n, old_top),
                    edge_y(new_from, new_n, old_n, new_top),
                    edge_y(new_to, new_n, old_n, new_top),
                )
            }
        };
        Some(PlacedBridge {
            index,
            kind: self.layout.bridges[index].kind(),
            y_l0,
            y_l1,
            y_r0,
            y_r1,
            tab_side: None,
        })
    }
}

/// Slack (px) so a seam hairline or a wave crest at the pane edge still counts.
const EDGE_SLACK: f32 = 4.;

/// Indices of items that can reach into the pane. `lo(i)` / `hi(i)` are the
/// item's top and bottom over both sides (pane y) and must be non-decreasing
/// in `i`, so "wholly above" is a prefix and "wholly below" a suffix.
fn window(
    n: usize,
    lo: impl Fn(usize) -> f32,
    hi: impl Fn(usize) -> f32,
    view_h: f32,
) -> Range<usize> {
    let start = partition(n, |i| hi(i) < -EDGE_SLACK);
    let end = partition(n, |i| lo(i) <= view_h + EDGE_SLACK);
    start..end.max(start)
}

/// Number of leading indices in `0..n` for which `pred` holds (pred must be
/// true then false).
fn partition(n: usize, pred: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if pred(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

fn other(side: Side) -> Side {
    match side {
        Side::Old => Side::New,
        Side::New => Side::Old,
    }
}

fn anchor_of(view_h: f32) -> f32 {
    if view_h < 1. { 0. } else { view_h / 3. }
}

/// Range of `scroll_s` (pixels) where the picture changes. Below the lower
/// bound both tops are 0; above the upper bound both are at their max. With
/// no measured viewport the whole knot range is allowed.
pub fn s_range(layout: &Layout, view_h: f32, row_h: f32) -> (f32, f32) {
    let end = layout.end_s() as f32 * row_h;
    if view_h < 1. || row_h <= 0. {
        return (0., end);
    }
    let anchor = anchor_of(view_h);
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for side in [Side::Old, Side::New] {
        let n = layout.side(side).rows();
        let max = max_scroll(n, anchor, row_h);
        if max <= 0. {
            continue;
        }
        // Top leaves 0 once content y passes the anchor; it sits at max once
        // content y reaches the last row's top.
        let leave = s_where(&layout.knots, side, anchor / row_h, false);
        let settle = s_where(&layout.knots, side, n as f32 - 1., true);
        if let (Some(a), Some(b)) = (leave, settle) {
            lo = lo.min(a * row_h);
            hi = hi.max(b * row_h);
        }
    }
    if lo > hi {
        return (0., 0.);
    }
    (lo.clamp(0., end), hi.clamp(0., end))
}

/// Smallest `s` (rows) with side y `>= y` when `reach`, else the largest `s`
/// with side y `<= y` (where y first exceeds it). `None` if never.
fn s_where(knots: &[ScrollKnot], side: Side, y: f32, reach: bool) -> Option<f32> {
    let past = |k: &ScrollKnot| {
        let ky = k.y(side) as f32;
        if reach { ky >= y } else { ky > y }
    };
    let b = knots.partition_point(|k| !past(k));
    let kb = knots.get(b)?;
    let Some(ka) = b.checked_sub(1).map(|a| knots[a]) else {
        return Some(kb.s as f32);
    };
    let (ya, yb) = (ka.y(side) as f32, kb.y(side) as f32);
    let t = if yb > ya { (y - ya) / (yb - ya) } else { 0. };
    Some(ka.s as f32 + t.clamp(0., 1.) * (kb.s as f32 - ka.s as f32))
}

/// Scroll parameter that puts content pixel `content_px` of `side` on the
/// anchor line. Keeps `current_s` if it already does; inside a stretch where
/// `side` stands still, lands where the side starts moving again.
pub fn s_for_content(
    layout: &Layout,
    side: Side,
    content_px: f32,
    row_h: f32,
    current_s: f32,
) -> f32 {
    let knots = &layout.knots;
    let (cur_old, cur_new) = interp(current_s, knots, row_h);
    let cur = if side == Side::Old { cur_old } else { cur_new };
    if (cur - content_px).abs() < 0.5 {
        return current_s;
    }
    let content_rows = content_px / row_h;
    for w in knots.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (a_y, b_y) = (a.y(side) as f32, b.y(side) as f32);
        if b_y > a_y && content_rows >= a_y && content_rows <= b_y {
            let t = (content_rows - a_y) / (b_y - a_y);
            return (a.s as f32 + t * (b.s as f32 - a.s as f32)) * row_h;
        }
    }
    let end = knots[knots.len() - 1];
    if content_rows >= end.y(side) as f32 {
        end.s as f32 * row_h
    } else {
        0.
    }
}

/// Scroll parameter that lands `target` on the anchor line (§3.5). Not
/// clamped: hunk navigation keeps the unclamped value; clamp with [`clamp_s`].
pub fn s_for_target(
    layout: &Layout,
    target: HunkJumpTarget,
    row_h: f32,
    current_s: f32,
) -> Option<f32> {
    let row = layout.side(target.side).row_of_line(target.ln)?;
    Some(s_for_content(
        layout,
        target.side,
        row as f32 * row_h,
        row_h,
        current_s,
    ))
}

/// Scroll parameter that puts the captured line back at its viewport y, clamped.
pub fn s_for_anchor(
    layout: &Layout,
    cap: AnchorCap,
    view_h: f32,
    row_h: f32,
    current_s: f32,
) -> Option<f32> {
    let row = layout.side(cap.side).row_of_line(cap.ln)?;
    let want = anchor_of(view_h) + row as f32 * row_h - cap.view_y;
    let s = s_for_content(layout, cap.side, want, row_h, current_s);
    Some(clamp_s(layout, s, view_h, row_h))
}

/// Rewrap / width / font / toggle: hold the logical line's first visual row on
/// the anchor (§3.1.1), even when capture was on a continuation row.
pub fn s_for_rewrap(
    layout: &Layout,
    cap: AnchorCap,
    view_h: f32,
    row_h: f32,
    _current_s: f32,
) -> Option<f32> {
    let row = layout.side(cap.side).row_of_line(cap.ln)?;
    Some(clamp_s(
        layout,
        s_for_content(layout, cap.side, row as f32 * row_h, row_h, 0.),
        view_h,
        row_h,
    ))
}

/// Match jump: land the visual row that contains `byte` at viewport center.
/// Hunk jumps keep the §3.1 one-third anchor via [`s_for_target`].
pub fn s_for_match_byte(
    layout: &Layout,
    side: Side,
    ln: u32,
    byte: usize,
    view_h: f32,
    row_h: f32,
    current_s: f32,
) -> Option<f32> {
    let row = layout.row_of_match_byte(side, ln, byte)?;
    let anchor = anchor_of(view_h);
    // top ≈ content_y − anchor; want row·row_h − top = view_h/2
    let content_px = row as f32 * row_h - view_h / 2. + anchor;
    Some(s_for_content(
        layout,
        side,
        content_px,
        row_h,
        current_s,
    ))
}

pub fn clamp_s(layout: &Layout, s: f32, view_h: f32, row_h: f32) -> f32 {
    let (lo, hi) = s_range(layout, view_h, row_h);
    s.clamp(lo, hi)
}

/// Content y on the anchor line for a side whose top is `scroll_top` (scrollbar drag).
pub fn content_from_top(scroll_top: f32, n: usize, view_h: f32, row_h: f32) -> f32 {
    let anchor = anchor_of(view_h);
    if n == 0 || scroll_top <= 0. {
        return 0.;
    }
    let max = max_scroll(n, anchor, row_h);
    if scroll_top >= max - 0.5 {
        n as f32 * row_h
    } else {
        scroll_top + anchor
    }
}

fn row_at(n: usize, scroll_top: f32, anchor: f32, row_h: f32) -> Option<usize> {
    if n == 0 || row_h <= 0. {
        return None;
    }
    let i = ((scroll_top + anchor) / row_h).floor();
    Some(if i < 0. { 0 } else { (i as usize).min(n - 1) })
}

/// Viewport Y of an empty side's only seam. A short opposite file cannot scroll
/// its last line down to the one-third anchor, so the seam sits on that line.
fn empty_seam(other_n: usize, view_h: f32, row_h: f32) -> f32 {
    let anchor = view_h / 3.;
    if other_n == 0 {
        return anchor;
    }
    ((other_n as f32 - 1.) * row_h).min(anchor)
}

/// One wheel / trackpad event split into (horizontal, vertical) travel, in
/// pixels, positive = right / down. Exactly one axis moves per event:
///
/// - Shift with no X delta maps the vertical delta to horizontal (a plain
///   mouse wheel; Windows already delivers shift+wheel as X).
/// - Otherwise the dominant axis wins and the other is dropped, so a mostly
///   horizontal trackpad swipe does not also nudge `scroll_s`, and a mostly
///   vertical one does not drift sideways. A tie goes to vertical.
pub fn route_wheel(dx: f32, dy: f32, shift: bool) -> (f32, f32) {
    if shift && dx == 0. {
        (dy, 0.)
    } else if dx.abs() > dy.abs() {
        (dx, 0.)
    } else {
        (0., dy)
    }
}

/// Horizontal travel of one code pane: how far its text (from the pane's inner
/// edge to the end of the widest line, padding included) overflows the pane.
pub fn max_x(text_extent: f32, pane_w: f32) -> f32 {
    (text_extent - pane_w).max(0.)
}

/// Keep a side's `x_offset` inside `0..=max`.
pub fn clamp_x(x: f32, max: f32) -> f32 {
    x.clamp(0., max.max(0.))
}

/// Horizontal scrollbar thumb geometry (matches `scrollbar.rs` insets/sizing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HThumbGeom {
    pub thumb_width: f32,
    pub thumb_left: f32,
    track_width: f32,
    max_x: f32,
}

const H_TRACK_PAD: f32 = 4.;
const H_MIN_THUMB: f32 = 24.;

/// Thumb along the bottom of a code pane when `max_x > 0`.
pub fn h_thumb_for(view_w: f32, max_x: f32, x_offset: f32) -> Option<HThumbGeom> {
    let track_width = (view_w - H_TRACK_PAD * 2.).max(0.);
    if max_x <= 0. || track_width <= 0. {
        return None;
    }
    let content = view_w + max_x;
    let thumb_width = (track_width * (view_w / content)).max(H_MIN_THUMB);
    let travel = (track_width - thumb_width).max(0.);
    let ratio = (x_offset / max_x).clamp(0., 1.);
    Some(HThumbGeom {
        thumb_width,
        thumb_left: travel * ratio,
        track_width,
        max_x,
    })
}

/// `x_offset` that puts the thumb's leading edge at `thumb_left` in the track.
pub fn x_at(geom: &HThumbGeom, thumb_left: f32) -> f32 {
    let travel = (geom.track_width - geom.thumb_width).max(0.);
    let ratio = if travel > 0. {
        (thumb_left / travel).clamp(0., 1.)
    } else {
        0.
    };
    geom.max_x * ratio
}

/// Horizontal travel when both panes share one offset (§3.1.2).
pub fn max_x_synced(max_per_side: [f32; 2]) -> f32 {
    max_per_side[0].max(max_per_side[1]).max(0.)
}

/// Clamp a shared horizontal offset to [`max_x_synced`].
pub fn clamp_x_synced(x: f32, max_per_side: [f32; 2]) -> f32 {
    clamp_x(x, max_x_synced(max_per_side))
}

/// Offset both sides should use when sync is turned on: the side last scrolled,
/// or old when neither side has been scrolled yet.
pub fn shared_x_on_sync_enable(offsets: [f32; 2], last_scrolled: Option<Side>) -> f32 {
    let ix = match last_scrolled {
        Some(Side::Old) | None => 0,
        Some(Side::New) => 1,
    };
    offsets[ix]
}

/// Round logical pixel `v` to the device pixel grid at `scale`.
pub fn snap(v: f32, scale: f32) -> f32 {
    if scale <= 0. {
        v
    } else {
        (v * scale).round() / scale
    }
}

fn max_scroll(n: usize, anchor: f32, row_h: f32) -> f32 {
    if n == 0 {
        0.
    } else {
        ((n as f32 - 1.) * row_h - anchor).max(0.)
    }
}

fn track(content_y: f32, n: usize, anchor: f32, row_h: f32) -> f32 {
    (content_y - anchor).clamp(0., max_scroll(n, anchor, row_h))
}

/// Per-side content y (pixels) at scroll parameter `s` (pixels).
fn interp(s: f32, knots: &[ScrollKnot], row_h: f32) -> (f32, f32) {
    let px = |k: ScrollKnot| (k.old_y as f32 * row_h, k.new_y as f32 * row_h);
    let Some(&first) = knots.first() else {
        return (0., 0.);
    };
    let b = knots.partition_point(|k| (k.s as f32 * row_h) < s);
    if b == 0 {
        return px(first);
    }
    let Some(&kb) = knots.get(b) else {
        return px(knots[knots.len() - 1]);
    };
    let ka = knots[b - 1];
    let (a_s, b_s) = (ka.s as f32 * row_h, kb.s as f32 * row_h);
    let t = if b_s == a_s {
        0.
    } else {
        (s - a_s) / (b_s - a_s)
    };
    let lerp = |a: u32, b: u32| (a as f32 + t * (b as f32 - a as f32)) * row_h;
    (lerp(ka.old_y, kb.old_y), lerp(ka.new_y, kb.new_y))
}

fn subtract_span(span: (f32, f32), cover: (f32, f32)) -> Vec<(f32, f32)> {
    let (a, b) = span;
    let (c, d) = cover;
    if b <= a {
        return Vec::new();
    }
    if d <= c || b <= c || a >= d {
        return vec![(a, b)];
    }
    let mut parts = Vec::new();
    if a < c {
        parts.push((a, c.min(b)));
    }
    if b > d {
        parts.push((d.max(a), b));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::super::layout::tests::{build, eq, lines, ten_then_insert};
    use super::*;
    use crate::domain::{Alignment, AlignmentOp, FoldState, LineSpan};

    const ROW_H: f32 = 20.;
    const VIEW_H: f32 = 300.;

    fn span(start: u32, count: u32) -> LineSpan {
        LineSpan { start, count }
    }

    fn tops(layout: &Layout, s: f32) -> (f32, f32) {
        let vp = Viewport::new(layout, s, VIEW_H, ROW_H);
        (vp.top(Side::Old), vp.top(Side::New))
    }

    /// Table of files: (name, old, new, ops). Index order is used below.
    fn cases() -> Vec<(&'static str, String, String, Vec<AlignmentOp>)> {
        let body = lines(1, 40, "L");
        vec![
            (
                "insert at start",
                body.clone(),
                format!("{}\n{body}", lines(1, 10, "N")),
                vec![
                    AlignmentOp::Insert {
                        after_old: 0,
                        news: span(1, 10),
                    },
                    eq(1, 11, 40),
                ],
            ),
            (
                "insert at end",
                body.clone(),
                format!("{body}\n{}", lines(41, 60, "N")),
                vec![
                    eq(1, 1, 40),
                    AlignmentOp::Insert {
                        after_old: 40,
                        news: span(41, 20),
                    },
                ],
            ),
            (
                "delete in the middle",
                format!(
                    "{}\n{}\n{}",
                    lines(1, 20, "L"),
                    lines(1, 8, "D"),
                    lines(21, 40, "L")
                ),
                body.clone(),
                vec![
                    eq(1, 1, 20),
                    AlignmentOp::Delete {
                        olds: span(21, 8),
                        at_new: 21,
                    },
                    eq(29, 21, 20),
                ],
            ),
            (
                "3→1 replace",
                format!("{}\nA\nB\nC\n{}", lines(1, 20, "L"), lines(21, 40, "L")),
                format!("{}\nZ\n{}", lines(1, 20, "L"), lines(21, 40, "L")),
                vec![
                    eq(1, 1, 20),
                    AlignmentOp::Replace {
                        olds: span(21, 3),
                        news: span(21, 1),
                    },
                    eq(24, 22, 20),
                ],
            ),
            (
                "one side empty",
                String::new(),
                body.clone(),
                vec![AlignmentOp::Insert {
                    after_old: 0,
                    news: span(1, 40),
                }],
            ),
        ]
    }

    #[test]
    fn snap_rounds_to_device_pixels() {
        assert_eq!(snap(10.3, 1.), 10.);
        assert_eq!(snap(10.3, 2.), 10.5);
        assert_eq!(snap(10.2, 2.), 10.);
        assert_eq!(snap(10.4, 1.5), 32. / 3.);
        assert_eq!(snap(7.25, 0.), 7.25);
    }

    #[test]
    fn snapped_tops_follow_into_hit_tests_and_bridges() {
        let (_, old, new, ops) = cases().remove(0);
        let layout = build(&old, &new, ops, None);
        let (lo, _) = s_range(&layout, VIEW_H, ROW_H);
        let raw = Viewport::new(&layout, lo + 60.3, VIEW_H, ROW_H);
        let vp = Viewport::new(&layout, lo + 60.3, VIEW_H, ROW_H).snapped(2.);
        assert_eq!(vp.top(Side::New), snap(raw.top(Side::New), 2.));
        assert!((vp.top(Side::New) * 2.).fract() == 0.);
        for p in vp.bridges() {
            assert!((p.y_r0 * 2.).fract() == 0. && (p.y_r1 * 2.).fract() == 0.);
        }
    }

    #[test]
    fn max_top_is_scrollbar_travel() {
        let (_, old, new, ops) = cases().remove(1);
        let layout = build(&old, &new, ops, None);
        let (_, hi) = s_range(&layout, VIEW_H, ROW_H);
        let vp = Viewport::new(&layout, hi, VIEW_H, ROW_H);
        assert_eq!(vp.top(Side::Old), vp.max_top(Side::Old));
        assert_eq!(vp.top(Side::New), vp.max_top(Side::New));
    }

    #[test]
    fn s_range_has_no_dead_travel() {
        for (name, old, new, ops) in cases() {
            for fold in [None, Some(FoldState::collapsed())] {
                let layout = build(&old, &new, ops.clone(), fold.as_ref());
                let (lo, hi) = s_range(&layout, VIEW_H, ROW_H);
                let end = layout.end_s() as f32 * ROW_H;
                assert!(lo < hi, "{name}: file taller than the pane must scroll");
                // Outside the range the picture is the same as at its edge.
                assert_eq!(tops(&layout, lo), (0., 0.), "{name}: both at top at lo");
                assert_eq!(
                    Viewport::new(&layout, 0., VIEW_H, ROW_H).s(),
                    lo,
                    "{name}: s below the range clamps to lo"
                );
                assert_eq!(
                    Viewport::new(&layout, end + 100., VIEW_H, ROW_H).s(),
                    hi,
                    "{name}: s above the range clamps to hi"
                );
                // Every step inside the range moves at least one side.
                let mut s = lo;
                let mut prev = tops(&layout, s);
                while s < hi {
                    s = (s + 1.).min(hi);
                    let next = tops(&layout, s);
                    assert!(next != prev, "{name}: no motion at s = {s}");
                    prev = next;
                }
            }
        }
    }

    /// The unclamped tops at the range edges equal the tops past them, so
    /// clamping changes no picture.
    #[test]
    fn s_range_edges_match_the_unclamped_extremes() {
        for (name, old, new, ops) in cases() {
            let layout = build(&old, &new, ops, None);
            let (lo, hi) = s_range(&layout, VIEW_H, ROW_H);
            let end = layout.end_s() as f32 * ROW_H;
            let anchor = VIEW_H / 3.;
            let raw = |s: f32| {
                let (o, n) = interp(s, &layout.knots, ROW_H);
                (
                    track(o, layout.old.rows(), anchor, ROW_H),
                    track(n, layout.new.rows(), anchor, ROW_H),
                )
            };
            assert_eq!(raw(0.), raw(lo), "{name}: top dead zone is invisible");
            assert_eq!(raw(end), raw(hi), "{name}: bottom dead zone is invisible");
            assert!(raw(lo + 1.) != raw(lo), "{name}: lo is the first moving s");
            assert!(raw(hi - 1.) != raw(hi), "{name}: hi is the last moving s");
        }
    }

    #[test]
    fn s_range_removes_the_top_dead_zone() {
        let (_, old, new, ops) = cases().remove(1);
        let layout = build(&old, &new, ops, None);
        let (lo, _) = s_range(&layout, VIEW_H, ROW_H);
        // Equal from the top: both sides reach the anchor at s = view_h / 3.
        assert_eq!(lo, VIEW_H / 3.);
        // A wheel step from the clamped start moves the picture at once.
        let vp = Viewport::new(&layout, 0., VIEW_H, ROW_H);
        assert_eq!(vp.s(), lo);
        assert!(tops(&layout, vp.s() + 5.).0 > 0.);
    }

    #[test]
    fn file_shorter_than_the_pane_does_not_scroll() {
        let layout = build(
            "a\nb",
            "a\nb\nc",
            vec![
                eq(1, 1, 2),
                AlignmentOp::Insert {
                    after_old: 2,
                    news: span(3, 1),
                },
            ],
            None,
        );
        assert_eq!(s_range(&layout, VIEW_H, ROW_H), (0., 0.));
        assert_eq!(Viewport::new(&layout, 50., VIEW_H, ROW_H).s(), 0.);
    }

    #[test]
    fn unmeasured_viewport_allows_the_whole_knot_range() {
        let (_, old, new, ops) = cases().remove(0);
        let layout = build(&old, &new, ops, None);
        assert_eq!(s_range(&layout, 0., ROW_H), (0., 50. * ROW_H));
    }

    #[test]
    fn shorter_side_stays_still_inside_insert_rows() {
        let (_, old, new, ops) = cases().remove(0);
        let layout = build(&old, &new, ops, None);
        // Inside the 10 inserted rows past the anchor, old waits at 0.
        let (lo, _) = s_range(&layout, VIEW_H, ROW_H);
        let (old_a, new_a) = tops(&layout, lo);
        let (old_b, new_b) = tops(&layout, lo + 60.);
        assert_eq!((old_a, old_b), (0., 0.));
        assert_eq!(new_b - new_a, 60.);
        // Past the insert both move together, 10 rows apart.
        let (o1, n1) = tops(&layout, 20. * ROW_H);
        let (o2, n2) = tops(&layout, 20. * ROW_H + 40.);
        assert_eq!((o2 - o1, n2 - n1), (40., 40.));
        assert_eq!(n1 - o1, 10. * ROW_H, "gap equals the insert height");
    }

    #[test]
    fn end_of_file_gap_hatches_below_the_shorter_side() {
        let (_, old, new, ops) = cases().remove(1);
        let layout = build(&old, &new, ops, None);
        // At the file end both sides sit at max: last lines share one row.
        let (_, hi) = s_range(&layout, VIEW_H, ROW_H);
        let end = Viewport::new(&layout, hi, VIEW_H, ROW_H);
        assert_eq!(
            40. * ROW_H - end.top(Side::Old),
            60. * ROW_H - end.top(Side::New)
        );
        // Inside the trailing insert, old has stopped; below its last line is gap.
        let vp = Viewport::new(&layout, 45. * ROW_H, VIEW_H, ROW_H);
        assert_eq!(vp.top(Side::Old), 39. * ROW_H - VIEW_H / 3.);
        let old_bottom = 40. * ROW_H - vp.top(Side::Old);
        assert_eq!(vp.gaps(Side::Old), vec![(old_bottom, VIEW_H)]);
        assert!(vp.gaps(Side::New).is_empty());
        // The insert bridge pinches at the old seam (end of old content).
        let placed = vp.bridges();
        assert_eq!(placed.len(), 1);
        assert_eq!((placed[0].y_l0, placed[0].y_l1), (old_bottom, old_bottom));
        assert_eq!(vp.bridge_at(old_bottom + 1.), Some(0));
        assert_eq!(vp.visible_seams(Side::Old), [40]);
    }

    #[test]
    fn one_empty_side_hatches_everything_and_pinches_at_its_seam() {
        let (_, old, new, ops) = cases().remove(4);
        let layout = build(&old, &new, ops, None);
        let vp = Viewport::new(&layout, 0., VIEW_H, ROW_H);
        assert_eq!(vp.gaps(Side::Old), vec![(0., VIEW_H)]);
        let placed = vp.bridges();
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].y_l0, VIEW_H / 3.);
        assert_eq!(vp.empty_seam(Side::Old), VIEW_H / 3.);
        assert!(vp.hit(Side::Old, 10.).is_none());
    }

    #[test]
    fn visible_rows_and_hit_follow_the_top() {
        let (_, old, new, ops) = cases().remove(1);
        let layout = build(&old, &new, ops, None);
        let vp = Viewport::new(&layout, VIEW_H / 3. + 30., VIEW_H, ROW_H);
        assert_eq!(vp.top(Side::Old), 30.);
        assert_eq!(vp.visible_rows(Side::Old), 1..17);
        let Some(Row::Line(l)) = vp.hit(Side::Old, 0.) else {
            panic!("line at pane top");
        };
        assert_eq!(l.ln, 2);
        assert_eq!(vp.row_at_anchor(Side::Old), Some(6));
    }

    /// 30 Equal, 2 inserted, 30 Equal; both Equal runs fold.
    fn folded_case() -> (String, String, Vec<AlignmentOp>) {
        let old = lines(1, 60, "L");
        let new = format!("{}\nX\nY\n{}", lines(1, 30, "L"), lines(31, 60, "L"));
        let ops = vec![
            eq(1, 1, 30),
            AlignmentOp::Insert {
                after_old: 30,
                news: span(31, 2),
            },
            eq(31, 33, 30),
        ];
        (old, new, ops)
    }

    fn view_y(layout: &Layout, s: f32, side: Side, ln: u32) -> f32 {
        let vp = Viewport::new(layout, s, VIEW_H, ROW_H);
        layout.side(side).row_of_line(ln).unwrap() as f32 * ROW_H - vp.top(side)
    }

    fn wrap_layout(
        old: &str,
        new: &str,
        ops: Vec<AlignmentOp>,
        width: f32,
        fold: Option<&FoldState>,
    ) -> Layout {
        use super::super::layout::Layout;
        use super::super::visual_wrap::{WrapPlan, WrapSide};
        let alignment = Alignment { ops };
        let plan = WrapPlan {
            old: WrapSide { width_px: width },
            new: WrapSide { width_px: width },
        };
        let mut cw = |_: char| 10.0f32;
        Layout::build(
            old.into(),
            new.into(),
            &alignment,
            fold,
            Some((&plan, &mut cw)),
        )
    }

    #[test]
    fn anchor_line_keeps_its_place_across_expand_and_collapse() {
        let (old, new, ops) = folded_case();
        let mut fold = FoldState::collapsed();
        let collapsed = build(&old, &new, ops.clone(), Some(&fold));
        // Old rows: L1-3, ~, L28-30, L31-33, ~, L58-60. Put old row 8 (L32) on the anchor.
        let s = s_for_content(&collapsed, Side::Old, 8.5 * ROW_H, ROW_H, 0.);
        let vp = Viewport::new(&collapsed, s, VIEW_H, ROW_H);
        let cap = vp.capture_anchor().expect("anchor line");
        assert_eq!((cap.side, cap.ln), (Side::Old, 32));
        let before = view_y(&collapsed, vp.s(), Side::Old, 32);

        fold.expand(0);
        let expanded = build(&old, &new, ops.clone(), Some(&fold));
        let s2 = s_for_anchor(&expanded, cap, VIEW_H, ROW_H, vp.s()).unwrap();
        assert_eq!(
            view_y(&expanded, s2, Side::Old, 32),
            before,
            "expand keeps L32"
        );

        // Collapse again from the expanded picture.
        let vp2 = Viewport::new(&expanded, s2, VIEW_H, ROW_H);
        let cap2 = vp2.capture_anchor().unwrap();
        let recollapsed = build(&old, &new, ops, Some(&FoldState::collapsed()));
        let s3 = s_for_anchor(&recollapsed, cap2, VIEW_H, ROW_H, s2).unwrap();
        assert_eq!(
            view_y(&recollapsed, s3, cap2.side, cap2.ln),
            view_y(&expanded, s2, cap2.side, cap2.ln),
            "collapse keeps the anchor line"
        );
    }

    #[test]
    fn anchor_on_a_separator_holds_the_line_above_it() {
        let (old, new, ops) = folded_case();
        let collapsed = build(&old, &new, ops.clone(), Some(&FoldState::collapsed()));
        // Old row 10 is the second separator.
        let s = s_for_content(&collapsed, Side::Old, 10.5 * ROW_H, ROW_H, 0.);
        let vp = Viewport::new(&collapsed, s, VIEW_H, ROW_H);
        let at = vp.row_at_anchor(Side::Old).unwrap();
        assert!(matches!(collapsed.old.row(at), Some(Row::Omit(_))));
        let cap = vp.capture_anchor().unwrap();
        assert_eq!((cap.side, cap.ln), (Side::Old, 33));
        let before = view_y(&collapsed, vp.s(), Side::Old, 33);
        let mut fold = FoldState::collapsed();
        fold.expand(2);
        let expanded = build(&old, &new, ops, Some(&fold));
        let s2 = s_for_anchor(&expanded, cap, VIEW_H, ROW_H, vp.s()).unwrap();
        assert_eq!(view_y(&expanded, s2, Side::Old, 33), before);
    }

    #[test]
    fn hunk_target_lands_on_the_anchor_line() {
        let (old, new, ops) = folded_case();
        let mut fold = FoldState::collapsed();
        fold.expand(0);
        fold.expand(2);
        let layout = build(&old, &new, ops, Some(&fold));
        let target = layout.hunk_lands[0].target;
        assert_eq!((target.side, target.ln), (Side::New, 31));
        let s = s_for_target(&layout, target, ROW_H, 0.).unwrap();
        assert_eq!(view_y(&layout, s, Side::New, 31), VIEW_H / 3.);
        // The old side shows its seam (after old 30) at the same height.
        let vp = Viewport::new(&layout, s, VIEW_H, ROW_H);
        assert_eq!(30. * ROW_H - vp.top(Side::Old), VIEW_H / 3.);
    }

    #[test]
    fn omit_links_join_row_middles_of_both_sides() {
        let (old, new, ops) = folded_case();
        let layout = build(&old, &new, ops, Some(&FoldState::collapsed()));
        let vp = Viewport::new(&layout, 0., VIEW_H, ROW_H);
        let links = vp.omit_links();
        // Both separators are within the 300px pane at the top.
        assert_eq!(links.len(), 2);
        assert_eq!(links[0], (3.5 * ROW_H, 3.5 * ROW_H));
        // Second separator: the new side is 2 rows lower (the insert).
        assert_eq!(links[1].1 - links[1].0, 2. * ROW_H);
    }

    #[test]
    fn off_screen_bridges_and_links_are_culled() {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        let mut ops = Vec::new();
        // 50 blocks of 10 Equal + 1 Replace.
        for b in 0..50u32 {
            let base = b * 11 + 1;
            for i in 0..10 {
                old_lines.push(format!("e{b}-{i}"));
                new_lines.push(format!("e{b}-{i}"));
            }
            old_lines.push(format!("o{b}"));
            new_lines.push(format!("n{b}"));
            ops.push(eq(base, base, 10));
            ops.push(AlignmentOp::Replace {
                olds: span(base + 10, 1),
                news: span(base + 10, 1),
            });
        }
        let (old, new) = (old_lines.join("\n"), new_lines.join("\n"));
        for fold in [None, Some(FoldState::collapsed())] {
            let layout = build(&old, &new, ops.clone(), fold.as_ref());
            let (lo, hi) = s_range(&layout, VIEW_H, ROW_H);
            let vp = Viewport::new(&layout, (lo + hi) / 2., VIEW_H, ROW_H);
            let placed = vp.bridges();
            assert!(
                !placed.is_empty() && placed.len() <= 5,
                "got {}",
                placed.len()
            );
            for p in &placed {
                assert!(p.y_l1 >= 0. && p.y_l0 <= VIEW_H);
            }
            assert!(vp.omit_links().len() <= 5);
        }
    }

    /// 20 Equal, a 1↔40 (or 40↔1) Replace, 2 Equal, then `second`, 20 Equal.
    fn stretched_case(old_long: bool, second: AlignmentOp) -> Layout {
        let (o, n) = if old_long { (40, 1) } else { (1, 40) };
        let second_rows = |s: &AlignmentOp| match s {
            AlignmentOp::Replace { olds, news } => (olds.count, news.count),
            AlignmentOp::Insert { news, .. } => (0, news.count),
            _ => unreachable!(),
        };
        let (old_rows, new_rows) = second_rows(&second);
        let old = lines(1, 20 + o + 2 + old_rows + 20, "L");
        let new = lines(1, 20 + n + 2 + new_rows + 20, "L");
        let ops = vec![
            eq(1, 1, 20),
            AlignmentOp::Replace {
                olds: span(21, o),
                news: span(21, n),
            },
            eq(21 + o, 21 + n, 2),
            second,
            eq(23 + o + old_rows, 23 + n + new_rows, 20),
        ];
        build(&old, &new, ops, None)
    }

    /// Scroll that puts the middle of the long side of the first Replace on
    /// the anchor; the short side waits there, so the second bridge's long
    /// side ends are far below the pane.
    fn mid_first_hunk(layout: &Layout, long: Side) -> Viewport<'_> {
        let s = s_for_content(layout, long, 40. * ROW_H, ROW_H, 0.);
        Viewport::new(layout, s, VIEW_H, ROW_H)
    }

    #[test]
    fn bridge_with_new_side_off_screen_is_culled_to_old() {
        let second = AlignmentOp::Replace {
            olds: span(24, 1),
            news: span(63, 1),
        };
        let layout = stretched_case(false, second);
        let vp = mid_first_hunk(&layout, Side::New);
        let placed = vp.bridges();
        let b = placed.iter().find(|p| p.index == 1).expect("second bridge");
        assert!(b.y_r0 > VIEW_H, "new end is below the pane");
        assert!(b.y_l0 >= 0. && b.y_l1 <= VIEW_H, "old block is on screen");
        assert_eq!(b.tab_side, Some(Side::Old));
        let first = placed.iter().find(|p| p.index == 0).unwrap();
        assert_eq!(
            first.tab_side, None,
            "a side spanning the pane is not culled"
        );
    }

    #[test]
    fn bridge_with_new_side_above_the_pane_is_culled_to_old() {
        let second = AlignmentOp::Replace {
            olds: span(24, 1),
            news: span(63, 40),
        };
        let layout = stretched_case(false, second);
        // Middle of the second Replace's 40 new lines on the anchor.
        let s = s_for_content(&layout, Side::New, 82. * ROW_H, ROW_H, 0.);
        let vp = Viewport::new(&layout, s, VIEW_H, ROW_H);
        let b = vp.bridges().into_iter().find(|p| p.index == 0).unwrap();
        assert!(b.y_r1 < 0., "new end is above the pane");
        assert!(b.y_l0 >= 0. && b.y_l1 <= VIEW_H, "old block is on screen");
        assert_eq!(b.tab_side, Some(Side::Old));
    }

    #[test]
    fn bridge_with_old_side_off_screen_is_culled_to_new() {
        let second = AlignmentOp::Replace {
            olds: span(63, 1),
            news: span(24, 1),
        };
        let layout = stretched_case(true, second);
        let vp = mid_first_hunk(&layout, Side::Old);
        let b = vp.bridges().into_iter().find(|p| p.index == 1).unwrap();
        assert!(b.y_l0 > VIEW_H, "old end is below the pane");
        assert!(b.y_r0 >= 0. && b.y_r1 <= VIEW_H, "new block is on screen");
        assert_eq!(b.tab_side, Some(Side::New));
    }

    #[test]
    fn culled_bridge_with_a_zero_height_visible_side_is_dropped() {
        let second = AlignmentOp::Insert {
            after_old: 23,
            news: span(63, 3),
        };
        let layout = stretched_case(false, second);
        let vp = mid_first_hunk(&layout, Side::New);
        assert!(vp.bridges().iter().all(|p| p.index != 1));
        // The old seam still marks the insertion point.
        assert_eq!(vp.visible_seams(Side::Old), [23]);
    }

    #[test]
    fn wheel_routes_to_one_axis() {
        // (dx, dy, shift) -> (h, v)
        let cases = [
            ((0., 30., false), (0., 30.)),
            ((40., 0., false), (40., 0.)),
            ((-40., 3., false), (-40., 0.)),
            ((2., -25., false), (0., -25.)),
            ((10., 10., false), (0., 10.)),
            ((0., 30., true), (30., 0.)),
            ((0., -30., true), (-30., 0.)),
            // Windows: shift+wheel already arrives as X.
            ((45., 0., true), (45., 0.)),
            ((0., 0., false), (0., 0.)),
        ];
        for ((dx, dy, shift), want) in cases {
            assert_eq!(
                route_wheel(dx, dy, shift),
                want,
                "dx {dx} dy {dy} shift {shift}"
            );
        }
    }

    #[test]
    fn x_travel_is_the_overflow_of_the_widest_line() {
        assert_eq!(max_x(900., 400.), 500.);
        assert_eq!(max_x(300., 400.), 0.);
        assert_eq!(max_x(0., 0.), 0.);
    }

    #[test]
    fn x_offset_clamps_to_the_travel() {
        assert_eq!(clamp_x(-5., 100.), 0.);
        assert_eq!(clamp_x(50., 100.), 50.);
        assert_eq!(clamp_x(250., 100.), 100.);
        // A pane that grew past its text snaps back to the start.
        assert_eq!(clamp_x(80., 0.), 0.);
        assert_eq!(clamp_x(80., -3.), 0.);
    }

    #[test]
    fn synced_horizontal_bound_is_the_larger_side() {
        assert_eq!(max_x_synced([80., 120.]), 120.);
        assert_eq!(max_x_synced([200., 50.]), 200.);
        assert_eq!(clamp_x_synced(150., [80., 120.]), 120.);
        assert_eq!(clamp_x_synced(150., [200., 50.]), 150.);
        assert_eq!(clamp_x_synced(-10., [30., 40.]), 0.);
    }

    #[test]
    fn enabling_sync_picks_last_scrolled_side_or_old() {
        assert_eq!(shared_x_on_sync_enable([10., 90.], None), 10.);
        assert_eq!(shared_x_on_sync_enable([10., 90.], Some(Side::Old)), 10.);
        assert_eq!(shared_x_on_sync_enable([10., 90.], Some(Side::New)), 90.);
    }

    #[test]
    fn h_thumb_none_without_overflow() {
        assert!(h_thumb_for(400., 0., 0.).is_none());
        assert!(h_thumb_for(0., 100., 0.).is_none());
    }

    #[test]
    fn h_thumb_at_left_and_right() {
        let left = h_thumb_for(400., 500., 0.).unwrap();
        assert_eq!(left.thumb_left, 0.);

        let right = h_thumb_for(400., 500., 500.).unwrap();
        let travel = right.track_width - right.thumb_width;
        assert!((right.thumb_left - travel).abs() < 0.01);
    }

    #[test]
    fn h_thumb_drag_round_trips_x_offset() {
        let view_w = 400.;
        let max = 800.;
        for x in [0., 1., 200., 799., 800.] {
            let geom = h_thumb_for(view_w, max, x).unwrap();
            let back = x_at(&geom, geom.thumb_left);
            assert!((back - x).abs() < 0.01, "x {x} came back as {back}");
        }
    }

    #[test]
    fn h_thumb_drag_clamps_to_travel() {
        let geom = h_thumb_for(400., 800., 200.).unwrap();
        assert_eq!(x_at(&geom, -40.), 0.);
        assert_eq!(x_at(&geom, 10_000.), 800.);
    }

    #[test]
    fn synced_h_thumbs_use_shared_max_for_geometry() {
        let view_w = 400.;
        let max_per_side = [200., 600.];
        let shared = max_x_synced(max_per_side);
        let x_offset = 300.;
        let g_short = h_thumb_for(view_w, shared, x_offset).unwrap();
        let g_long = h_thumb_for(view_w, shared, x_offset).unwrap();
        assert_eq!(g_short.thumb_left, g_long.thumb_left);
        let per_side_short = h_thumb_for(view_w, max_per_side[0], x_offset).unwrap();
        assert_ne!(g_short.thumb_left, per_side_short.thumb_left);
        let travel = g_short.track_width - g_short.thumb_width;
        assert!((x_at(&g_short, travel) - shared).abs() < 0.01);
    }

    #[test]
    fn hit_on_continuation_row_maps_to_logical_line() {
        use super::super::layout::LinePart;
        let text = "a".repeat(25);
        let layout = wrap_layout(&text, &text, vec![eq(1, 1, 1)], 100., None);
        let vp = Viewport::new(&layout, 0., VIEW_H, ROW_H);
        let Some(Row::Line(l)) = vp.hit(Side::Old, ROW_H + 2.) else {
            panic!("continuation row");
        };
        assert_eq!((l.ln, l.part), (1, LinePart::Continuation));
    }

    #[test]
    fn rewrap_pins_logical_line_first_visual_row() {
        let text = "a".repeat(25);
        let layout = wrap_layout(&text, &text, vec![eq(1, 1, 1)], 100., None);
        let s = s_for_content(&layout, Side::Old, 0., ROW_H, 0.);
        let before = view_y(&layout, s, Side::Old, 1);
        let cap = Viewport::new(&layout, s, VIEW_H, ROW_H)
            .capture_anchor()
            .expect("anchor");
        let layout2 = wrap_layout(&text, &text, vec![eq(1, 1, 1)], 50., None);
        let s2 = s_for_rewrap(&layout2, cap, VIEW_H, ROW_H, s).unwrap();
        assert_eq!(view_y(&layout2, s2, Side::Old, 1), before);
    }

    #[test]
    fn rewrap_from_continuation_row_pins_first_visual_row() {
        use super::super::layout::LinePart;
        const H: f32 = 60.;
        let long = "a".repeat(50);
        let text = format!("head\n{long}");
        let ops = vec![eq(1, 1, 1), eq(2, 2, 1)];
        let layout = wrap_layout(&text, &text, ops.clone(), 100., None);
        assert!(layout.old.rows() >= 4);
        let first = layout.old.row_of_line(2).unwrap();
        assert!(first > 0, "wrapped line not at content y 0");
        let s = s_for_content(&layout, Side::Old, (first + 1) as f32 * ROW_H, ROW_H, 0.);
        let cap = Viewport::new(&layout, s, H, ROW_H)
            .capture_anchor()
            .expect("anchor on continuation");
        assert_eq!(cap.ln, 2);
        let layout2 = wrap_layout(&text, &text, ops, 50., None);
        assert!(layout2.old.rows() > layout.old.rows());
        let vp1 = Viewport::new(&layout, s, H, ROW_H);
        let Some(Row::Line(at_cap)) = vp1.hit(Side::Old, H / 3.) else {
            panic!("setup: row on anchor");
        };
        assert_eq!(at_cap.part, LinePart::Continuation);

        let s2 = s_for_rewrap(&layout2, cap, H, ROW_H, s).unwrap();
        let vp2 = Viewport::new(&layout2, s2, H, ROW_H);
        let Some(Row::Line(l)) = vp2.hit(Side::Old, H / 3.) else {
            panic!("first row on anchor");
        };
        assert_eq!((l.ln, l.part), (2, LinePart::First));
    }

    #[test]
    fn hit_on_equal_pad_row_returns_logical_line() {
        let old = "a".repeat(25);
        let new = "b".repeat(15);
        let layout = wrap_layout(&old, &new, vec![eq(1, 1, 1)], 100., None);
        let pad_row = layout.new.lines()[2].row as usize;
        let vp = Viewport::new(&layout, 0., VIEW_H, ROW_H);
        let Some(Row::Line(l)) = vp.hit(Side::New, pad_row as f32 * ROW_H + 1.) else {
            panic!("padding row");
        };
        assert!(l.is_equal_padding());
        assert_eq!(l.ln, 1);
    }

    #[test]
    fn match_on_continuation_row_lands_at_center() {
        const H: f32 = 200.;
        // Enough equal lines that a mid-file wrapped continuation can scroll to center.
        let long = "a".repeat(25);
        let text: String = (0..30)
            .map(|i| if i == 14 { long.clone() } else { format!("L{i}") })
            .collect::<Vec<_>>()
            .join("\n");
        let layout = wrap_layout(&text, &text, vec![eq(1, 1, 30)], 100., None);
        let b = layout
            .wrap
            .as_ref()
            .unwrap()
            .old_breaks
            .get(&15)
            .expect("line 15 wraps")
            .breaks[0];
        let row = layout.row_of_match_byte(Side::Old, 15, b).unwrap();
        let s = s_for_match_byte(&layout, Side::Old, 15, b, H, ROW_H, 0.).unwrap();
        let vp = Viewport::new(&layout, s, H, ROW_H);
        let view_y = row as f32 * ROW_H - vp.top(Side::Old);
        assert!(
            (view_y - H / 2.).abs() < 1.,
            "search land should center; got view_y={view_y}, want {}",
            H / 2.
        );
    }

    #[test]
    fn anchor_keeps_place_across_expand_with_wrap() {
        let pad = "x".repeat(30);
        let (old, new, ops) = ten_then_insert();
        let old: String = old
            .lines()
            .map(|l| format!("{l}{pad}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new: String = new
            .lines()
            .map(|l| format!("{l}{pad}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut fold = FoldState::collapsed();
        let collapsed = wrap_layout(&old, &new, ops.clone(), 40., Some(&fold));
        assert!(collapsed.old.rows() > 12, "lines should wrap");
        let row = collapsed.old.row_of_line(8).unwrap() as f32;
        let s = s_for_content(&collapsed, Side::Old, row * ROW_H, ROW_H, 0.);
        let cap = Viewport::new(&collapsed, s, VIEW_H, ROW_H)
            .capture_anchor()
            .expect("anchor");
        let before = view_y(&collapsed, s, Side::Old, 8);
        fold.expand(0);
        let expanded = wrap_layout(&old, &new, ops, 40., Some(&fold));
        let s2 = s_for_anchor(&expanded, cap, VIEW_H, ROW_H, s).unwrap();
        assert_eq!(view_y(&expanded, s2, Side::Old, 8), before);
    }

    #[test]
    fn hunk_target_lands_on_first_visual_row_when_wrapped() {
        const H: f32 = 60.;
        let old = "a\n";
        let new = format!("a\n{}", "b".repeat(25));
        let layout = wrap_layout(
            old,
            &new,
            vec![
                eq(1, 1, 1),
                AlignmentOp::Insert {
                    after_old: 1,
                    news: span(2, 1),
                },
            ],
            100.,
            None,
        );
        let target = layout.hunk_lands[0].target;
        let s = s_for_target(&layout, target, ROW_H, 0.).unwrap();
        let vp = Viewport::new(&layout, s, H, ROW_H);
        let row = layout.new.row_of_line(2).unwrap() as f32;
        assert!((row * ROW_H - vp.top(Side::New) - H / 3.).abs() < 0.01);
    }
}

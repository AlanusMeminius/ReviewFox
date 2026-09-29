//! `DualPaneElement`: old pane | gutter | new pane as one GPUI Element. Prepaint
//! takes the bounds (so `view_h` is this frame's), builds the Viewport and a
//! paint list for the visible rows only; paint draws that list and registers
//! the mouse listeners, which go through Viewport hit tests. See
//! docs/diffview-architecture.md §3–§5 and docs/dual-pane-diff.md §3.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, Entity,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Rgba,
    ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, Window, fill, font, point, px,
    relative, rgb, size,
};

use super::layout::{Layout, LineKind, Row};
use super::pane::DualPane;
use super::tabs::TabExpansion;
use super::trace::{self, FrameStats};
use super::viewport::{Viewport, route_wheel, snap};
use super::visual_wrap::WrapSide;
use super::wrap::{clip_runs_to_display_segment, display_row_segments};
use crate::domain::Side;
use crate::syntax::Span;
use crate::ui::scrollbar::{self, ThumbGeom};
use crate::ui::theme;

pub(super) const LN_FONT_PX: f32 = 10.;
/// Floor for a line-number digit's width: wider than Menlo/Consolas at 10px
/// (~6px) so a digit is never clipped. A wider Code Font uses its own advance.
const LN_DIGIT_PX: f32 = 8.;
/// Line-number inset from the code side of its column.
const LN_PAD: f32 = 4.;
const BRIDGE_COL: f32 = 24.;
/// Code text inset from the pane's inner edge.
pub(super) const TEXT_PAD: f32 = 12.;
/// Left bar on a commented line; the text moves right by the same amount.
pub(super) const COMMENT_BAR: f32 = 2.;
const SEAM_H: f32 = 2.;
const DRAFTING_BG: u32 = 0xdbe4ff;
const SEARCH_HIT_BG: u32 = 0xfff59d;

/// State that changes without touching Layout or the shaped-line cache.
pub(super) struct Decorations {
    pub drafting: Option<(Side, u32)>,
    /// Case-insensitive query for in-file search highlights on visible rows.
    pub search_query: Option<Arc<str>>,
}

/// Shaped text and line number per `(side, visual row)`. Cleared on Layout
/// rebuild and Code Font family / size change; evicted outside visible ± one
/// screen.
#[derive(Default)]
pub(super) struct ShapeCache {
    rows: HashMap<(Side, u32), RowShape>,
}

struct RowShape {
    /// Code text shaped with tabs expanded (display-only).
    text: Option<ShapedLine>,
    /// Maps byte ranges into the original line onto `text`.
    tabs: Option<TabExpansion>,
    label: Option<ShapedLine>,
    text_leading: f32,
}

impl ShapeCache {
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    fn retain(&mut self, keep: &[Range<usize>; 2]) {
        self.rows
            .retain(|&(side, row), _| keep[side_ix(side)].contains(&(row as usize)));
    }
}

/// Scrollbar interaction state for both sides (auto-hide like `scrollbar.rs`).
#[derive(Default)]
pub(super) struct BarState {
    pub visible: bool,
    pub hovered: [bool; 2],
    /// Vertical: side being dragged and grab offset inside the thumb.
    pub drag: Option<(Side, f32)>,
    pub h_hovered: [bool; 2],
    /// Horizontal: side being dragged and grab offset inside the thumb.
    pub h_drag: Option<(Side, f32)>,
}

impl BarState {
    fn shown(&self, side: Side) -> bool {
        self.visible || self.hovered[side_ix(side)] || self.drag.is_some_and(|(s, _)| s == side)
    }

    pub(super) fn h_shown(&self, side: Side) -> bool {
        self.visible || self.h_hovered[side_ix(side)] || self.h_drag.is_some_and(|(s, _)| s == side)
    }

    pub(super) fn any_drag(&self) -> bool {
        self.drag.is_some() || self.h_drag.is_some()
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

    /// Horizontal track along the bottom of the code column (overlay, §3.1.2).
    pub fn h_track(&self, side: Side) -> Bounds<Pixels> {
        let pane = self.pane(side);
        let w = (f32::from(pane.size.width) - scrollbar::PAD * 2.).max(0.);
        Bounds::new(
            point(
                px(f32::from(pane.left()) + scrollbar::PAD),
                px(f32::from(pane.bottom()) - scrollbar::TRACK_WIDTH - scrollbar::PAD),
            ),
            size(px(w), px(scrollbar::TRACK_WIDTH)),
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

/// Usable width for soft-wrap breaks in a code column (§3.1.1).
pub(super) fn code_wrap_width_px(pane_w: f32) -> f32 {
    (pane_w - TEXT_PAD * 2. - COMMENT_BAR).max(0.)
}

pub(super) fn wrap_plan_for_panes(old_w: f32, new_w: f32) -> super::layout::WrapPlan {
    super::layout::WrapPlan {
        old: WrapSide {
            width_px: code_wrap_width_px(old_w),
        },
        new: WrapSide {
            width_px: code_wrap_width_px(new_w),
        },
    }
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

/// `digit_advance`: the Code Font's `'0'` advance at [`LN_FONT_PX`].
pub(super) fn ln_col_width(digits: u32, digit_advance: f32) -> f32 {
    digits as f32 * digit_advance.max(LN_DIGIT_PX) + LN_PAD
}

struct RowPaint {
    y0: f32,
    y1: f32,
    kind_bg: Rgba,
    bg: Rgba,
    commented: bool,
    text: Option<ShapedLine>,
    /// Extra x before shaped text (continuation indent when wrapped).
    text_leading: f32,
    /// Word-mark runs, x relative to the text origin; one rect per run.
    marks: Vec<(f32, f32)>,
    /// Search-hit runs, x relative to the text origin.
    search: Vec<(f32, f32)>,
    label: Option<ShapedLine>,
    show_label: bool,
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
    h_thumb: Option<Thumb>,
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
    pub(super) hitbox: Hitbox,
    pub(super) code: [Hitbox; 2],
    pub(super) tracks: [Option<Hitbox>; 2],
    pub(super) h_tracks: [Option<Hitbox>; 2],
    sides: [SideFrame; 2],
    bridges: Vec<WinBridge>,
    /// Omission separator joins: (old y, new y), window.
    waves: Vec<(f32, f32)>,
    /// Frame-trace numbers (zeros unless `REVIEWFOX_FRAME_TRACE=1`).
    pub(super) stats: FrameStats,
}

pub(super) struct FrameInput<'a> {
    pub layout: &'a Layout,
    /// Per-side highlight spans; `None` means plain text (no language).
    pub highlights: &'a [Option<Arc<[Span]>>; 2],
    pub vp: &'a Viewport<'a>,
    pub geom: Geom,
    pub row_h: f32,
    pub font_px: f32,
    /// Code Font family, for both the code text and the line numbers.
    pub code_family: &'a SharedString,
    pub scale: f32,
    pub decorations: Decorations,
    pub bars: &'a BarState,
}

/// Shape and place the visible rows of both sides; evict far cache entries.
pub(super) fn build_frame(
    input: FrameInput<'_>,
    shapes: &mut ShapeCache,
    window: &mut Window,
) -> Frame {
    let FrameInput {
        layout,
        highlights,
        vp,
        geom,
        row_h,
        font_px,
        code_family,
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
        let search_query = decorations.search_query.as_deref();
        let side_spans = highlights[side_ix(side)].as_deref();
        let mut out = Vec::with_capacity(visible.len());
        let mut widest = 0f32;
        for i in visible {
            let Some(row) = rows.row(i) else { continue };
            let shape = shapes.rows.entry((side, i as u32)).or_insert_with(|| {
                let t = trace::start();
                let shape = shape_row(layout, side, row, side_spans, font_px, code_family, window);
                stats.shaped += 1;
                stats.shape += trace::since(t);
                shape
            });
            let (kind, commented, drafting_here, marks, search, show_label, leading) = match row {
                Row::Line(l) => {
                    let marks = match (layout.mark_runs(side, l), &shape.text, &shape.tabs) {
                        (Some(runs), Some(text), Some(tabs)) => {
                            if layout.wrap.is_some() {
                                let (seg, _) =
                                    wrap_segment(layout, side, l, tabs, layout.side(side));
                                let clipped = clip_runs_to_display_segment(&runs, tabs, seg);
                                run_spans(&clipped, text)
                            } else {
                                mark_spans(&runs, tabs, text)
                            }
                        }
                        _ => Vec::new(),
                    };
                    let pad = l.is_equal_padding();
                    let search = if pad {
                        Vec::new()
                    } else {
                        shape
                            .text
                            .as_ref()
                            .zip(shape.tabs.as_ref())
                            .map(|(text, tabs)| {
                                let line_text = layout.side(side).text(l);
                                let ranges: Vec<_> = search_query
                                    .map(|q| {
                                        crate::domain::match_byte_ranges(line_text, q)
                                            .into_iter()
                                            .map(|r| (r.start, r.end))
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                let (seg, _) =
                                    wrap_segment(layout, side, l, tabs, layout.side(side));
                                let clipped = clip_runs_to_display_segment(&ranges, tabs, seg);
                                run_spans(&clipped, text)
                            })
                            .unwrap_or_default()
                    };
                    (
                        Some(l.kind),
                        !pad && rows.has_comment(l.ln),
                        !pad && drafting == Some(l.ln),
                        marks,
                        search,
                        l.shows_line_number(),
                        shape.text_leading,
                    )
                }
                Row::Omit(_) => (None, false, false, Vec::new(), Vec::new(), false, 0.),
            };
            let kind_bg = kind_bg(kind);
            if let Some(text) = &shape.text {
                widest = widest.max(shape.text_leading + f32::from(text.width));
            }
            out.push(RowPaint {
                y0: y_of(i),
                y1: y_of(i + 1),
                kind_bg,
                bg: if drafting_here {
                    rgb(DRAFTING_BG)
                } else {
                    kind_bg
                },
                commented,
                text: shape.text.clone(),
                text_leading: leading,
                marks,
                search,
                label: shape.label.clone(),
                show_label,
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
            h_thumb: None,
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

    let hitbox = window.insert_hitbox(geom.bounds, HitboxBehavior::Normal);
    let code = [Side::Old, Side::New]
        .map(|side| window.insert_hitbox(geom.pane(side), HitboxBehavior::Normal));
    stats.build = trace::since(t_build);
    Frame {
        geom,
        row_h,
        hitbox,
        code,
        tracks: [None, None],
        h_tracks: [None, None],
        sides,
        bridges,
        waves,
        stats,
    }
}

fn shape_row(
    layout: &Layout,
    side: Side,
    row: Row<'_>,
    spans: Option<&[Span]>,
    font_px: f32,
    family: &SharedString,
    window: &mut Window,
) -> RowShape {
    let Row::Line(line) = row else {
        return RowShape {
            text: None,
            tabs: None,
            label: None,
            text_leading: 0.,
        };
    };
    if line.is_equal_padding() {
        return RowShape {
            text: None,
            tabs: None,
            label: None,
            text_leading: 0.,
        };
    }
    let side_layout = layout.side(side);
    let text = side_layout.text(line);
    let tabs = TabExpansion::new(text);
    let (segment, text_leading) = wrap_segment(layout, side, line, &tabs, side_layout);
    RowShape {
        text: Some({
            let segment_text = tabs.text[segment.clone()].to_string();
            let display = SharedString::from(segment_text);
            match spans {
                Some(spans) => {
                    let line_spans: Vec<_> = crate::syntax::spans_in(spans, line.bytes()).collect();
                    let runs = runs_for_segment(
                        text,
                        &line_spans,
                        &tabs,
                        segment,
                        theme::syntax_colors(),
                        theme::text(),
                    );
                    shape_runs(window, display, family.clone(), font_px, &runs)
                }
                None => shape(window, display, family.clone(), font_px, theme::text()),
            }
        }),
        tabs: Some(tabs),
        label: line.shows_line_number().then(|| {
            shape(
                window,
                SharedString::from(line.ln.to_string()),
                family.clone(),
                LN_FONT_PX,
                theme::faint(),
            )
        }),
        text_leading,
    }
}

fn wrap_segment(
    layout: &Layout,
    side: Side,
    line: &super::layout::LineRow,
    tabs: &TabExpansion,
    side_layout: &super::layout::SideLayout,
) -> (Range<usize>, f32) {
    let full = 0..tabs.text.len();
    let Some(applied) = layout.wrap.as_ref() else {
        return (full, 0.);
    };
    let Some(breaks) = applied.breaks(side, line.ln) else {
        return (full, 0.);
    };
    let segs = display_row_segments(tabs.text.len(), &breaks.breaks);
    let ix = line
        .segment_index(side_layout)
        .min(segs.len().saturating_sub(1));
    let leading = if ix > 0 {
        breaks.continuation_indent_px
    } else {
        0.
    };
    (segs[ix].clone(), leading)
}

fn runs_for_segment(
    line_text: &str,
    spans: &[(Range<usize>, crate::syntax::CaptureId)],
    tabs: &TabExpansion,
    segment: Range<usize>,
    palette: &[Rgba],
    default: Rgba,
) -> Vec<(usize, Rgba)> {
    let display_len = segment.len();
    if display_len == 0 {
        return Vec::new();
    }
    if spans.is_empty() {
        return vec![(display_len, default)];
    }
    let color =
        |id: crate::syntax::CaptureId| palette.get(usize::from(id.0)).copied().unwrap_or(default);
    let mut display_runs = Vec::new();
    for (range, capture) in spans {
        let start = range.start.min(line_text.len());
        let end = range.end.min(line_text.len());
        if end <= start {
            continue;
        }
        let d0 = tabs.display_offset(start);
        let d1 = tabs.display_offset(end);
        if d1 <= d0 {
            continue;
        }
        display_runs.push((d0, d1, color(*capture)));
    }
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (d0, d1, color) in display_runs {
        let start = d0.max(segment.start);
        let end = d1.min(segment.end);
        if end <= start {
            continue;
        }
        let rel0 = start - segment.start;
        let rel1 = end - segment.start;
        if rel0 > cursor {
            out.push((rel0 - cursor, default));
        }
        let from = rel0.max(cursor);
        if rel1 > from {
            out.push((rel1 - from, color));
        }
        cursor = cursor.max(rel1);
    }
    if cursor < display_len {
        out.push((display_len - cursor, default));
    }
    out
}

fn shape(
    window: &mut Window,
    text: SharedString,
    family: SharedString,
    font_px: f32,
    color: Rgba,
) -> ShapedLine {
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

fn shape_runs(
    window: &mut Window,
    text: SharedString,
    family: SharedString,
    font_px: f32,
    runs: &[(usize, Rgba)],
) -> ShapedLine {
    let face = font(family);
    let text_runs: Vec<TextRun> = runs
        .iter()
        .filter(|&&(len, _)| len > 0)
        .map(|&(len, color)| TextRun {
            len,
            font: face.clone(),
            color: color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        })
        .collect();
    window
        .text_system()
        .shape_line(text, px(font_px), &text_runs, None)
}

/// x spans of highlight runs (byte ranges into the original line text,
/// mapped through the tab expansion `text` was shaped from).
fn mark_spans(runs: &[(usize, usize)], tabs: &TabExpansion, text: &ShapedLine) -> Vec<(f32, f32)> {
    runs.iter()
        .map(|&(a, b)| (tabs.display_offset(a), tabs.display_offset(b)))
        .map(|(a, b)| (a.min(text.len()), b.min(text.len())))
        .filter(|(a, b)| b > a)
        .map(|(a, b)| {
            (
                f32::from(text.x_for_index(a)),
                f32::from(text.x_for_index(b)),
            )
        })
        .collect()
}

fn run_spans(runs: &[(usize, usize)], text: &ShapedLine) -> Vec<(f32, f32)> {
    runs.iter()
        .filter(|&&(a, b)| b > a)
        .map(|&(a, b)| {
            (
                f32::from(text.x_for_index(a)),
                f32::from(text.x_for_index(b)),
            )
        })
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

    pub(super) fn set_h_thumb(
        &mut self,
        side: Side,
        geom: super::viewport::HThumbGeom,
        bars: &BarState,
    ) {
        let track = self.geom.h_track(side);
        let inset = (scrollbar::TRACK_WIDTH - scrollbar::THUMB_WIDTH) / 2.;
        let rect = Bounds::new(
            point(track.left() + px(geom.thumb_left), track.top() + px(inset)),
            size(px(geom.thumb_width), px(scrollbar::THUMB_WIDTH)),
        );
        self.sides[side_ix(side)].h_thumb = Some(Thumb {
            rect,
            shown: bars.h_shown(side),
            dragging: bars.h_drag.is_some_and(|(s, _)| s == side),
        });
    }

    fn paint(&self, window: &mut Window, cx: &mut App) {
        let geom = self.geom;
        window.paint_quad(fill(geom.bounds, theme::white()));
        for side in [Side::Old, Side::New] {
            self.paint_code(side, window, cx);
        }
        window.with_content_mask(
            Some(ContentMask {
                bounds: geom.gutter,
            }),
            |window| {
                self.paint_gutter(window, cx);
            },
        );
        paint_omit_waves(window, geom, &self.waves);
        for frame in &self.sides {
            for thumb in frame
                .thumb
                .iter()
                .chain(frame.h_thumb.iter())
                .filter(|t| t.shown)
            {
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
                let mut text_x = x0 + TEXT_PAD - frame.x_offset + row.text_leading;
                if row.commented {
                    text_x += COMMENT_BAR;
                }
                for &(a, b) in &row.search {
                    let rect = hline(
                        text_x + a,
                        text_x + b,
                        row.y0 + (self.row_h - mark_h) / 2.,
                        mark_h,
                    );
                    window.paint_quad(fill(rect, rgb(SEARCH_HIT_BG)).corner_radii(px(2.)));
                }
                for &(a, b) in &row.marks {
                    let rect = hline(
                        text_x + a,
                        text_x + b,
                        row.y0 + (self.row_h - mark_h) / 2.,
                        mark_h,
                    );
                    window.paint_quad(fill(rect, theme::mod_chg()).corner_radii(px(2.)));
                }
                if let Some(text) = &row.text {
                    text.paint(point(px(text_x), px(row.y0)), row_h, window, cx)
                        .ok();
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
                if !row.show_label {
                    continue;
                }
                let Some(label) = &row.label else { continue };
                // Old numbers hug the gutter's inner edge from the left column's
                // right; new numbers start at the right column's left.
                let x = match side {
                    Side::Old => c1 - LN_PAD - f32::from(label.width),
                    Side::New => c0 + LN_PAD,
                };
                label
                    .paint(point(px(x), px(row.y0)), row_h, window, cx)
                    .ok();
            }
        }
    }
}

fn paint_bridges(window: &mut Window, placed: &[WinBridge], ln_w: f32) {
    for bridge in placed {
        let parallel =
            (bridge.y_l0 - bridge.y_r0).abs() < 1. && (bridge.y_l1 - bridge.y_r1).abs() < 1.;
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
        Some(lock) => {
            (x - lock) / WAVE_PERIOD * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2
        }
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
        for track in frame.tracks.iter().chain(frame.h_tracks.iter()).flatten() {
            window.set_cursor_style(CursorStyle::Arrow, track);
        }
        register_listeners(&self.pane, frame, window);
        if t.is_some() {
            trace::frame(&frame.stats, trace::since(t));
        }
    }
}

/// Vertical and horizontal scrollbar tracks when that side overflows.
pub(super) fn insert_scrollbar_hitboxes(
    geom: &Geom,
    v_tracks: [bool; 2],
    h_tracks: [bool; 2],
    window: &mut Window,
) -> ([Option<Hitbox>; 2], [Option<Hitbox>; 2]) {
    let tracks = [Side::Old, Side::New].map(|side| {
        v_tracks[side_ix(side)]
            .then(|| window.insert_hitbox(geom.track(side), HitboxBehavior::Normal))
    });
    let h_tracks = [Side::Old, Side::New].map(|side| {
        h_tracks[side_ix(side)]
            .then(|| window.insert_hitbox(geom.h_track(side), HitboxBehavior::Normal))
    });
    (tracks, h_tracks)
}

fn register_listeners(pane: &Entity<DualPane>, frame: &Frame, window: &mut Window) {
    let geom = frame.geom;
    let hitbox = frame.hitbox.clone();
    let code = frame.code.clone();
    let tracks = frame.tracks.clone();
    let h_tracks = frame.h_tracks.clone();

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
        let (dx, dy) = route_wheel(
            -f32::from(delta.x),
            -f32::from(delta.y),
            event.modifiers.shift,
        );
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
    let down_h_tracks = h_tracks.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        let y = f32::from(event.position.y) - geom.top();
        for side in [Side::Old, Side::New] {
            let ix = side_ix(side);
            if down_tracks[ix]
                .as_ref()
                .is_some_and(|t| t.is_hovered(window))
            {
                let track = geom.track(side);
                let local = f32::from(event.position.y - track.top());
                entity.update(cx, |pane, cx| pane.press_track(side, local, cx));
                return;
            }
        }
        for side in [Side::Old, Side::New] {
            let ix = side_ix(side);
            if down_h_tracks[ix]
                .as_ref()
                .is_some_and(|t| t.is_hovered(window))
            {
                let track = geom.h_track(side);
                let local = f32::from(event.position.x - track.left());
                entity.update(cx, |pane, cx| {
                    if pane.h_bar_shown(side) {
                        pane.press_h_track(side, local, cx);
                    } else {
                        pane.press_row(side, y);
                    }
                });
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
        let hovered = [Side::Old, Side::New].map(|side| {
            tracks[side_ix(side)]
                .as_ref()
                .is_some_and(|t| t.is_hovered(window))
        });
        let h_hovered = [Side::Old, Side::New].map(|side| {
            h_tracks[side_ix(side)]
                .as_ref()
                .is_some_and(|t| t.is_hovered(window))
        });
        let in_gutter = hitbox.is_hovered(window) && geom.gutter.contains(&pos);
        let y = f32::from(pos.y) - geom.top();
        let track_y = [Side::Old, Side::New].map(|side| f32::from(pos.y - geom.track(side).top()));
        let h_track_x =
            [Side::Old, Side::New].map(|side| f32::from(pos.x - geom.h_track(side).left()));
        entity.update(cx, |pane, cx| {
            pane.mouse_moved(
                hovered,
                h_hovered,
                in_gutter.then_some(y),
                track_y,
                h_track_x,
                cx,
            )
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
        // Consolas / Menlo digits (~6px at 10px) keep the 8px floor.
        assert_eq!(ln_col_width(2, 6.), 2. * 8. + LN_PAD);
        assert_eq!(ln_col_width(5, 6.), 5. * 8. + LN_PAD);
        // A wider Code Font widens the column instead of spilling into the code.
        assert_eq!(ln_col_width(3, 9.5), 3. * 9.5 + LN_PAD);
    }

    fn rgba(c: Rgba) -> (u8, u8, u8) {
        (
            (c.r * 255.).round() as u8,
            (c.g * 255.).round() as u8,
            (c.b * 255.).round() as u8,
        )
    }

    #[test]
    fn runs_for_segment_table() {
        use crate::syntax::CaptureId;
        let kw = rgb(0xaa00aa);
        let str_c = rgb(0x00aa00);
        let def = theme::text();
        let palette = [kw, str_c];
        let got = |line: &str, spans: &[(Range<usize>, CaptureId)]| {
            let tabs = TabExpansion::new(line);
            let segment = 0..tabs.text.len();
            runs_for_segment(line, spans, &tabs, segment, &palette, def)
                .into_iter()
                .map(|(len, c)| (len, rgba(c)))
                .collect::<Vec<_>>()
        };
        let kw_c = rgba(kw);
        let str_rgb = rgba(str_c);
        let def_c = rgba(def);

        // Tabs before a token: "\tfn" → four spaces then "fn".
        assert_eq!(
            got("\tfn", &[(1..3, CaptureId(0))]),
            [(4, def_c), (2, kw_c)]
        );
        // Token spanning the whole line.
        assert_eq!(got("fn", &[(0..2, CaptureId(0))]), [(2, kw_c)]);
        // Line with no spans → single default run.
        assert_eq!(got("plain", &[]), [(5, def_c)]);
        // Multibyte chars inside a string span.
        let line = "\"中文\"";
        assert_eq!(line.len(), 8);
        assert_eq!(got(line, &[(0..8, CaptureId(1))]), [(8, str_rgb)]);
    }
}

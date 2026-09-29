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
use super::pane::{DualPane, PointerMove};
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
/// Comment-icon slot beside each line-number column, on the bridge side of it
/// (`old` = number then icon, `new` = icon then number), per the prototype.
const ICON_COL: f32 = 18.;
/// Bubble glyph drawn centred in an [`ICON_COL`] slot.
const ICON_GLYPH: f32 = 12.;
/// Code text inset from the pane's inner edge.
pub(super) const TEXT_PAD: f32 = 12.;
/// Comment marker bar width; hugs the center gutter (Old right / New left).
pub(super) const COMMENT_BAR: f32 = 2.;
const SEAM_H: f32 = 2.;
const DRAFTING_BG: u32 = 0xdbe4ff;
/// Non-current search hit (`--hit` in find-capsule prototype).
const SEARCH_HIT_BG: u32 = 0xffe08a;
/// Current search hit (`--hit-cur`).
const SEARCH_HIT_CUR_BG: u32 = 0xffc53d;
/// Outline on the current hit (`box-shadow: 0 0 0 1px #e6a800`).
const SEARCH_HIT_CUR_RING: u32 = 0xe6a800;

/// Current Diff find occurrence for paint (side + line + byte range).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSearchMatch {
    pub side: Side,
    pub ln: u32,
    pub bytes: std::ops::Range<usize>,
}

/// Which bubble an icon slot carries, and what a click on it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IconMark {
    /// Outline bubble on the selection's start line: click creates a comment.
    Empty,
    /// Solid bubble on an existing DraftComment's start line: click reopens it.
    Filled(u64),
}

/// State that changes without touching Layout or the shaped-line cache.
pub(super) struct Decorations {
    /// Side and inclusive line span of the open DraftComment, if any.
    pub drafting: Option<(Side, u32, u32)>,
    /// Side and inclusive line span of the drag selection, if any.
    pub selection: Option<(Side, u32, u32)>,
    /// Start line and id of every DraftComment on the path, in comment order:
    /// where the filled bubbles go.
    pub comment_starts: Vec<(Side, u32, u64)>,
    /// Case-insensitive query for in-file search highlights on visible rows.
    pub search_query: Option<Arc<str>>,
    /// Side factor: only highlight occurrences on these sides.
    pub search_side: crate::domain::SearchSide,
    /// Active occurrence (darker fill + ring); identity must match byte range.
    pub active_match: Option<ActiveSearchMatch>,
    /// Pulse strength 1→0 over ~250ms after land; `None` = settled.
    pub search_pulse: Option<f32>,
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
        let gutter_w = ln_w * 2. + ICON_COL * 2. + BRIDGE_COL;
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

    /// Width the gutter holds flat on each edge before the bridge curve starts:
    /// the line-number column plus its icon slot.
    pub fn flat_w(&self) -> f32 {
        self.ln_w + ICON_COL
    }

    /// Line-number column of `side`, as window `(left, right)`.
    fn ln_col(&self, side: Side) -> (f32, f32) {
        match side {
            Side::Old => {
                let l = f32::from(self.gutter.left());
                (l, l + self.ln_w)
            }
            Side::New => {
                let r = f32::from(self.gutter.right());
                (r - self.ln_w, r)
            }
        }
    }

    /// Line numbers plus icon slot — the band that carries the row's kind tint
    /// and starts a comment line-selection drag.
    pub(super) fn gutter_band(&self, side: Side) -> (f32, f32) {
        match side {
            Side::Old => {
                let l = f32::from(self.gutter.left());
                (l, l + self.flat_w())
            }
            Side::New => {
                let r = f32::from(self.gutter.right());
                (r - self.flat_w(), r)
            }
        }
    }

    /// Icon slot of `side` over the row band `y0..y1`.
    fn icon_slot(&self, side: Side, y0: f32, y1: f32) -> Bounds<Pixels> {
        let (a, b) = match side {
            Side::Old => {
                let x = f32::from(self.gutter.left()) + self.ln_w;
                (x, x + ICON_COL)
            }
            Side::New => {
                let x = f32::from(self.gutter.right()) - self.flat_w();
                (x, x + ICON_COL)
            }
        };
        Bounds::from_corners(point(px(a), px(y0)), point(px(b), px(y1)))
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

/// COMMENT_BAR x range and left text inset when the bar is shown.
/// Old hugs the pane's right (gutter) edge; New hugs the left (gutter) edge.
pub(super) fn comment_bar_layout(side: Side, pane_left: f32, pane_right: f32) -> (f32, f32, f32) {
    match side {
        Side::Old => (pane_right - COMMENT_BAR, pane_right, 0.),
        Side::New => (pane_left, pane_left + COMMENT_BAR, COMMENT_BAR),
    }
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
    /// Inside the drag selection: the code column takes a translucent wash.
    selected: bool,
    text: Option<ShapedLine>,
    /// Extra x before shaped text (continuation indent when wrapped).
    text_leading: f32,
    /// Word-mark runs, x relative to the text origin; one rect per run.
    marks: Vec<(f32, f32)>,
    /// Search-hit runs, x relative to the text origin; `true` = current hit.
    search: Vec<(f32, f32, bool)>,
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
    /// Visible icon slots on this side, as row bands.
    icons: Vec<IconPlace>,
    thumb: Option<Thumb>,
    h_thumb: Option<Thumb>,
    /// Widest shaped text among this frame's rows.
    widest: f32,
    /// Horizontal scroll of the code text, device-pixel snapped. Set after
    /// the frame is built, once `widest` has fed the clamp.
    x_offset: f32,
}

/// An icon slot before its side is turned into gutter x, as a row band.
struct IconPlace {
    y0: f32,
    y1: f32,
    mark: IconMark,
    /// The draft dock is open on this line, so the bubble reads active.
    open: bool,
}

/// A placed icon slot: what to paint, and the rect a click must hit.
pub(super) struct IconSlot {
    pub(super) slot: Bounds<Pixels>,
    pub(super) mark: IconMark,
    open: bool,
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
    /// Pulse strength for the current search hit (1→0); None = settled.
    search_pulse: Option<f32>,
    pub(super) hitbox: Hitbox,
    pub(super) code: [Hitbox; 2],
    pub(super) tracks: [Option<Hitbox>; 2],
    pub(super) h_tracks: [Option<Hitbox>; 2],
    /// Comment icons on this frame's visible rows, in paint order.
    pub(super) icons: Vec<IconSlot>,
    /// One per [`Self::icons`], same order; filled by the pane's prepaint.
    pub(super) icon_hitboxes: Vec<Hitbox>,
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
    let search_pulse = decorations.search_pulse;
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
        let span_on_side = |span: Option<(Side, u32, u32)>| {
            span.and_then(|(s, start, end)| (s == side).then_some(start..=end))
        };
        let drafting = span_on_side(decorations.drafting);
        let selection = span_on_side(decorations.selection);
        let search_query = decorations.search_query.as_deref();
        let search_side = decorations.search_side;
        let active_match = decorations.active_match.as_ref();
        let side_spans = highlights[side_ix(side)].as_deref();
        let side_ok = match search_side {
            crate::domain::SearchSide::Old => side == Side::Old,
            crate::domain::SearchSide::New => side == Side::New,
            crate::domain::SearchSide::Both => true,
        };
        let mut out = Vec::with_capacity(visible.len());
        let mut widest = 0f32;
        let mut icons = Vec::new();
        for i in visible {
            let Some(row) = rows.row(i) else { continue };
            let shape = shapes.rows.entry((side, i as u32)).or_insert_with(|| {
                let t = trace::start();
                let shape = shape_row(layout, side, row, side_spans, font_px, code_family, window);
                stats.shaped += 1;
                stats.shape += trace::since(t);
                shape
            });
            // Selection is line-granular, so it needs no shaped text; the
            // icon rides the start line's first visual row.
            let selected_here = match row {
                Row::Line(l) if !l.is_equal_padding() => {
                    let sel = selection.as_ref().filter(|s| s.contains(&l.ln));
                    if l.shows_line_number() {
                        let mark = icon_mark_for(
                            &decorations.comment_starts,
                            side,
                            l.ln,
                            sel.is_some_and(|s| *s.start() == l.ln),
                        );
                        if let Some(mark) = mark {
                            icons.push(IconPlace {
                                y0: y_of(i),
                                y1: y_of(i + 1),
                                mark,
                                open: drafting.as_ref().is_some_and(|d| *d.start() == l.ln),
                            });
                        }
                    }
                    sel.is_some()
                }
                _ => false,
            };
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
                    let search = if pad || !side_ok {
                        Vec::new()
                    } else {
                        shape
                            .text
                            .as_ref()
                            .zip(shape.tabs.as_ref())
                            .zip(search_query)
                            .map(|((text, tabs), q)| {
                                let line_text = layout.side(side).text(l);
                                let (seg, _) =
                                    wrap_segment(layout, side, l, tabs, layout.side(side));
                                let mut hits = Vec::new();
                                for r in crate::domain::match_byte_ranges(line_text, q) {
                                    let is_cur = active_match.is_some_and(|m| {
                                        m.side == side
                                            && m.ln == l.ln
                                            && m.bytes.start == r.start
                                            && m.bytes.end == r.end
                                    });
                                    let clipped = clip_runs_to_display_segment(
                                        &[(r.start, r.end)],
                                        tabs,
                                        seg.clone(),
                                    );
                                    for (a, b) in run_spans(&clipped, text) {
                                        hits.push((a, b, is_cur));
                                    }
                                }
                                hits
                            })
                            .unwrap_or_default()
                    };
                    (
                        Some(l.kind),
                        !pad && rows.has_comment(l.ln),
                        !pad && drafting.as_ref().is_some_and(|d| d.contains(&l.ln)),
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
                selected: selected_here,
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
            icons,
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
    let icons = [Side::Old, Side::New]
        .into_iter()
        .flat_map(|side| {
            sides[side_ix(side)]
                .icons
                .iter()
                .map(move |place| IconSlot {
                    slot: geom.icon_slot(side, place.y0, place.y1),
                    mark: place.mark,
                    open: place.open,
                })
        })
        .collect();
    stats.build = trace::since(t_build);
    Frame {
        geom,
        row_h,
        search_pulse,
        hitbox,
        code,
        tracks: [None, None],
        h_tracks: [None, None],
        icons,
        icon_hitboxes: Vec::new(),
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

/// Which DraftComment the filled bubble on `(side, ln)` belongs to. Overlapping
/// comments are allowed, so two can start on the same line; `starts` is in
/// comment order, and taking the first match keeps the choice stable across
/// frames (it is also the oldest comment, since ids ascend).
fn comment_start_id(starts: &[(Side, u32, u64)], side: Side, ln: u32) -> Option<u64> {
    starts
        .iter()
        .find(|&&(s, start, _)| s == side && start == ln)
        .map(|&(_, _, id)| id)
}

/// The one bubble `(side, ln)` can show. A line has a single slot, so a comment
/// starting there takes it: reopening an existing comment must stay reachable
/// even when a new selection starts on the same line.
fn icon_mark_for(
    starts: &[(Side, u32, u64)],
    side: Side,
    ln: u32,
    selection_starts_here: bool,
) -> Option<IconMark> {
    comment_start_id(starts, side, ln)
        .map(IconMark::Filled)
        .or(selection_starts_here.then_some(IconMark::Empty))
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
                if row.selected {
                    window.paint_quad(fill(
                        hline(x0, x1, row.y0, row.y1 - row.y0),
                        theme::selection_wash(),
                    ));
                }
                let (bar_x0, bar_x1, bar_text_inset) = comment_bar_layout(side, x0, x1);
                let mut text_x = x0 + TEXT_PAD - frame.x_offset + row.text_leading;
                if row.commented {
                    text_x += bar_text_inset;
                }
                for &(a, b, is_cur) in &row.search {
                    let y = row.y0 + (self.row_h - mark_h) / 2.;
                    let rect = hline(text_x + a, text_x + b, y, mark_h);
                    if is_cur {
                        // Pulse: briefly enlarge ring, then settle on hit-cur + 1px ring.
                        let pulse = self.search_pulse.unwrap_or(0.);
                        let ring = 1. + pulse * 2.;
                        let ring_rect = hline(
                            text_x + a - ring,
                            text_x + b + ring,
                            y - ring,
                            mark_h + ring * 2.,
                        );
                        window.paint_quad(
                            fill(ring_rect, rgb(SEARCH_HIT_CUR_RING)).corner_radii(px(2. + ring)),
                        );
                        window.paint_quad(fill(rect, rgb(SEARCH_HIT_CUR_BG)).corner_radii(px(2.)));
                    } else {
                        window.paint_quad(fill(rect, rgb(SEARCH_HIT_BG)).corner_radii(px(2.)));
                    }
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
                // Row marker, pinned to the gutter-facing pane edge above scrolled text.
                if row.commented {
                    window.paint_quad(fill(
                        hline(bar_x0, bar_x1, row.y0, row.y1 - row.y0),
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
        window.paint_quad(fill(g, theme::white()));
        for side in [Side::Old, Side::New] {
            // The kind tint runs under the icon slot as well, so the bridge
            // leaves a flat edge past both.
            let (c0, c1) = self.geom.gutter_band(side);
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
        paint_bridges(window, &self.bridges, self.geom.flat_w());
        let row_h = px(self.row_h);
        for side in [Side::Old, Side::New] {
            let (c0, c1) = self.geom.ln_col(side);
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
        for icon in &self.icons {
            paint_comment_icon(window, icon);
        }
    }
}

/// A comment bubble in its icon slot — the prototype's `ICON_EMPTY` outline or
/// `ICON_FILLED` solid, scaled from their 16-unit viewBox into an [`ICON_GLYPH`]
/// box centred in the slot. An existing comment reads accent like its
/// [`COMMENT_BAR`]; `open` adds the range tint while the dock is on this line.
fn paint_comment_icon(window: &mut Window, icon: &IconSlot) {
    let slot = icon.slot;
    let unit = ICON_GLYPH / 16.;
    let x0 = f32::from(slot.left()) + (f32::from(slot.size.width) - ICON_GLYPH) / 2.;
    let y0 = f32::from(slot.top()) + (f32::from(slot.size.height) - ICON_GLYPH) / 2.;
    let at = |x: f32, y: f32| point(px(x0 + x * unit), px(y0 + y * unit));
    if icon.open {
        window.paint_quad(fill(slot, theme::range()).corner_radii(px(4.)));
    }
    match icon.mark {
        IconMark::Empty => {
            let mut stroke = PathBuilder::stroke(px((1.5 * unit).max(1.)));
            stroke.move_to(at(3.5, 3.5));
            for (x, y) in [
                (12.5, 3.5),
                (12.5, 10.7),
                (8.2, 10.7),
                (5.5, 13.),
                (5.5, 10.7),
                (3.5, 10.7),
                (3.5, 3.5),
            ] {
                stroke.line_to(at(x, y));
            }
            if let Ok(path) = stroke.build() {
                window.paint_path(
                    path,
                    if icon.open {
                        theme::accent()
                    } else {
                        theme::muted()
                    },
                );
            }
        }
        IconMark::Filled(_) => {
            let mut bubble = PathBuilder::fill();
            bubble.move_to(at(3., 3.));
            for (x, y) in [
                (13., 3.),
                (13., 10.5),
                (8.1, 10.5),
                (5., 13.2),
                (5., 10.5),
                (3., 10.5),
            ] {
                bubble.line_to(at(x, y));
            }
            bubble.close();
            if let Ok(path) = bubble.build() {
                window.paint_path(path, theme::accent());
            }
            // Two text rules knocked out of the bubble, so a filled icon reads
            // as a written comment rather than a solid blob.
            for (x1, y) in [(10.8, 6.2), (9., 8.2)] {
                let mut stroke = PathBuilder::stroke(px((1.2 * unit).max(0.75)));
                stroke.move_to(at(5.2, y));
                stroke.line_to(at(x1, y));
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, theme::white());
                }
            }
        }
    }
}

fn paint_bridges(window: &mut Window, placed: &[WinBridge], flat_w: f32) {
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
            pinch_bezier(&mut path, bridge, flat_w);
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, kind_bg(Some(bridge.kind)));
        }
    }
}

/// Full row on the long side, cubic Bézier through the center gutter, then a 2px
/// hairline across the short side's line-number and icon columns (`flat_w`) so
/// it meets the code hairline.
fn pinch_bezier(path: &mut PathBuilder, bridge: &WinBridge, flat_w: f32) {
    let x_l = bridge.x_l;
    let x_r = bridge.x_r;
    let mid_l = x_l + flat_w;
    let mid_r = x_r - flat_w;
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
            let flat = geom.flat_w();
            let path = joined_wave(x0, x1, y_l, gutter_l + flat, gutter_r - flat, y_r);
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
            window.set_cursor_style(CursorStyle::IBeam, code);
        }
        for icon in &frame.icon_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, icon);
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
    let down_hitbox = hitbox.clone();
    let down_icons: Vec<(Hitbox, IconMark)> = frame
        .icon_hitboxes
        .iter()
        .cloned()
        .zip(frame.icons.iter().map(|icon| icon.mark))
        .collect();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
            return;
        }
        let y = f32::from(event.position.y) - geom.top();
        if let Some(&(_, mark)) = down_icons
            .iter()
            .find(|(hitbox, _)| hitbox.is_hovered(window))
        {
            entity.update(cx, |pane, cx| pane.click_comment_icon(mark, cx));
            return;
        }
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
                        // Bar hidden: still allow omit-expand under the track strip.
                        pane.press_code(side, y, cx);
                    }
                });
                return;
            }
        }
        // Comment line selection: center gutter band (ln + icon) only.
        if down_hitbox.is_hovered(window) {
            let x = f32::from(event.position.x);
            for side in [Side::Old, Side::New] {
                let (l, r) = geom.gutter_band(side);
                if x >= l && x < r {
                    entity.update(cx, |pane, cx| pane.press_gutter_select(side, y, cx));
                    return;
                }
            }
        }
        // Code column: omit-expand click only — never starts a line selection.
        for side in [Side::Old, Side::New] {
            if down_code[side_ix(side)].is_hovered(window) {
                entity.update(cx, |pane, cx| pane.press_code(side, y, cx));
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
                PointerMove {
                    hovered,
                    h_hovered,
                    gutter_y: in_gutter.then_some(y),
                    pane_y: y,
                    track_y,
                    h_track_x,
                },
                cx,
            )
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_bar_hugs_the_center_gutter() {
        let (left, right) = (100., 400.);
        let (old_x0, old_x1, old_inset) = comment_bar_layout(Side::Old, left, right);
        assert_eq!((old_x0, old_x1), (right - COMMENT_BAR, right));
        assert_eq!(
            old_inset, 0.,
            "Old bar is on the right; text needs no left inset"
        );
        let (new_x0, new_x1, new_inset) = comment_bar_layout(Side::New, left, right);
        assert_eq!((new_x0, new_x1), (left, left + COMMENT_BAR));
        assert_eq!(
            new_inset, COMMENT_BAR,
            "New bar is on the left; text clears it"
        );
    }

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
    fn filled_icon_belongs_to_the_comment_starting_on_the_line() {
        // Two comments on `new`, one on `old`; the middle one spans 8..=10.
        let starts = [(Side::New, 3, 1), (Side::New, 8, 2), (Side::Old, 3, 3)];
        assert_eq!(comment_start_id(&starts, Side::New, 3), Some(1));
        assert_eq!(comment_start_id(&starts, Side::Old, 3), Some(3));
        assert_eq!(comment_start_id(&starts, Side::New, 8), Some(2));
        // Inside a span but not its start: no icon, only the COMMENT_BAR.
        assert_eq!(comment_start_id(&starts, Side::New, 9), None);
        assert_eq!(comment_start_id(&starts, Side::Old, 8), None);
        assert_eq!(comment_start_id(&[], Side::New, 3), None);
    }

    #[test]
    fn overlapping_starts_pick_the_first_comment() {
        let starts = [(Side::New, 4, 7), (Side::New, 4, 9)];
        assert_eq!(comment_start_id(&starts, Side::New, 4), Some(7));
    }

    #[test]
    fn one_slot_per_line_goes_to_the_comment_over_the_selection() {
        let starts = [(Side::New, 5, 1)];
        // Selection start with nothing on it: the offer to create one.
        assert_eq!(
            icon_mark_for(&starts, Side::New, 9, true),
            Some(IconMark::Empty)
        );
        // A comment's start line keeps its filled bubble, selection or not.
        assert_eq!(
            icon_mark_for(&starts, Side::New, 5, false),
            Some(IconMark::Filled(1))
        );
        assert_eq!(
            icon_mark_for(&starts, Side::New, 5, true),
            Some(IconMark::Filled(1)),
            "filled wins the slot when the selection starts on a comment's start"
        );
        // Neither: an ordinary line has no icon at all.
        assert_eq!(icon_mark_for(&starts, Side::New, 9, false), None);
        // Same line number on the other side is a different place.
        assert_eq!(icon_mark_for(&starts, Side::Old, 5, false), None);
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

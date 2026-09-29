# Dual-pane Diff — Architecture

Status: settled in grill (2026-09-27). Behavior spec stays `docs/dual-pane-diff.md` §3; this doc only restructures how it is computed and drawn. Decision record: ADR-0008. Work items: `.scratch/diffview/issues/`.

## 1. Why

The GPUI port draws for the sake of drawing and is slow:

- Every frame builds a div per row for both code panes plus four line-number columns (≈6 elements per line), with no viewport culling.
- Every frame clones `old_rows` / `new_rows` / `knots` / `bridges`, reruns intra-line LCS for every Replace (`replace_side_marks`), scans comments per row (`has_comment`), scans seams per row, and pairs omit separators in O(N²).
- Scroll is a round trip: native `overflow_y_scroll` moves, `sync_scroll` reads the offset back a frame later, diffs against `applied_*`, reverse-maps to `scroll_s`, writes `set_offset`. `view_h` is measured in a canvas and costs one more `notify`.
- `DiffView` holds 30+ fields mixing UI state, row-unit geometry (domain knots), pixel geometry, and window-space caches (`placed`, `hover_bands`, `omit_links`). `RowKind::Omit` lives inside the line list, so `!matches!(…Omit)` is sprinkled everywhere.
- **First-scroll hitch** (any file size): `scroll_s` starts at 0 but each side's top is `(content_y − view_h/3).clamp(0, max)`, so the first ≈view_h/3 px of wheel travel moves nothing. Same dead zone recurs at the top and, symmetrically, the bottom.
- macOS only: GPUI rasterizes glyphs for every painted row before culling; the first fractional scroll adds sub-pixel variants for the whole file.

## 2. Goals

- Behavior unchanged (§3 of `dual-pane-diff.md` is the spec).
- Per-frame work proportional to **visible rows**, not file size. Baseline: 20k-line file scrolls at 120 fps.
- No wheel travel without visible motion (`s_range`).

## 3. Layers

Split by change frequency. Lower-frequency data never sits on the per-frame path.

| Layer | File | Inputs | Outputs | Rebuilt when |
|---|---|---|---|---|
| **Layout** (pure) | `src/ui/diff/layout.rs` | `Arc<str>` preimage/postimage text, `Alignment`, `FoldState`, optional soft wrap (per-side width + char width fn) | Per-side visual rows (a logical line may span several when wrapped; Equal pairs padded to the same count) (line rows and omit separators modeled separately, not a `RowKind`), bridges, knots, hunk lands, seam and comment row indices, lazy per-Replace word marks. Line text is a byte range into the shared text. | Alignment / fold / ignore-whitespace change; with wrap on, also wrap width / font change |
| **Viewport** (pure) | `src/ui/diff/viewport.rs` | Layout, `scroll_s`, `view_h`, `row_h`, device scale (the per-side `x_offset` is applied at paint; `route_wheel` / `max_x` / `clamp_x` are free fns here) | `s_range`, per-side top (device-pixel snapped), visible row ranges, pixel bridges / gaps / omit links, hit testing `hit(side, y)` / `bridge_at(y)` | Every frame (cheap, visible-only) |
| **Render** | `src/ui/diff/element.rs`, `pane.rs` | Layout, Viewport, `Decorations` | Paint only | — |

Layout and Viewport do not depend on GPUI and are unit-tested. `interp`, `s_from`, `track`, `place_bridge`, `gap_intervals`, `empty_seam`, `content_pad` move into Viewport.

### 3.1 Decorations

Drafting LineSpan, drag selection, commented lines (plus each comment's start line and id, for the gutter's filled bubbles), search hits, hovered bridge. Passed to the element separately so changing them never invalidates Layout or shaped-text cache.

### 3.2 Word marks

Computed lazily per Replace block the first time it becomes visible, memoized on Layout. Opening a file never pays for LCS it does not show.

## 4. Scroll model

- `scroll_s` is the **only** vertical scroll source. Wheel / trackpad (incl. momentum), scrollbar drag, hunk jump, match jump, anchor restore after fold all write `scroll_s`. No native `ScrollHandle` on the panes; `applied_*`, `scroll_nudge` go away.
- `scroll_s` is clamped to `Viewport::s_range()`: lower bound = first `s` where either side's top leaves 0; upper bound = first `s` where both sides sit at their max. Outside that range the picture is identical, so clamping changes nothing visible and removes the dead zone.
- Each side's top is snapped to device pixels; bridge endpoints follow.
- Horizontal (updated 2026-09-29, `docs/dual-pane-diff.md` §3.1.2; replaces issue 04's "independent, no scrollbar"): with sync on, one shared offset bounded by the larger side's `max_x`; with sync off, the per-side behavior below. Each side paints a horizontal scrollbar when it overflows. Soft wrap (§3.1.1) forces the offset to 0 and hides the bars.
- Horizontal, unsynced: per-side `x_offset`, independent, driven only by horizontal input over that pane (shift+wheel / trackpad X). One axis per wheel event (`viewport::route_wheel`: shift with no X maps Y→X, else the dominant axis wins), so a horizontal swipe never moves `scroll_s`. Bound: `0..=max_x`, where the widest line is the whole side's longest shown line (display columns × mono advance, tabs expanded to 4-column stops, raised by any wider shaped line seen), so it does not jump while scrolling. Only code text and word marks shift; row backgrounds, comment bars, line numbers, gutter, gaps, seams, waves and scrollbars stay. Reset on file open; kept and re-clamped on fold, Alignment, font size and resize.
- Scrollbars are painted by the element: old on the outer left, new on the outer right (ADR-0003). Dragging a side's thumb maps that side's content position back to `scroll_s`; the other side follows §3.1. Styling reuses `scrollbar.rs` theme constants, not its lockstep.

## 5. Entities and data flow

```
DiffView (shell: tree, chrome, find bar, draft dock, comments, Review)
  │  methods: select_file, jump_hunk, expand_all, set_font, set_view_options…
  │  events ◄─ OpenDraft{side, start, count}, OpenEdit{id}, SelectionStarted, HunkIndexChanged, HoverCopy
  ▼
DualPane (Entity) ── owns FileDiffState { alignment, fold, layout, scroll_s, x_offsets, shape cache }
  │  render → DualPaneElement(layout, decorations)
  ▼
DualPaneElement (one Element: old pane | gutter | new pane)
  prepaint: bounds → view_h → Viewport (same frame)
  paint: visible rows only, bridges, gaps/hatch, omit waves, line numbers, scrollbars
  mouse: forwarded to Viewport hit tests
```

- Scroll notifies `DualPane` only; the tree, chrome and comments do not re-render.
- GPUI re-renders every ancestor of a notified view, so `DualPane` is not nested in the shell. `DiffView` renders a cached `DiffShell` view (tree, chrome, search, comments) that leaves a slot, and mounts the cached `DualPane` over that slot (`pane::slot`). A pane frame re-renders only `DiffView`'s thin root and the pane; a shell change reuses the pane.
- `git::FileDiff::Text` carries text + `Alignment` only (no `display`, no `hunk_count`). `DisplayRows`, `ScrollKnot`, `Bridge`, `RowKind` move out of `domain` into `ui/diff/layout.rs`; `domain` keeps Alignment, Hunk, FoldState, Search, DraftComment.

## 6. Caches and invalidation

| Cache | Key | Invalidated by |
|---|---|---|
| Layout | (alignment, fold) | fold / ignore-whitespace / file change |
| Word marks | Replace block index | Layout rebuild |
| Shaped lines | (side, visual row) | Layout rebuild, font size change, Code Font family change; evicted outside visible ± one screen |
| Mono advances (code text, line-number digit) | font px / — | Code Font family change (code-text advance also on font size change) |
| Soft-wrap breaks + visual row counts | part of Layout (one `Layout::build` entry, so a fold rebuild cannot drop wrap) | Layout rebuild, which wrap also triggers: pane width change, font size / family change, wrap toggle |
| Viewport | — | never cached; recomputed each frame |

## 7. Verification

- Table-driven unit tests for Layout and Viewport: insert at file start / end, delete, 3→1 replace, one side empty, anchor preserved across expand / collapse, end-of-file gap, `s_range` has no dead travel (every step inside the range moves at least one side).
- Render layer checked by running the app.
- `REVIEWFOX_FRAME_TRACE=1` (off by default, `src/ui/diff/trace.rs`) prints to stderr one line per Layout build and one per drawn pane frame: Viewport, frame build, newly shaped rows + shaping time, prepaint, paint, visible rows.
- Headless: `cargo test --release -- --ignored frame_cost_is_flat_in_file_length --nocapture` (`src/ui/diff/perf.rs`) times Layout build and a full-range scroll of Viewport + visible-row paint list (no shaping) on synthetic 2k and 20k-line files, and asserts the per-frame cost stays flat.
- Headless: `cargo test --release -- --ignored rewrap_cost_report --nocapture` reports a full soft-wrap `Layout::build` on a synthetic 20k-line file (folded and expanded) against the wrap-off build. Rewrap misses the 8 ms frame budget (~20 ms folded, ~48 ms expanded at 640 px), so a resize drag keeps the previous wrap and rewraps once the width settles.

## 8. Migration

Incremental; each step leaves the app working.

1. Extract Layout + Viewport with tests; add `s_range` clamp. UI still divs, but stops cloning and re-running LCS per frame.
2. Split `DualPane` Entity out of `DiffView`.
3. `DualPaneElement`: visible-only paint, shaped-line cache, painted scrollbars, prepaint `view_h`, pixel snapping.
4. Per-side horizontal scroll.
5. Frame trace; delete dead fields / functions.

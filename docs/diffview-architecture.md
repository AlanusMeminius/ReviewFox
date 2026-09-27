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
| **Layout** (pure) | `src/ui/diff/layout.rs` | `Arc<str>` old/new text, `Alignment`, `FoldState` | Per-side visual lines (line rows and omit separators modeled separately, not a `RowKind`), bridges, knots, hunk lands, seam and comment row indices, lazy per-Replace word marks. Line text is a byte range into the shared text. | Alignment / fold / ignore-whitespace change |
| **Viewport** (pure) | `src/ui/diff/viewport.rs` | Layout, `scroll_s`, `view_h`, `row_h`, per-side `x_offset`, device scale | `s_range`, per-side top (device-pixel snapped), visible row ranges, pixel bridges / gaps / omit links, hit testing `hit(side, y)` / `bridge_at(y)` | Every frame (cheap, visible-only) |
| **Render** | `src/ui/diff/element.rs`, `pane.rs` | Layout, Viewport, `Decorations` | Paint only | — |

Layout and Viewport do not depend on GPUI and are unit-tested. `interp`, `s_from`, `track`, `place_bridge`, `gap_intervals`, `empty_seam`, `content_pad` move into Viewport.

### 3.1 Decorations

Drafting line, commented lines, search hits, hovered bridge. Passed to the element separately so changing them never invalidates Layout or shaped-text cache.

### 3.2 Word marks

Computed lazily per Replace block the first time it becomes visible, memoized on Layout. Opening a file never pays for LCS it does not show.

## 4. Scroll model

- `scroll_s` is the **only** vertical scroll source. Wheel / trackpad (incl. momentum), scrollbar drag, hunk jump, match jump, anchor restore after fold all write `scroll_s`. No native `ScrollHandle` on the panes; `applied_*`, `scroll_nudge` go away.
- `scroll_s` is clamped to `Viewport::s_range()`: lower bound = first `s` where either side's top leaves 0; upper bound = first `s` where both sides sit at their max. Outside that range the picture is identical, so clamping changes nothing visible and removes the dead zone.
- Each side's top is snapped to device pixels; bridge endpoints follow.
- Horizontal: per-side `x_offset`, independent, driven only by horizontal input over that pane (shift+wheel / trackpad X).
- Scrollbars are painted by the element: old on the outer left, new on the outer right (ADR-0003). Dragging a side's thumb maps that side's content position back to `scroll_s`; the other side follows §3.1. Styling reuses `scrollbar.rs` theme constants, not its lockstep.

## 5. Entities and data flow

```
DiffView (shell: tree, chrome, search bar, draft bar, Review)
  │  methods: select_file, jump_hunk, expand_all, set_font, set_view_options…
  │  events ◄─ BeginDraft{side, ln}, HunkIndexChanged, HoverCopy
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
- `git::FileDiff::Text` carries text + `Alignment` only (no `display`, no `hunk_count`). `DisplayRows`, `ScrollKnot`, `Bridge`, `RowKind` move out of `domain` into `ui/diff/layout.rs`; `domain` keeps Alignment, Hunk, FoldState, Search, DraftComment.

## 6. Caches and invalidation

| Cache | Key | Invalidated by |
|---|---|---|
| Layout | (alignment, fold) | fold / ignore-whitespace / file change |
| Word marks | Replace block index | Layout rebuild |
| Shaped lines | (side, visual row) | Layout rebuild, font size change; evicted outside visible ± one screen |
| Viewport | — | never cached; recomputed each frame |

## 7. Verification

- Table-driven unit tests for Layout and Viewport: insert at file start / end, delete, 3→1 replace, one side empty, anchor preserved across expand / collapse, end-of-file gap, `s_range` has no dead travel (every step inside the range moves at least one side).
- Render layer checked by running the app.
- `REVIEWFOX_FRAME_TRACE=1` (off by default) logs per-frame Layout / Viewport / shape / paint time and visible row count.

## 8. Migration

Incremental; each step leaves the app working.

1. Extract Layout + Viewport with tests; add `s_range` clamp. UI still divs, but stops cloning and re-running LCS per frame.
2. Split `DualPane` Entity out of `DiffView`.
3. `DualPaneElement`: visible-only paint, shaped-line cache, painted scrollbars, prepaint `view_h`, pixel snapping.
4. Per-side horizontal scroll.
5. Frame trace; delete dead fields / functions.

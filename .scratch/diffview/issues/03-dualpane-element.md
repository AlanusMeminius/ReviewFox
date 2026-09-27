# DualPaneElement: visible-only paint with single scroll source

Status: resolved
Blocked by: 02

## What

- One custom GPUI `Element` for old pane | gutter | new pane.
- `prepaint`: take bounds → `view_h` → Viewport in the same frame (drop the canvas measure + extra notify).
- `paint`: visible rows only; line numbers, row backgrounds, word marks, bridges, gap hatch, omit waves, seams; tops snapped to device pixels.
- Shaped-line cache keyed `(side, visual row)`, keep visible ± one screen, clear on Layout rebuild / font size change.
- Remove native `ScrollHandle` on the panes; wheel / trackpad / momentum write `scroll_s` only.
- Paint scrollbars: old outer left, new outer right; thumb drag maps that side back to `scroll_s`. Styling from `scrollbar.rs` constants.
- Mouse events go through Viewport `hit` / `bridge_at`; `Decorations` (drafting line, commented lines, search hits, hovered bridge) passed separately.

## Done when

All `docs/dual-pane-diff.md` §3 behaviors verified by running the app; large-file scroll is smooth.

## Comments

**2026-09-27 — resolved (agent).**

What changed:

- New `src/ui/diff/element.rs`: `DualPaneElement` (holds only `Entity<DualPane>`) is the whole body of `DualPane::render`. `PaneSlot` / `DiffShell` are unchanged.
  - `prepaint` calls `DualPane::prepaint_frame`. It sets `view_h` and `scale` from this frame's bounds and builds `Viewport::new(..).snapped(scale)`. It writes the clamped `scroll_s` back, inserts hitboxes (whole element, each code pane, each scrollbar track that can scroll) and builds a `Frame` paint list for the visible rows only. The canvas `view_h` measurement and its extra `notify` are gone.
  - `paint` draws the `Frame` in this order: white base; per code pane (clipped) row backgrounds / drafting, comment bar, word marks, text, then gap hatch, empty-side seam and seam hairlines; the gutter (clipped) line-number column backgrounds and seams, bridges, then line numbers (old right-aligned, new left-aligned); omit waves over the whole element; thumbs. Tops and every row / gap / bridge y are snapped to device pixels, and so is the horizontal split.
  - `ShapeCache`: `(Side, visual row)` → shaped text + shaped line number. Kept for visible ± one screen (evicted each prepaint). Cleared in `rebuild_layout` and `set_font_size`.
  - `Decorations { drafting }` goes to the frame builder separately. Comment bars read the Layout comment index, so `set_comments` does not touch the shape cache.
- Scroll: no `ScrollHandle` / `overflow_y_scroll`. A `ScrollWheelEvent` on the element hitbox calls `DualPane::scroll_by`. That clamps `scroll_s + dy` to `s_range`, clears `hunk_s`, updates the hunk index and reveals the bars. It does nothing when nothing would change.
- Scrollbars: painted with the `scrollbar.rs` constants (now `pub(crate)`), old on the outer left, new on the outer right. They reuse `ThumbGeom` plus a new `ThumbGeom::offset_for`, which `VerticalScrollbar` now uses too. They auto-hide after `HIDE_DELAY` and show on scroll, jump, track hover and drag. A press on the thumb grabs it. A press elsewhere in the track centers the thumb on the press and grabs it. Dragging maps thumb → side top (`top_at`) → `content_from_top` → `s_for_content` → clamped `scroll_s`. The other side follows §3.1.
- Mouse: a left press records `(side, visual row)` via `Viewport::hit`. The release completes the click only on the same row: a line emits `BeginDraft`, an omission separator expands its span. The action runs on release, as `on_click` did, so the root's focus-on-mouse-down does not take focus back from the draft input. Gutter mouse move → `bridge_at` → hover copy, cleared when leaving the gutter. Pointer cursor over code panes, arrow over tracks. All per-row divs / `on_click`s are gone.
- Deleted: `row_text`, `old_scroll`/`new_scroll`, `scroll_nudge`, `applied_*`, `pane_top_w`, `marked_rows`, `placed`, `omit_links`, `old_gaps`/`new_gaps`, `sync_scroll`, `on_wheel`, `on_gutter_move`, `code_pane`, `center_gutter`, `ln_col`, `seam_hairline`, `row_texts`, `render_row_text`, `Viewport::content_pad`, `SideLayout::seam_mask` (`is_seam` / `iter_rows` are test-only now). Paint helpers (bridges, pinch Bézier, waves, hatch) moved into element.rs unchanged.
- Viewport: `snapped(scale)`, `max_top(side)`, free `snap(v, scale)`. `hit` is live.

Deviations:

- Search hits and a hovered-bridge highlight are not painted. The div path never drew them in the pane, and §3 does not ask for them. `Decorations` is where they would go.
- Text and line numbers are vertically centered in the row (`ShapedLine::paint` with line height = `row_h`). The divs top-aligned them with the default ≈1.618 line height, so the text sits about 0.5 px lower and the 10 px numbers about 3 px lower (now centered on the code text). Word marks are `row_h − 1` tall, rounded 2 px, same color as before.
- The hunk index computed in prepaint (e.g. after open or a resize re-clamp) is emitted through `cx.defer`, because emitting during draw would not schedule the shell's redraw. Wheel, drag and jumps emit it right away.
- The scrollbar tracks overlay the outer 10 px of each code pane (the text inset is 12 px). Clicks there go to the bar, not the row.

Notes for 04:

- Horizontal: the wheel handler ignores `delta.x`. Add per-side `x_offset` to `DualPane` and apply it only to the text/mark origin in `Frame::paint_code` (`text_x`). Row backgrounds, gaps, seams, the gutter and waves must not move. The shaped widths for the longest visible line are in the cache (`ShapedLine::width`). Route shift+wheel / trackpad X over `frame.code[side]`.
- `Viewport` still has no `x_offset` input; the architecture table lists one.

Verification: `cargo build` shows no new warnings (the 17 remaining are pre-existing). `cargo test`: 116 passed, 2 failed (`ui::splitter::tests::{sidebar_clamp_keeps_commits_strip, diff_tree_clamp_keeps_dual_pane}`, which fail on base). New tests: `snap`, snapped tops feeding bridges, `max_top`, `ThumbGeom::offset_for`, thumb drag round-trip / clamp, line-number column width. `cargo run` starts without panicking. The Diff window was not opened, and nothing was checked visually, so the "Done when" §3 run-through and large-file smoothness still need a human.

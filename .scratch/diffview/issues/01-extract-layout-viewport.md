# Extract pure Layout and Viewport; clamp scroll_s to s_range

Status: resolved

## What

- New `src/ui/diff/layout.rs`: build per-file Layout from `Arc<str>` old/new text + `Alignment` + `FoldState`. Visual lines store byte ranges; omit separators are modeled apart from line rows (no `RowKind::Omit` in the line list). Includes bridges, knots, hunk lands, seam row index set, comment row index (rebuilt separately when comments change), lazy per-Replace word marks.
- New `src/ui/diff/viewport.rs`: from Layout + `scroll_s` + `view_h` + `row_h`, compute `s_range`, per-side tops, visible ranges, pixel bridges / gaps / omit links, `hit(side, y)`, `bridge_at(y)`. Move `interp`, `s_from`, `track`, `place_bridge`, `gap_intervals`, `empty_seam`, `content_pad`, `row_at` here.
- Move `DisplayRows`, `ScrollKnot`, `Bridge`, `RowKind` out of `domain`. `git::FileDiff::Text` keeps text + Alignment only.
- Clamp `scroll_s` to `s_range()` (fixes the first-scroll dead zone).
- Existing div UI consumes Layout/Viewport: no per-frame clones of rows/knots/bridges, no per-frame LCS, O(1) seam/comment lookups.

## Tests

Table-driven: insert at start/end, delete, 3→1 replace, one side empty, anchor preserved across expand/collapse, end-of-file gap, `s_range` has no dead travel.

## Done when

`cargo test` green; app behaves as before, minus the dead zone.

## Comments

**2026-09-27 — resolved (agent).**

What changed:

- `src/ui/diff/layout.rs` (pure): `Layout::build(old: Arc<str>, new: Arc<str>, &Alignment, Option<&FoldState>)`. Per side a `SideLayout` with `lines: Vec<LineRow>` (byte range into the shared text, `LineKind`, visual `row`) and `omits: Vec<OmitRow>` kept apart; `row(i)` / `iter_rows()` yield `Row::Line | Row::Omit`. Holds bridges, knots, hunk lands, `hunk_count`, max line number, seam list + O(1) mask, comment index (`set_comments`, rebuilt by `DiffView::refresh_comments` on rebuild and after a draft is saved), and lazy per-Replace word marks (`OnceCell` per bridge, filled on first `marks()` call).
- `src/ui/diff/viewport.rs` (pure): `Viewport::new(layout, scroll_s, view_h, row_h)` clamps to `s_range` and gives per-side tops, `visible_rows`, `visible_seams`, `gaps`, `bridges` (visible only, via a binary-searched index window), `omit_links` (k-th old separator pairs with k-th new — no O(N²)), `hit`, `bridge_at`, `row_at_anchor`, `capture_anchor`. Free fns: `s_range`, `clamp_s`, `s_for_content` (old `s_from`), `s_for_target`, `s_for_anchor`, `content_from_top`. `interp` is now a binary search.
- `domain` lost `RowKind`, `DisplayRow(s)`, `Bridge`, `ScrollKnot`, `HunkLand`, `display_rows*`. `git::FileDiff::Text` is `{ alignment, old_text: Arc<str>, new_text: Arc<str> }`.
- `DiffView` owns `layout: Option<Layout>` plus per-row `SharedString` text; render/sync no longer clone rows/knots/bridges, no longer run LCS per frame (marks only for visible rows ± one screen, memoized), seam/comment lookups are O(1), bridges/gaps/links/seams are culled to the viewport. Hover copy uses `Viewport::bridge_at` on mouse move (no per-frame `position_copy` strings).
- `scroll_s` is clamped to `s_range` every frame and after wheel, native readback, jump, match jump and anchor restore; reset leaves `scroll_s = 0` and the first measured frame lifts it to the lower bound.

Deviations:

- `HunkLand` moved to layout too (it carries a view `s`). `Anchor::line_on` became `Anchor::lines()` (span per side) so the comment index keeps its semantics in domain. `Bridge` is now `Copy`.
- New `DiffView::hunk_s` (unclamped hunk-navigation position, rows). Without it, jumps to Hunks inside the clamped top/bottom zones would move no pixels, and chrome (`hunk i of n`) plus the next `[`/`]` would be computed from the clamped `s` and skip those Hunks. It is set by open/reset and jumps, cleared by wheel / native scroll / fold changes, so navigation and chrome match the old behavior.
- Not done here (by plan): device-pixel snapping and per-side `x_offset` (03/04); the div panes still build every row (culling is 03).
- `s_range` only trims the ends. A mid-file stretch where one side has finished and the other has not yet left its top (e.g. whole-file Delete followed by Insert) still has wheel travel without motion; the spec only requires the ends.

For issue 02: move `layout`, `row_text`, `scroll_s`, `hunk_s`, `marked_rows`, gaps/placed/links into `DualPane`; `applied_*`, `scroll_nudge` and the native `ScrollHandle`s are still here and go away with 03. `hit` is unused until the Element exists (`allow(dead_code)`).

Verification: `cargo test` — 108 passed, 2 failed (`ui::splitter::tests::{sidebar_clamp_keeps_commits_strip, diff_tree_clamp_keeps_dual_pane}`, failing before this change: min width is 140, tests expect 160). GUI not run by the agent; visual behavior unverified.

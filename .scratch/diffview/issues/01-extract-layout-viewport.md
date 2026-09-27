# Extract pure Layout and Viewport; clamp scroll_s to s_range

Status: ready-for-agent

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

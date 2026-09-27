# Dual-pane Diff restructure — spec

Design: `docs/diffview-architecture.md`. Decision: ADR-0008. Behavior spec (unchanged): `docs/dual-pane-diff.md` §3.

## Acceptance

1. Every §3 behavior still holds (1/3 anchor, piecewise gap, fold/expand keeping anchor, hunk / match jump, search, font size, ignore whitespace, word-level marks).
2. Per-frame work scales with visible rows; a 20k-line file scrolls at 120 fps (measured with `REVIEWFOX_FRAME_TRACE=1`).
3. No wheel travel without visible motion at the top or bottom of any file.
4. Layout and Viewport are pure and covered by table-driven unit tests.

## Issues

01 → 02 → 03 → 04 → 05 (each blocked by the previous).

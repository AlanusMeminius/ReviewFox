# Dual-pane Diff restructure — spec

Design: `docs/diffview-architecture.md`. Decision: ADR-0008. Behavior spec (unchanged): `docs/dual-pane-diff.md` §3.

## Acceptance

1. Every §3 behavior still holds (1/3 anchor, piecewise gap, fold/expand keeping anchor, hunk / match jump, search, font size, ignore whitespace, word-level marks).
2. Per-frame work scales with visible rows; a 20k-line file scrolls at 120 fps (measured with `REVIEWFOX_FRAME_TRACE=1`).
3. No wheel travel without visible motion at the top or bottom of any file.
4. Layout and Viewport are pure and covered by table-driven unit tests.

## Acceptance notes (2026-09-27, after 05)

- 1: covered by unit tests for Layout / Viewport; on-screen §3 run-through still needs a human.
- 2: headless pure path (release, `perf::frame_cost_is_flat_in_file_length`): warm per-frame 1.3–3.3 µs, 20k / 2k ratio 1.25–1.32, ≤ 92 visible rows. Real 120 fps with shaping and GPU paint is not measured; run the app with `REVIEWFOX_FRAME_TRACE=1` on a 20k-line file to check.
- 3: `s_range` clamp, unit-tested (`s_range` no dead travel).
- 4: done (layout.rs / viewport.rs tests).

## Issues

01 → 02 → 03 → 04 → 05 (each blocked by the previous).

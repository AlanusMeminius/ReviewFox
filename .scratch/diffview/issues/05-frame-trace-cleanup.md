# Frame trace and dead-code removal

Status: resolved
Blocked by: 04

## What

- `REVIEWFOX_FRAME_TRACE=1` (off by default): log per-frame Layout / Viewport / shape / paint time and visible row count.
- Verify a 20k-line file scrolls at 120 fps and frame cost is independent of file length.
- Delete leftover fields and helpers from `diff_window.rs` (`applied_*`, `scroll_nudge`, `placed`, `hover_bands`, `omit_links`, `old_gaps`/`new_gaps`, seam row vecs, `ln_col`, `code_pane`, `center_gutter`, …).

## Done when

Trace numbers recorded in `## Comments`; no dead code left.

## Comments

**2026-09-27 — resolved (agent).**

What changed:

- `src/ui/diff/trace.rs`: `REVIEWFOX_FRAME_TRACE=1` (env read once via `OnceLock`). When it is off, each hook reads a cached bool and does nothing else: no `Instant`, no allocation. When it is on, stderr gets:
  - `[frame-trace] layout build <ms> rows <old>+<new> bridges <n>` on every `DualPane::rebuild_layout`.
  - `[frame-trace] frame: viewport <ms> build <ms> shape <n> rows <ms> prepaint <ms> paint <ms> visible <old>+<new> rows` per drawn pane frame. `build` is the paint-list build, shaping included. `shape` counts only cache misses and their time. The numbers ride in `Frame::stats` (`FrameStats`, `Copy`), and the line is printed at the end of `DualPaneElement::paint`. The project has no logger set up (`log` appears only once, in reqwest_client, with no init), so this uses `eprintln!` like the rest of the app.
- `pane::nearest_hunk_index` is now a `partition_point` (it was a linear scan over all Hunks every frame).
- `src/ui/diff/perf.rs` (test-only, `#[ignore]`): builds a synthetic file with N Equal runs + N Hunks that cycle insert 0→3, delete 3→0, replace 3→2 and 2→4. Every fifth Equal run is expanded and the rest fold. It times `Layout::build` (+ `max_chars`), then scrolls the whole `s_range` in 2000 frames, twice (cold, where word marks are computed as rows appear, and then warm). Each frame runs clamp, `Viewport::new().snapped(1.5)`, the visible-row paint list (row lookup, marks → spans, comment lookup, y snap), gaps, seams, bridges, omit links and the hunk index. It does not do glyph shaping, which needs a Window. It asserts warm 20k/2k < 2.5× and warm 20k < 1 ms.
- Dead code: `cargo build` already showed no warnings in `src/ui/diff*` / `diff_window.rs` after 03/04. A grep for `applied_*`, `scroll_nudge`, `placed`, `hover_bands`, `omit_links` (window), gaps / seam vecs, `ln_col`, `code_pane`, `center_gutter`, `ScrollHandle` and `sync_scroll` finds nothing left over. `row_at` backs `Viewport::row_at_anchor` (live), and `hit` is live. There was nothing to delete.
- Docs: architecture §3 table (Viewport takes no `x_offset`) and §7 (trace + headless test). The spec has acceptance notes.

Measured (headless, `cargo test --release -- --ignored frame_cost_is_flat_in_file_length --nocapture`, Windows, view 900 px, row 20 px):

| file | Layout build | rows old+new | per frame cold | warm (max) |
|---|---|---|---|---|
| 2k / 50 hunks, folded | 0.36 ms | 756+768 | 2.66 µs | 2.50 µs (18.1 µs) |
| 20k / 500 hunks, folded | 2.15 ms | 7507+7632 | 4.85 µs | 3.30 µs (43.9 µs) |
| 2k / 50 hunks, expanded | 0.29 ms | 1986+1998 | 1.40 µs | 1.30 µs (7.1 µs) |
| 20k / 500 hunks, expanded | 3.44 ms | 19537+19662 | 3.04 µs | 1.63 µs (12.0 µs) |

Warm ratio 20k / 2k: 1.32 (folded) and 1.25 (expanded). At most 92 visible rows in every case. So the pure per-frame path is flat in file length and takes about 0.04% of a 120 fps frame budget (8.3 ms).

Still needs a human:

- A real on-screen check at 120 fps: open a 20k-line diff with `REVIEWFOX_FRAME_TRACE=1` and scroll fast. `prepaint` + `paint` should stay well under 8 ms, and `shape` should be ~0 rows after the first frame, except for rows newly entering the view. Shaping and GPU paint are not covered headlessly.
- The §3 visual run-through left over from 03/04.

Verification: `cargo build`: 17 warnings, all pre-existing and outside this track (0 in `src/ui/diff*` / `diff_window.rs`, before and after). `cargo test`: 120 passed, 2 failed (`ui::splitter::tests::{sidebar_clamp_keeps_commits_strip, diff_tree_clamp_keeps_dual_pane}`, pre-existing), 1 ignored (the perf test, which passes in release). `cargo run` with the trace on starts and stays up for 10 s. No trace lines were printed, because the Diff window was not opened.

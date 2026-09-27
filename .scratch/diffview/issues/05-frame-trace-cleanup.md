# Frame trace and dead-code removal

Status: ready-for-agent
Blocked by: 04

## What

- `REVIEWFOX_FRAME_TRACE=1` (off by default): log per-frame Layout / Viewport / shape / paint time and visible row count.
- Verify a 20k-line file scrolls at 120 fps and frame cost is independent of file length.
- Delete leftover fields and helpers from `diff_window.rs` (`applied_*`, `scroll_nudge`, `placed`, `hover_bands`, `omit_links`, `old_gaps`/`new_gaps`, seam row vecs, `ln_col`, `code_pane`, `center_gutter`, …).

## Done when

Trace numbers recorded in `## Comments`; no dead code left.

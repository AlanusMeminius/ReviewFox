# Background highlighting and size guard

Status: resolved

Blocked by: 03

## What

- Move `highlight` off the UI thread (`cx.background_spawn` or equivalent): open shows plain text; on completion, if the same file is still open, store the spans, clear only `ShapeCache`, `notify`. Stale results (another file opened meanwhile) are dropped — key by an open-generation counter.
- Both sides may run in one task or two; either way, no partial repaint that mixes sides from different files.
- Size guard: skip highlighting when a side exceeds limits on total lines, longest line, or bytes. Pick the numbers by benchmarking `highlight` on large real Rust / C++ files (release build) and record them with the measurements in this issue's Comments.
- Trace: add highlight time to the existing `REVIEWFOX_FRAME_TRACE` output (or a parse counter) so acceptance 3 (no re-parse on fold / ignore-whitespace) is observable.

## Tests

Guard predicate table-tested at the boundaries; stale-generation drop logic as a pure unit if it can be factored out.

## Done when

`cargo test` green; human check: a very large C++ file opens as fast as before and colors in shortly after; a file above the guard stays plain.

## Carried from 01 review (2026-09-28)

- First `syntax` call pays the one-time query compile (~53 ms release / ~148 ms debug); it must happen on the background thread, never on the UI thread (consider warming it at startup in the background).

## Comments

**2026-09-28 — resolved (agent).**

What changed:

- `DualPane::open` bumps `open_generation`, clears highlights, opens as plain text, then `cx.spawn` + `background_executor().spawn` highlights both sides in one task. On completion: if generation still matches, store both sides together, `shapes.clear()` only, `notify`. Stale results dropped via `should_apply_highlight`.
- Size guard `syntax::exceeds_size_guard` / `DEFAULT_SIZE_GUARD` per side before `highlight`. Both sides over guard → no spawn.
- `syntax::warm()` + `theme::syntax_colors()` at app start on `cx.background_spawn` so query compile never hits the UI thread.
- Trace: `[frame-trace] highlight N.NNNms (count=K)` under `REVIEWFOX_FRAME_TRACE=1`. `syntax::highlight_count()` bumps only inside `highlight` (fold / ignore-ws must not change it).
- Tests: size-guard table at boundaries; stale-generation pure unit; ignored release benches for re-tuning.

### Guard thresholds

| Limit | Value | Why |
| --- | --- | --- |
| `max_bytes` | 512 KiB | Dense synth ~215 ms release; keeps “colors shortly after” under ~¼ s worst case |
| `max_lines` | 20_000 | Dense synth ~80 ms; real `app_view.rs` (~2.7k lines) ~20 ms |
| `max_line_bytes` | 8_192 | Highlight itself is cheap on long lines; cap protects shaping / TextRun cost |

### Measurements (`cargo test --bin reviewfox highlight_size_bench --release -- --ignored --nocapture`)

| Case | Size | Release `highlight` |
| --- | --- | --- |
| repo `app_view.rs` | 97 KB, 2748 lines | 20.3 ms |
| rust ×2k lines | 25 KB | 7.1 ms |
| rust ×10k lines | 126 KB | 38.2 ms |
| rust ×20k lines | 252 KB | 80.9 ms |
| cpp ×2k / ×10k / ×20k lines | 22 / 108 / 216 KB | 7.9 / 34.3 / 73.0 ms |
| rust long-line 16k / 64k | 1 line | 0.2 / 0.5 ms |
| rust ~256 / ~512 KB / ~1 MB | 12k / 24k / 48k lines | 107 / 215 / 435 ms |

Warm / query compile (`warm_compile_bench`): **51.4 ms release**, **144.9 ms debug** (matches carried ~53 / ~148).

Verify: `cargo test --bin reviewfox` → 177 passed, 3 ignored.

Deviations: used `background_executor().spawn` (same as GitLab paths) rather than `AppContext::background_spawn` inside the entity async context; startup warm uses `background_spawn` on `App`.

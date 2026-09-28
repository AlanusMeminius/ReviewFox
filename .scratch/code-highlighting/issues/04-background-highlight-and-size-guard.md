# Background highlighting and size guard

Status: ready-for-agent

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

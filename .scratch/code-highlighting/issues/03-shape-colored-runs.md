# Shape rows with colored runs

Status: ready-for-agent

Blocked by: 02

## What

- `DualPane::open` needs the file path: pass it with the `FileDiff` (today only alignment + texts reach `PaneFile`). Detect the language there.
- Hold per-side highlights on the pane beside `layout` (not inside `Layout`): `Option<Arc<[Span]>>` per side. `rebuild_layout` (fold / ignore-whitespace) keeps them.
- `element.rs` `shape_row`: build multiple `TextRun`s from `spans_in` for the line, mapped through `TabExpansion::display_offset`, gaps filled with `theme::text()`; no highlights → single run as today. Line numbers unchanged.
- Row backgrounds, word marks, comment bar untouched.
- Computed synchronously in this issue (async is 04), so the path is testable end-to-end first.

## Tests

Pure helper `runs_for_line(line_text, spans, tabs, palette) -> Vec<(len, color)>` table-tested: tabs before a token, token spanning the whole line, line with no spans, multibyte chars inside a string.

## Done when

`cargo test` green; human check: a `.rs` / `.cpp` / `CMakeLists.txt` file shows color on both sides, fold/expand keeps it, other files look as before.

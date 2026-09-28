# Shape rows with colored runs

Status: resolved

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

## Carried from 01 review (2026-09-28)

- Spans may include line terminators (`"/// doc\n"`, CRLF `"// c\r"`). `spans_in` clips to the line range, but check the Layout line range and `display_offset` treat a trailing `\r` the same way word marks do.

## Comments

- `DualPane::open(path, file, …)`: path from `DiffSnapshot::selected_path`. On open, `syntax::detect` → sync `highlight` per side into `highlights: [Option<Arc<[Span]>>; 2]` beside `layout`. `rebuild_layout` does not clear them.
- `shape_row` uses `runs_for_line` + multi-`TextRun` shaping when highlights exist; plain single run otherwise. Palette resolved lazily on first highlight (`syntax_colors()`), not at theme build.
- Pure `runs_for_line` table-tested (tabs before token, whole-line token, no spans, multibyte string). Layout test confirms CRLF `\r` is stripped from line ranges like word-mark text, so terminator-inclusive spans clip cleanly.
- Deviation: path is an `open` argument rather than a `FileDiff` field (avoids touching git/`FileDiff`). Async/size guard still issue 04.
- Verify: `cargo test --bin reviewfox` → 174 passed, 1 ignored.

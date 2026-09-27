# Syntax engine and language detection (pure)

Status: ready-for-agent

## What

- Add `tree-sitter` 0.27, `tree-sitter-highlight`, `tree-sitter-rust`, `tree-sitter-cpp`, `tree-sitter-c` (query text only), `tree-sitter-cmake`.
- New pure module (e.g. `src/syntax/`), no GPUI:
  - `Language` registry: Rust, Cpp, CMake — each with its grammar, highlights query (C++ = C query + cpp query; CMake = `assets/queries/cmake/highlights.scm`, a placeholder here, written in 05) and the capture-name list.
  - `detect(path, first_line) -> Option<Language>`: extension → special file name → shebang hook, per the spec table. Extensions match exactly as listed (lower case); `CMakeLists.txt` by file name; `*.cmake.in` by suffix.
  - `highlight(lang, text) -> Vec<Span>` where `Span { range: Range<usize>, capture: CaptureId }`: sorted, non-overlapping, byte ranges into `text`; innermost capture wins.
  - `spans_in(spans, line_range) -> impl Iterator<(Range<usize>, CaptureId)>` relative to the line start (binary search, no per-line scan of the whole file).
- Queries are compiled once per language (lazy static), not per file.

## Tests

Table-driven: detection for every listed extension / file name, unknown → `None`; Rust keyword / string / comment spans; a C++ file gets C-level keywords (proves the C query is prepended); a block comment spanning 3 lines yields comment spans on each line via `spans_in`; empty text; syntactically broken input still returns spans (no panic).

## Done when

`cargo test` green; module is not yet wired into the UI.

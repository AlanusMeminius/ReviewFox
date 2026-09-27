# Syntax engine and language detection (pure)

Status: resolved

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

## Comments

**2026-09-28 — resolved (agent).**

What changed:

- `Cargo.toml`: `tree-sitter` 0.27, `tree-sitter-highlight` 0.27, `tree-sitter-rust` 0.24, `tree-sitter-cpp` 0.23, `tree-sitter-c` 0.24 (query text only), `tree-sitter-cmake` 0.7. Grammars build through `cc` on MSVC with no extra setup.
- New `src/syntax/mod.rs` (pure, no GPUI; `mod syntax;` in `main.rs`, `#![allow(dead_code)]` until 03 wires it):
  - `enum Language { Rust, Cpp, CMake }`.
  - `detect(path: &Path, first_line: &str) -> Option<Language>`: extension (exact, lower case) → `CMakeLists.txt` / `*.cmake.in` by file name → shebang. The shebang hook parses the interpreter (`#!/bin/sh`, `#!/usr/bin/env -S python3 -u` → `python3`) and maps it through `from_interpreter`, which resolves nothing in v1.
  - `struct CaptureId(pub u16)`, `struct Span { range: Range<usize>, capture: CaptureId }`.
  - `highlight(lang, text) -> Vec<Span>`: sorted, non-overlapping, non-empty byte ranges; innermost capture wins (top of the `HighlightStart` stack); adjacent ranges with the same capture are merged; a highlighter error keeps the spans found so far.
  - `spans_in(spans, line: Range<usize>) -> impl Iterator<Item = (Range<usize>, CaptureId)>`: `partition_point` to the first span, clipped, relative to `line.start`, empty pieces dropped.
  - `capture_names() -> &'static [String]`, `capture_name(CaptureId) -> &'static str`.
  - One `LazyLock<Registry>` compiles all three queries on first use. C++ = `tree_sitter_c::HIGHLIGHT_QUERY` + `"\n"` + `tree_sitter_cpp::HIGHLIGHT_QUERY`. CMake = `include_str!("assets/queries/cmake/highlights.scm")`.
- New `assets/queries/cmake/highlights.scm`: a placeholder with only comments (`line_comment`, `bracket_comment`) and strings (`quoted_argument`, `bracket_argument`). 05 replaces it.

Deviations:

- The capture-name list is global, not per Language. Every config is `configure`d with the sorted union of all three queries' capture names (minus `_`-prefixed ones), so each capture maps to exactly itself and a `CaptureId` means the same name in every Language. 02 can resolve one `Vec<Rgba>` indexed by `CaptureId`, with no per-language table. Because of this the Registry compiles all three queries together on first use, not one Language at a time. This costs one-off compile time, which 04's background thread absorbs.
- No locals or injections queries are passed (`""`). Spec: no injections in v1. Locals only refine `variable` vs local names.

For 02: the palette must cover `capture_names()`, which 05 will grow. Rust emits dotted names such as `function.method`, `type.builtin`, `constant.builtin`, `punctuation.delimiter`, `comment.documentation` and `escape`. A test that iterates `capture_names()` covers all of them.
For 03: ranges are bytes into the full side text and are always on char boundaries (tested with CJK). Rust's `line_comment` span does not include the `\n`.
For 05: the query is compiled by `Registry::new`, which panics with the Language and the `QueryError` if the query is invalid, so any `highlight(Language::CMake, …)` test also checks that the query compiles. `cmake_placeholder_query_compiles` should be replaced by the full category test.

Verification: `cargo test --bin reviewfox`: 167 passed, 0 failed, 1 ignored (the two `ui::splitter` failures did not show up in this run). 11 new tests in `syntax::tests`. `cargo clippy --tests` shows no warnings in `src/syntax`. The GUI was not run (the module is not wired in).

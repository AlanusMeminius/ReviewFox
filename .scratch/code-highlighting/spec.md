# Diff syntax highlighting — spec

Settled in grill (2026-09-28). Decision: ADR-0010. Presentation only: no Comparison / Alignment / Hunk / Export change, no `CONTEXT.md` term.

## Scope

- Dual-pane Diff code area only. Export stays plain text; the main window has no code view.
- Always on; no setting, no toolbar toggle.
- Foreground color only: row backgrounds (`add_bg` / `del_bg` / `mod_bg`), word marks (`mod_chg`), comment bar and search behavior are unchanged. Both sides get full syntax color (no dimming of the old side or deleted lines).

## Engine

- crates.io `tree-sitter` (0.27) + `tree-sitter-highlight`; grammars compiled in. No runtime (WASM / dylib) grammar loading; a new language is a new dependency plus a registry entry.
- No Zed `language` / `languages` crates (ADR-0001, GPL). Zed `.scm` files are reference only.
- No injections in v1: embedded code renders as its host language sees it.

## Languages (v1)

| Language | Grammar | Files | Highlights query |
|---|---|---|---|
| Rust | `tree-sitter-rust` | `*.rs` | bundled `HIGHLIGHTS_QUERY` |
| C / C++ | `tree-sitter-cpp` | `*.c *.h *.cpp *.cc *.cxx *.hpp *.hh *.hxx *.inl *.ipp` | `tree-sitter-c` `HIGHLIGHT_QUERY` **then** `tree-sitter-cpp` `HIGHLIGHT_QUERY` (cpp's `tree-sitter.json` inherits C's; cpp alone misses keywords, strings, comments) |
| CMake | `tree-sitter-cmake` | `CMakeLists.txt`, `*.cmake`, `*.cmake.in` | repo-owned `assets/queries/cmake/highlights.scm` (upstream is Neovim-flavored: `#lua-match?`, `@spell`, `@none`) |

- Query source rule: bundled query by default; a repo file under `assets/queries/<lang>/highlights.scm` overrides it when the bundled one is not good enough.
- Detection order: extension → special file name → first-line shebang (only `bash` / `sh` / `python` / `node`; none map to a v1 language yet, so the hook exists but resolves nothing). No match → plain text. Old and new share the path (no rename detection), so one language per file.
- Unlisted languages render as today.

## Computation

- Each side's full blob text is parsed and highlighted once → sorted, non-overlapping `(byte range, highlight)` spans over that text. Multi-line strings / block comments color correctly, including inside folded regions.
- Spans are cut per line (Layout line = byte range into the same text) and mapped through `TabExpansion::display_offset`, the same path word marks use.
- The result lives per open file per side, beside Layout, not in it: fold / expand / ignore-whitespace rebuild Layout but reuse the highlights (the text never changes within a Diff snapshot, ADR-0005).
- Asynchronous: a file opens as plain text; highlighting runs on a background thread; on completion only that file's `ShapeCache` is cleared and the pane re-renders. A result for a file no longer open is dropped.
- Size guard: files above a threshold (lines, longest line, bytes) stay plain. Numbers are set by benchmark in issue 04.

## Colors

- Palette derived from Zed One Light `syntax`, keyed by capture name (`keyword`, `string`, `comment`, `function`, `type`, …).
- Fallback by dropping the last dotted segment (`function.method` → `function`); nothing matches → `theme::text()`.
- Every color is checked against `add_bg`, `del_bg` (grey, the hardest), `mod_bg` and `mod_chg`, and tuned where it is not legible.

## Acceptance

1. `.rs`, C/C++ and CMake files show syntax color on both sides; other files look exactly as before.
2. Block comments / raw strings spanning many lines color correctly on every line, including after expanding a fold.
3. Toggling fold / ignore-whitespace does not re-parse (visible in trace or a counter).
4. Opening a large file is not slower than today before first paint; color appears after.
5. Files above the size guard stay plain and responsive.
6. Word marks, row tints, comment bar, search and Export behave as before.
7. Detection and span computation are pure and unit-tested.

## Issues

01 → 02 → 03 → 04. 05 is blocked by 01 only and can run beside 02–04.

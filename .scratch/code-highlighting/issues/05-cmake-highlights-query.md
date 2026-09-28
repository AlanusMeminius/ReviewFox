# Repo-owned CMake highlights query

Status: ready-for-agent

Blocked by: 01

## What

- Write `assets/queries/cmake/highlights.scm` against `tree-sitter-cmake` 0.7.5's node types. Upstream's query is Neovim-flavored and unusable as is: `#lua-match?` is not evaluated by `tree-sitter-highlight` (the pattern would match unconditionally), and `@spell` / `@none` are not our capture names.
- Use only standard predicates (`#match?`, `#eq?`, `#any-of?`) and capture names the palette (02) knows.
- Cover at least: comments (line / bracket), quoted and bracket arguments as strings, command names as functions (control-flow commands `if/elseif/else/endif/foreach/endforeach/while/endwhile/function/endfunction/macro/endmacro/return` as keywords), `${VAR}` / `$ENV{}` / `$CACHE{}` refs as variables, ALL_CAPS unquoted arguments (`PUBLIC`, `REQUIRED`, …) as constants, generator expressions `$<...>` if the grammar exposes them.
- Write it fresh; upstream / Zed / nvim queries are reference only.

## Tests

Via the 01 module: a sample `CMakeLists.txt` yields the expected capture for each category above; the query compiles with no unknown predicates.

## Done when

`cargo test` green.

## Carried from 01 review (2026-09-28)

- Harden first: all three queries compile inside one shared `LazyLock` (`src/syntax/mod.rs`), so an invalid CMake query panics and poisons it for Rust/C++ too. Keep compile errors per Language (e.g. `Option<HighlightConfiguration>` + `log::warn!`) so a bad query makes only that language plain; keep a test that every bundled/repo query compiles.
- Small cleanups in the same module: `from_interpreter` collapses to `None` with the doc comment (drop the unused digit stripping); `Registry::config` indexes by `lang as usize` instead of a linear search; `log::debug!` where highlighter errors are dropped.
- Replace the `cmake_placeholder_query_compiles` test with the per-category test.

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

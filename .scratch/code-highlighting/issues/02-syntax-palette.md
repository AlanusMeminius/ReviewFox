# Syntax palette

Status: ready-for-agent

Blocked by: 01

## What

- In `src/ui/theme.rs` (or a `syntax` submodule of it): a capture-name → `Rgba` table derived from Zed One Light `syntax` (values re-typed here, not copied from Zed source).
- Resolution: exact name, then drop the last dotted segment repeatedly (`function.method.call` → `function.method` → `function`), else `theme::text()`. Resolve once per `CaptureId` into a lookup vector so painting never string-matches.
- Contrast pass: every color against `add_bg`, `del_bg`, `mod_bg`, `mod_chg`; tune any that are hard to read (comment grey on `del_bg` is the known risk). Record the check (e.g. a small test asserting a minimum contrast ratio against each background).

## Tests

Fallback chain; unknown capture → `text()`; contrast test over the table.

## Done when

`cargo test` green; table covers every capture name the three v1 queries emit.

## Carried from 01 review (2026-09-28)

- Any `syntax` call (`capture_names()` included) compiles all queries once: ~53 ms release / ~148 ms debug. Do not resolve the palette on the UI thread at theme build or first paint; resolve it lazily alongside the first highlight result (or on the background thread).
- The C query emits bare `delimiter` (for `;` `.`), Rust emits `punctuation.delimiter`. The drop-last-segment fallback never maps `delimiter` to punctuation: add an explicit `delimiter` entry. The test looping over `syntax::capture_names()` should enforce every name has a deliberate color.
- Current union (25): attribute, comment, comment.documentation, constant, constant.builtin, constructor, delimiter, escape, function, function.macro, function.method, function.special, keyword, label, number, operator, property, punctuation.bracket, punctuation.delimiter, string, type, type.builtin, variable, variable.builtin, variable.parameter.

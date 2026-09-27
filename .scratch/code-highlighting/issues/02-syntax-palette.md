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

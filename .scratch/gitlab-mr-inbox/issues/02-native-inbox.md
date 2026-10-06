# Implement the selected native time inbox

Status: fixed

## Scope

Implement A in GPUI using registered GitLab Repositories (including Pin), all MR states, paginated discovery, local civil-day time ranges, read-only selection preview and explicit MR Entry open. Preserve filters on return. Isolate stale list, preview and MR activation responses; report per-Repository failures without suppressing successful results.

## Source

Prototype decision: `01-inbox-prototype.md`; source branch `prototype/gitlab-mr-inbox` at `3acad5e`.

## Validation

- `cargo check --quiet` passed.
- `cargo test --quiet`: 512 passed, 4 previously ignored, 0 failures.
- New coverage: calendar-day/month boundaries, complete pagination and duplicate suppression, missing credentials, preview without diff_refs, same IID in different Repositories, partial errors, stale list/preview results.
- Rendered the actual GPUI view with isolated in-memory fixtures at 1280×820 and 900×600; inspected screenshots. The temporary harness redirected disk state and skipped startup credentials; it is not shipped.
- No live GitLab-instance acceptance was performed. Discovery uses the existing configured Base URL/PAT; MR open uses the existing resolver.
- Prototype source stays on `prototype/gitlab-mr-inbox` (`3acad5e`); the native branch contains only the chosen design.

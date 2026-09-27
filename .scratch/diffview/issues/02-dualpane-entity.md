# Split DualPane Entity out of DiffView

Status: ready-for-agent
Blocked by: 01

## What

- `DualPane` Entity owns `FileDiffState { alignment, fold, layout, scroll_s, x_offsets, caches }`.
- `DiffView` keeps the shell (tree, chrome, search bar, draft bar, Review) and drives `DualPane` via methods (select file, jump hunk, expand/collapse, font size, view options, match jump).
- `DualPane` emits events: `BeginDraft { side, ln }`, hunk index change, hover copy.
- Scroll notifies `DualPane` only.

## Done when

Scrolling does not re-render the tree / chrome / comments; behavior unchanged.

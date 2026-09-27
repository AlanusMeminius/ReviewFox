# Split DualPane Entity out of DiffView

Status: resolved
Blocked by: 01

## What

- `DualPane` Entity owns `FileDiffState { alignment, fold, layout, scroll_s, x_offsets, caches }`.
- `DiffView` keeps the shell (tree, chrome, search bar, draft bar, Review) and drives `DualPane` via methods (select file, jump hunk, expand/collapse, font size, view options, match jump).
- `DualPane` emits events: `BeginDraft { side, ln }`, hunk index change, hover copy.
- Scroll notifies `DualPane` only.

## Done when

Scrolling does not re-render the tree / chrome / comments; behavior unchanged.

## Comments

**2026-09-27 — resolved (agent).**

What changed:

- New `src/ui/diff/pane.rs`: `DualPane` Entity owns the per-file state: `file` (Alignment + old/new `Arc<str>`), `fold`, `layout`, `row_text`, comment anchors, drafting line, `scroll_s`, `hunk_s`, `hunk_index`, `font_size`, native `ScrollHandle`s / `applied_*` / `scroll_nudge`, `view_h`, gaps / placed bridges / omit links, `marked_rows`, hover copy. It also holds all dual-pane rendering and painting (code panes, gutter, line numbers, bridges, waves, hatch). Methods: `open(file, comments)`, `set_alignment`, `set_comments`, `set_drafting`, `jump_hunk`, `jump_match`, `expand_all`, `collapse_unchanged`, `set_font_size`, `hunk_count`. It emits `PaneEvent::{BeginDraft{side, ln}, HunkIndexChanged, HoverCopy}`. Hunk index and hover copy are emitted only when they change.
- `DiffView` keeps the shell (tree, chrome, search, comments, draft bar, Review, ViewOptions, snapshot). It subscribes with `subscribe_in` so `BeginDraft` can focus the window. It mirrors `hunk_index` / `hover_copy` for the chrome. Wheel, native scroll, view_h measurement and gutter hover now notify only the pane.
- **GPUI constraint:** `Window::mark_view_dirty` marks every ancestor of a notified view as dirty. A pane nested inside DiffView's render would therefore still re-render the tree, chrome and comments on every scroll frame. So DiffView's render is now a thin root: `track_focus` + `on_key_down`, a cached `DiffShell` view (it holds a `WeakEntity<DiffView>`, calls `DiffView::render_shell` and re-renders when DiffView notifies), and `pane::slot`. `PaneSlot` is a small custom Element that lays out and prepaints the cached `DualPane` AnyView at the bounds that the shell's body canvas recorded earlier in the same frame (`SlotBounds = Rc<Cell<Option<Bounds>>>`). A scroll frame re-runs only DiffView's root and the pane, and the shell is reused. A shell change (typing a draft, search) reuses the pane.

Deviations:

- `DiffView::with_snapshot` now takes `window` and `apply_snapshot` takes `cx`. `app_view.rs` was updated to match.
- DiffView keeps `snapshot.file` (search reads the texts) and recomputes Alignment there on ignore-whitespace. The pane gets a clone of the Alignment through `open` / `set_alignment`. The pane's copy is what it renders from.
- Chrome `hunk i of n` is updated one frame after a scroll changes the hunk: the pane emits during its render, and the shell redraws on the next frame. Before this change it updated in the same frame.
- The pane is painted above the shell (it is a later sibling), not inside `diff-main`. The pane sets its own white background.

Notes for 03:

- `DualPaneElement` replaces the body of `DualPane::render`. `PaneSlot` / `DiffShell` stay as they are. With the element measuring `view_h` in prepaint, the canvas measurement and its prepaint-time `notify` (pre-existing, no frame request during draw) can go away.
- `ScrollHandle`s, `applied_*`, `scroll_nudge`, `pane_top_w` and `placed` / `omit_links` in window coords are all in `DualPane` now and can be removed there.
- The pane is not focusable. Keys go to DiffView's focus handle (root div). Keep it that way, or forward keys.

Verification: `cargo build` produced no new warnings. `cargo test`: 108 passed, 2 failed (`ui::splitter::tests::{sidebar_clamp_keeps_commits_strip, diff_tree_clamp_keeps_dual_pane}`, which also fail on base). The agent did not run the GUI, so visual behavior and the cache/slot mounting are not verified.

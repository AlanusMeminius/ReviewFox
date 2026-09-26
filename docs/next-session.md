# Next session

## Done

- Domain + ADRs: `CONTEXT.md`, `docs/adr/0001`–`0003`
- UI accepted: Beads-rail shell (prototype A) — `prototype/review-shell.html`, ADR 0003
- GPUI scaffold (`gpui = 0.2.2`)
- Real Git: Open Repo, commit list + Shift+range → Comparison, paths, Alignment Diff
- DraftComment + Anchor (in-memory) on Diff lines
- **Export** (`src/export.rs`): narrative snippets at anchors (±2 context) → clipboard via Diff **Export** button
- **Viewing-flow UX** (2026-09-26): nested ChangedPath trees (main read-only + Diff clickable), ± stats + M/A/D, `Open Diff` button (only refresh path for Diff), unified Diff scroll, gap/ribbon/checkbox polish, IBM Plex font names, `ChangedPath` in CONTEXT
- **Diff chrome sync** (2026-09-26): tree toggle/traffic-lights match main, resizable tree|dual splitter (`clamp_diff_tree_width`), Export icon button, dual-heads row removed
- **Workspace sidebar** (2026-09-26): persist `{last, pinned, workspaces}` to Application Support; restore `last` on launch; sidebar is **Recent** (0–1, no context menu) + **Pin** (hidden when empty, newest on top) + **Repositories** (non-pinned only, alpha by display name); right-click Pin → Unpin|Remove, Repositories → Pin|Remove; Open Repo / branch select / click row write immediately; CLI argv / `REVIEWFOX_REPO` removed.
- **Overlay scrollbars** (2026-09-26): BeadsViewer-shaped overlay thumbs on sidebar / commits / file tree / head-meta / branch picker / Diff tree / comments; Diff dual-pane per-pane scroll (old left / new right) + gutter lockstep (ADR 0003).

## Still deferred

- **Review / DraftComment disk persistence** — still parked (comments remain in-memory only)
- UnresolvedAnchor / SuggestedAnchor / hunk-owned anchors
- ViewOptions UI toggle
- Proper text input (draft bar is keystroke-based)
- Real branch switcher / commit filter
- True diagonal gap hatch (flat `gap_bg` + `╱` stand-in)
- Bundle IBM Plex fonts if OS lacks them
- Syntax highlighting
- Rename detection, GitLab, remote publish

## Next candidates (when ready)

- Draft text input polish
- UnresolvedAnchor when Comparison changes with open comments
- Review reopen persistence (parked)

## Run

```bash
cd /Users/alanus/Dev/ReviewFox
cargo run
```

Open Repo → commits → **Open Diff** → Diff tree file → click line → type → Enter → **Export**.

Smoke: `cargo test --bin reviewfox`

## Constraints (settled)

- crates.io GPUI only; custom diff, not Zed editor
- Comparison is the only reviewable surface
- v1: no rename detection, no GitLab, no remote publish
- Diff stays a separate window with its own snapshot
- Default Export is narrative snippets, not the full patch
- Main Changes tree is browse-only; enter Diff via **Open Diff**
- Diff keeps its own snapshot; changing Comparison on main does not refresh Diff until **Open Diff** again

# Next session

## Done

- Domain + ADRs: `CONTEXT.md`, `docs/adr/0001`–`0003`
- UI accepted: Beads-rail shell (prototype A) — `prototype/review-shell.html`, ADR 0003
- GPUI scaffold (`gpui = 0.2.2`)
- Real Git: Open Repo, commit list + Shift+range → Comparison, paths, Alignment Diff
- DraftComment + Anchor (in-memory) on Diff lines
- **Export** (`src/export.rs`): comment targets and bodies plus shared unified diff excerpts (up to ±2 unchanged context lines), using captured comment-time text → clipboard via Diff **Export** button
- **Viewing-flow UX** (2026-09-26): nested ChangedPath trees (main read-only + Diff clickable), ± stats + M/A/D, `Open Diff` button (only refresh path for Diff), unified Diff scroll, gap/ribbon/checkbox polish, IBM Plex font names, `ChangedPath` in CONTEXT
- **Diff chrome sync** (2026-09-26): tree toggle/traffic-lights match main, resizable tree|dual splitter (`clamp_diff_tree_width`), Export icon button, dual-heads row removed
- **Workspace sidebar** (2026-09-26): persist `{last, pinned, workspaces}` to Application Support; restore `last` on launch; sidebar is **Recent** (0–1, no context menu) + **Pin** (hidden when empty, newest on top) + **Repositories** (non-pinned only, alpha by display name); right-click Pin → Unpin|Remove, Repositories → Pin|Remove; Open Repo / branch select / click row write immediately; CLI argv / `REVIEWFOX_REPO` removed.
- **Overlay scrollbars** (2026-09-26): BeadsViewer-shaped overlay thumbs on sidebar / commits / file tree / head-meta / branch picker / Diff tree / comments; Diff dual-pane per-pane scroll (old left / new right) + gutter lockstep (ADR 0003).
- **GitLab Phase A** (2026-09-26): Settings window (ReviewFox → Settings…, `Cmd+,` / `Ctrl+,`); Base URL → `settings.json`, PAT → keychain; explicit Verify via GPUI `HttpClient` → `GET /api/v4/user` (`docs/adr/0006-gitlab-mr-is-readonly-entry.md`).
- **GitLab Phase B** (2026-09-26): Map open repo remotes → GitLab `path_with_namespace` (prefer host matching Settings Base URL; among matches prefer `origin`); reject when no remote matches; optional `GET /api/v4/projects/{path}` confirm; Settings shows project line.
- **GitLab Phase C** (2026-09-26): List open MRs for resolved project; MR Entry picker in main chrome (in-memory); shared GitLab connection line (user + Re-verify + Settings) in shell and Settings.
- **GitLab Phase D** (2026-09-26): On MR select, fetch detail + `diff_refs` (base/head/start SHA); pipeline / approval check state; MR Entry chrome shows loaded detail and short SHAs.
- **GitLab MR activate** (2026-09-27): Hide MR + connection chrome unless a remote host matches Settings; on MR select load GitLab MR commits (web-parity set, newest-first), `git fetch` SHAs from matching remote, replace commit list, set Comparison to `diff_refs`; clear MR / switch branch restores Branch Browser (E+F slice).
- **GitLab Phase G** (2026-09-27): Persist MR Entry label (`project` + `iid`) on Workspace; restore + refetch on launch / repo select; clear label when clearing MR or switching branch.
- **Dual-pane Diff** (2026-09-27): block-first Replace, fold, hunk jump, ignore-whitespace, word highlight, search, font size, bezier ribbons. Omission separator is one gray sine (no fill, no label). A height change eases between the line-number columns; the sine rides that centerline. Design: `docs/dual-pane-diff.md` §3.3. Prototype variants: branch `throwaway/diff-omit-wave`.

## Still deferred

- **Review / DraftComment disk persistence** — still parked (comments remain in-memory only)
- UnresolvedAnchor / SuggestedAnchor / hunk-owned anchors
- ViewOptions UI toggle
- ~~Proper text input (draft bar is keystroke-based)~~ → settled 2026-09-29: drag-select + gutter icon + bottom dock with a real TextField (`.scratch/diff-multiline-comment/`)
- Real branch switcher / commit filter
- True diagonal gap hatch (flat `gap_bg` + `╱` stand-in)
- Bundle IBM Plex fonts if OS lacks them
- ~~Syntax highlighting~~ → settled 2026-09-28: tree-sitter, Rust / C++ / CMake (ADR-0010, `.scratch/code-highlighting/`)
- Rename detection, remote publish (comments stay local; Export is egress)
- GitLab discussion-thread UI, OAuth (deferred—not rejected; see ADR-0006), remote-only (no local Repository) Diff

## GitLab MR Entry (ADR-0006) — phased plan

Shared understanding confirmed 2026-09-26; **Phase A aligned 2026-09-26**. Not one session; resume by phase.

| Phase | Work | Depends on |
|---|---|---|
| **A** | Settings: GitLab Base URL + PAT; API smoke | — |
| **B** | Map current repo `remote` → GitLab project; reject mismatch | A |
| **C** | List open MRs for that project; select → MR Entry; **persistent auth/connection chrome** (Settings or shell: current user / re-verify) | A, B |
| **D** | Fetch MR detail + check state; take `diff_refs` | C or H |
| **E** | `git fetch` SHAs into local Repository; clear errors | D — **done in MR activate slice** |
| **F** | `set_comparison_oids` → existing Diff/Review/Export; MR commits list = GitLab commits API; hide GitLab chrome without matching remote | E — **done in MR activate slice** |
| **G** | Persist/restore MR Entry label on Workspace; refetch on open | F — **done** |
| **H** | Paste URL/IID as secondary entry | **won't do** — MR list entry is enough |

## Next candidates (when ready)

- Multi-line comment editing in the draft dock (Shift+Enter stores a newline, but the field shows one line)
- UnresolvedAnchor when Comparison changes with open comments
- Review / DraftComment disk persistence (parked)
- **Syntax highlighting** — spec + issues in `.scratch/code-highlighting/` (ADR-0010); start at 01
- ViewOptions UI toggle / rename detection (later polish)

## Run

```bash
cd /Users/alanus/Dev/ReviewFox
cargo run
```

Open Repo → commits → **Open Diff** → Diff tree file → click line → type → Enter → **Export**.

Smoke: `cargo test --bin reviewfox`

## Constraints (settled)

- crates.io GPUI only; custom diff, not Zed editor
- Syntax highlighting: tree-sitter, compiled-in curated grammars (ADR-0010)
- Comparison is the only reviewable surface
- v1: no rename detection, no remote publish
- GitLab MR = read-only Entry (`diff_refs` → Comparison); see ADR-0006
- Diff stays a separate window with its own snapshot
- Default Export separates single-side comment targets from shared unified diff context; it includes complete addressed blocks, not the full patch, and preserves comment-time text even for Uncommitted
- Double-click a file in the main Changes tree to open its Diff; single-click leaves it unchanged, and directory rows only toggle expansion
- Diff keeps its own snapshot; changing Comparison on main does not refresh Diff until **Open Diff** or a file double-click

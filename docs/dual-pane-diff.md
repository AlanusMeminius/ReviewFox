# Dual-pane Diff — Design & Plan

Status: interaction rules for fold and scroll settled in grill (2026-09-26). Prototype Diff at `prototype/diff-view.html` proves the single-omission case only. Next: implement §3.1–§3.5, then GPUI port.
GPUI structure / performance: `docs/diffview-architecture.md` (ADR-0008).
References: `CONTEXT.md` (Alignment / Hunk / ViewOptions), ADR-0001, ADR-0003, ADR-0006, CLion side-by-side UX, `prototype/diff-view.html`.

## 1. Intent

ReviewFox Diff is a **read-only, Alignment-driven dual-pane** (old left, new right). The primary capability is **position relationship**: the user must see how code on one side relates to the other—not merely two tinted buffers.

Paint and chrome serve that capability. Comments / Export stay adjacent product; this plan does not expand them.

## 2. Settled decisions

| Topic | Decision |
|-------|----------|
| Scope | Dual-pane comparison only (not DraftComment persistence / UnresolvedAnchor in this track) |
| Domain | Position relationship **is** Alignment; do not invent a parallel glossary term |
| Replace | **Block-first** (ADR-0006); pairwise only when reliable |
| Tooling path | **Prototype first**, then port to GPUI |
| Context fold | **Equal only**, **3 lines** kept at each end of a run beside a Hunk; the rest of that run is one omission separator **per side** (2026-09-26) |
| Wave order | (1) position viz → (2) fold + hunk jump → (3) word-level + ignore-whitespace → (4) search + font size |
| Feature set | Full list below — nothing from the product checklist is dropped for “v1 polish later” as an excuse to skip; waves are sequencing, not cuts |

## 3. Capability checklist (complete)

### 3.1 Layout & sync

Settled 2026-09-26. Each pane has its own vertical scrollbar (ADR-0003). Vertical positions are coupled, but not by one shared pixel offset.

The viewport anchor is the line about one third of the way down the viewport. The scroll gap at a point is `old visual rows above it − new visual rows above the corresponding point`. Insert, Delete, and Replace change that gap when the two sides have different numbers of visual rows. An omission separator (§3.3) does not add a new gap; it sits at the gap already produced by the hunks above it. Only the point the viewport anchor is on is held in correspondence. Two omission separators stay collinear together only when their gaps are equal.

While scrolling down:

1. While the viewport anchor is inside the extra visual rows of a Hunk (lines that exist only on the longer side), the shorter side stays still.
2. Once the anchor has passed those rows, both sides move by the same distance, so the following paired lines share a row. An omission separator is held collinear the same way for as long as the anchor remains in that span.
3. Entering the next imbalance repeats 1–2 with the new gap. The last point is the file end: the shorter side stops, and the taller side continues until both last lines share one viewport row. That end gap is the whole-file height difference. If one side has no lines, that side stays on its only seam (file start or file end) and does not grow a blank body. At the end, the other side’s last line shares a viewport row with that seam.

Scrolling up reverses the same three steps.

- Side-by-side: Old | center gutter | New
- Each pane has its own vertical scrollbar. The old pane’s bar is on the outer left; the new pane’s bar is on the outer right.
- Independent line numbers (two columns in the center gutter, CLion-like)
- Horizontal scroll if a line overflows (§3.1.2); none while soft wrap is on

#### 3.1.1 Soft wrap

Settled 2026-09-29. A toggle in the Fold / ViewOptions capsule, persisted in settings, default off. It is a pane display preference, not a ViewOptions member: it never changes Alignment or Hunks.

- Wrap width is the code column of each pane; resizing the window or a splitter rewraps.
- Break rules (Zed's, plus one): a run of word chars (letters, digits, `- _ . ' $ % @ # ^ ~ , = :`) stays whole; a break may fall before a word that follows a space, or before a non-word, non-space char (so CJK breaks per char). Never inside the leading indent. Never between two consecutive non-word, non-space chars, nor inside `->`, `::`, `<=`, `>=`, `!=`, `==`, `<<=`, `>>=` (whose chars Zed counts as word chars), so operators like `->`, `<<`, `>>=`, `&&`, `//`, `/*` stay whole. A piece with no break point is split at a char, but never inside those operators; an operator wider than the whole wrap width stays on one row. No language-specific rules.
- Widths are real glyph widths (tabs expanded to their stops), so wrapped text never overflows the pane.
- Continuation rows keep the logical line's leading indent; when that indent is more than half the wrap width, they start at column 0.
- Line number on the first visual row only; continuation rows show none and no wrap marker.
- An Equal pair whose sides wrap to different row counts takes the larger count on both sides, the shorter side padded with blank rows (no hatch), so paired lines stay on one row and Equal never changes the §3.1 gap. Insert, Delete and Replace use each side's own row count.
- Toggling wrap, resizing and font size changes keep the viewport-anchor line in place (its first visual row when the anchor is on a continuation row); the other side is recomputed per §3.1.
- Everything on a continuation row belongs to its logical line: clicking starts a draft on that line, the comment bar spans all its rows, bridges and hatch span every visual row of the Hunk. Search hits and word marks crossing a break are drawn as one piece per row; a match jump puts the hit's visual row on the viewport anchor.
- While wrap is on, horizontal offset is 0 and horizontal scrollbars are hidden.

#### 3.1.2 Horizontal scroll

Settled 2026-09-29.

- **Sync** toggle in the Fold / ViewOptions capsule, persisted in settings, default on. Synced: one shared horizontal offset, bounded by the larger of the two sides' max; horizontal input over either pane moves both; the side with shorter lines just shows blank. Unsynced: each side keeps its own offset, moved only by input over that pane.
- Each pane has a horizontal scrollbar along the bottom of its code column (not under the gutter), shown only when that side overflows, drawn as a thin overlay in the vertical bars' style. Dragging either thumb while synced moves both sides.

### 3.2 Change coloring

| Kind | Visual |
|------|--------|
| Insert (add) | New-side tint (green). The connector uses that same green on both ends |
| Delete | Old-side tint (grey). The connector uses that same grey on both ends |
| Replace (modify) | Both sides the same blue, including the connector |
| Equal | Neutral; used as anchors |
| Gap (no line on that side) | Diagonal hatch (true hatch, not `╱` stand-in) |

### 3.3 Unchanged context: show + fold

Settled 2026-09-26. Omit **Equal** lines only. Delete, Insert, and Replace stay expanded.

- Beside each Hunk, keep **3 Equal lines** at each end of the contiguous Equal run (also before the first Hunk and after the last). A run that does not outlast that window stays fully visible. A file with no Hunk has nothing to locate: show every line, no omission separator.
- The lines beyond those 3 collapse. Each side draws **one omission separator** in its own line list, after that side’s last kept line. It is a view of one collapsed Equal span, not an Alignment op and not a shared row.
- The two separators are the same span. Their line numbers differ, and their vertical positions differ, only when the two sides have different numbers of visual lines above that span.
- The separator is drawn as one gray sine across both code columns and the gutter. No fill, no line-range label. Each side stays horizontal through its code and its line numbers. Where the two sides sit at different heights, a cubic Bézier joins them in the gap between the line-number columns. Each sine meets that curve at a crest, so the stroke is horizontal on both sides of the join, the same way the change ribbons leave a flat edge.
- Collapsing does not change Alignment. Expanding restores those Equal lines. Expanding or collapsing keeps the line on the viewport anchor where it is; the other side’s scroll is recomputed from the new visual row counts. If the anchor is the omission separator itself, the last visible line above it stays put, and the restored lines open below that line.
- Clicking either side’s separator expands that one span on both sides. **Expand all** opens every collapsed Equal span in the file. **Collapse unchanged** folds again every Equal run that outlasts the 3-line context. All three keep the anchor line in place.
- No semantic/AST folding.

### 3.4 Center connector (position relationship UI)

The gutter is the **primary** position UI. It must encode:

| Alignment op | What the user must perceive |
|--------------|----------------------------|
| **Insert** | New line numbers; insertion anchored **between** two old lines (or file start / file end). Connector pinches to a point / edge on the old side and opens on the new side |
| **Delete** | Old line numbers; deletion point on the new side. Connector opens on old, pinches on new |
| **Replace** | One bridge covering the whole Hunk (olds block ↔ news block). **Not** a stack of implied 1:1 row links |
| **Equal** | Accurate paired line numbers (old N ↔ new M); stable visual anchors |
| **Pairwise** (optional refinement on Replace) | Only when reliable: thin links or labels `old N → new M` inside the block |
| **Hover / focus copy** (assist) | Short text restating the same facts, e.g. “Insert new 16–18 after old 15”, “Replace old 20–22 ↔ new 21” — never a second source of truth |

Connector ends use each side’s current viewport Y, so a bridge slants while one side waits inside a Hunk’s extra rows.

### 3.5 Navigation & view options

- **Hunk jump**: previous / next Hunk. The Hunk’s first visual line — on the side that has lines — is placed on the viewport anchor (§3.1, about one third down) and the scroll gap is the gap at that line. The other side shows its corresponding line, or its seam when it has no line there, at that same height. If that line lies in a collapsed Equal span, expand that span on both sides first, then place the line using the row counts after expansion.
- **In-file search**: find string in old and/or new text; matches listed with side + line. Jumping to a match uses the same landing as a Hunk jump: if the match lies in a collapsed Equal span, expand that span on both sides first; then place the line on the viewport anchor (§3.1) and use the scroll gap at that line after expansion.
- **Font size**: increase / decrease / reset for the dual-pane mono text. The base size is the persisted Code Font size setting; A−/A+ are session-only per-pane overrides, `A` returns to the setting, and a change of the setting resets every pane to it
- **Ignore whitespace**: `ViewOptions.ignore_whitespace` wired into `compute_alignment` and a Diff UI toggle; changing it **recomputes Alignment** (same Comparison identity — see CONTEXT ViewOptions)

### 3.6 Intra-line (word-level) highlight

- Applies inside **Replace** (and pairwise-aligned line pairs when present)
- Darker span within mod background shows which tokens/chars differ
- Does **not** upgrade an unreliable many-to-many block into fake pairwise rows

### 3.7 Product boundary (not on this checklist)

These appear in CLion merge UX but are **outside** ReviewFox’s clipboard-review product (ADR-0001/0005): accept/reject chevrons that rewrite buffers, editable editors, apply-patch from gutter. Scrollbar change-tick minimap is **optional enhancement** after hunk jump exists—schedule only if it does not delay waves 1–2.

## 4. Domain model (no new identity)

Already in `CONTEXT.md` / `src/domain`:

- `Alignment` / `AlignmentOp::{Equal,Insert,Delete,Replace}` — position relationships
- `Hunk` — contiguous non-equal op (jump / fold unit)
- `ViewOptions` — ignore whitespace etc.
- `DisplayRow` — **view projection** only; must not lie about pairwise

### 4.1 Projection rules (fix current Replace padding)

Today `display_rows` pads Replace to `max(olds,news)` parallel rows — **rejected** by ADR-0006.

Target projection:

1. **Equal**: one visual row on each side for the paired lines (old_ln + new_ln). The two rows share a viewport Y only when the §3.1 scroll gap at that pair is the gap currently applied. They are not stored as one shared row.
2. **Delete**: one row per deleted old line; new side gap; gutter knows `at_new`.
3. **Insert**: one row per inserted new line; old side gap; gutter knows `after_old` (0 = before first line; `len` = after last).
4. **Replace (block)**: render as a **block unit** for connector geometry:
   - Left: all old lines of the Hunk stacked
   - Right: all new lines stacked
   - Vertical span of the bridge = union of both stacks (asymmetric heights OK — trapezoid)
   - Do **not** invent empty partner lines that look like matches unless that side truly has a gap for insert/delete
5. **Pairwise refinement** (later in wave 3): optional `Vec<(old_ln, new_ln)>` or per-line links **inside** a Replace; UI draws sub-links only for those pairs; unpaired lines stay block-only

Folding wraps Equal runs that outlast the 3-line context window. Each side keeps its own omission separator. Hunk ops stay intact and visible.

## 5. Prototype plan (`prototype/diff-view.html`)

Diff is a **separate page** from `review-shell.html` (full-width Diff window). Prove visuals and interaction before GPUI.

### Wave P1 — Position relationship ✅

- Replace demo `dualModel` with an explicit Alignment-shaped model (ops + line texts)
- Draw insert wedge / delete wedge / replace trapezoid in `.A-gutter`
- Hover title or side legend with position copy
- True diagonal gap hatch (CSS already closer than GPUI)
- Stop implying 1:1 inside replace blocks (demo includes old 5–7 ↔ new 3)

### Wave P2 — Fold + hunk jump

- Collapse Equal runs beyond ±3
- Prev/next hunk buttons + keys
- Scroll follows §3.1 when a fold changes row counts

### Wave P3 — Word-level + ignore whitespace

- Token/char diff spans inside replace lines
- Toggle ignore-whitespace recomputes the demo Alignment

### Wave P4 — Search + font size

- Find box; highlight matches; jump
- Font size controls affecting both panes + gutter line height

Exit criteria for prototype: a reviewer can answer, for every Hunk, the table in §3.4 without guessing.

## 6. GPUI port plan (`src/ui/diff_window.rs`, `domain`, `git`)

Mirror prototype waves; keep domain tests as the contract.

### Wave G1 — Position relationship

- Fix `display_rows` / introduce block-aware projection + connector layout input
- Center gutter: bridges from `Alignment` ops (not per-row `»` only)
- Hover/status text from ops
- Theme: real gap hatch
- Tests: insert at start/end; delete; many-to-many replace; equal pairing

### Wave G2 — Fold + hunk jump

- Fold state in DiffView (per-file)
- Jump uses `Alignment::hunks()`
- Subtitle / chrome shows hunk index

### Wave G3 — Word-level + ViewOptions

- Wire `ignore_whitespace` in `compute_alignment`
- Toggle in Diff chrome
- Intra-line highlight renderer for Replace lines (and pairwise if present)

### Wave G4 — Search + font size

- Search model + match navigation
- Font size on DiffView (and gutter metrics)

## 7. Gap vs today (short)

| Capability | Today | Target |
|------------|-------|--------|
| Side-by-side + central LNs | Yes | Keep |
| Sync scroll | Unified outer scroll | §3.1 piecewise gap; per-pane bars |
| Colors add/del/mod | Yes | Keep / tune to CLion-like |
| Gap hatch | Flat + `╱` | True hatch |
| Center bridges | Glyph ribbon | Wedge/trapezoid from Alignment |
| Position copy | No | Hover/focus assist |
| Replace semantics | Padded pseudo-1:1 | Block-first (ADR-0006) |
| Context fold | No | ±3 + expand |
| Hunk jump | Count in subtitle only | Prev/next |
| Search | No | Yes |
| Font size | Fixed | Adjustable |
| Ignore whitespace | Field unused | Wired + UI |
| Word-level | No | Yes inside Replace |
| Pairwise | Documented only | When reliable |

## 8. Acceptance (dual-pane track done)

1. For Insert/Delete/Replace/Equal, a user can state the §3.4 position facts from the UI alone.
2. A 3→1 rewrite never looks like three independent 1:1 line maps.
3. Fold, expand, and hunk jump keep the §3.1 scroll gap for the span in view.
4. Ignore whitespace changes Hunk boundaries without changing Comparison identity.
5. Word-level highlight never invents pairwise rows.
6. Prototype demonstrates P1–P4; GPUI matches behavior (chrome may differ).

## 9. Out of this track (still elsewhere)

Review persistence, UnresolvedAnchor / SuggestedAnchor, rename detection, GitLab, syntax highlighting, DraftComment input polish — see `docs/next-session.md`. Not required to close dual-pane position relationship.

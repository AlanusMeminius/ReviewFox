# Current Hunk highlight

Status: needs-triage

## What

Highlight the Hunk the user is on, the way Meld does: it paints the current chunk's connector once more with a 50% white overlay (`current-chunk-highlight`) before the outline stroke, so that ribbon reads lighter than the rest.

Deferred from the connector outline work (docs/dual-pane-diff.md §3.2 / §3.4).

## Open questions

- There is no current-Hunk state for paint today. The nearest thing is `DualPane::hunk_index` (the Hunk at / nearest the viewport, from `HunkJumpTarget` lands; it only drives chrome). Decide what "current" means: that index, the last jump target only, or the Hunk under the pointer.
- Whether the highlight covers only the connector (Meld) or the code-pane blocks too.
- How it interacts with the drafting and search-hit fills on the same rows.

## Comments

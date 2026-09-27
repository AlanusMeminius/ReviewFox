# ReviewFox

Local clipboard-oriented code review over a Git two-tree comparison: browse structured diffs, attach draft comments to positions, export plain text for pasting elsewhere.

## Language

**Repository**:
A local Git worktree root identified by its canonical absolute path (symlinks resolved).
_Avoid_: project, clone, repo path (unspecified canonicalization), working copy

**Workspace**:
The user's current place in the app: which Repository is open and which Entry is selected; not Comparison identity (OIDs). Survives relaunch as a set of Workspaces plus which one is last (Recent in the UI) and which are pinned; last is opened on launch.
_Avoid_: session, last opened (as a domain term), MRU

**Entry**:
How the user arrives at a Comparison inside a Workspace: a Branch Browser selection, or a forge Merge Request (MR) / Pull Request label. An MR Entry may carry read-only remote context—title, description, author, status, source/target branch labels, and forge check state (pipeline / approval)—to help understand the commits; that context is not Comparison identity and is never a publish target. Discussion threads are out of this context for now. MR Entries for a Repository are discovered from the forge (project MR list) and may also be opened by URL/IID; the Comparison OID pair for an MR Entry is the forge-reported diff pair (`diff_refs`), not branch-tip guesswork. Typical use: Branch Entry for reviewing one's own work, MR Entry for reviewing someone else's—capability is not restricted by that story.
_Avoid_: review mode, session mode, PR/MR as Comparison identity, workflow (as a domain type), published discussion

**Pin**:
A user-pinned Workspace shown in the Pin section and excluded from the Repositories list; not Comparison identity.
_Avoid_: favorite, bookmark, starred

**Comparison**:
A reviewable surface identified only by `(repository, base_oid, head_oid)` where `base_oid` and `head_oid` are commit object IDs; the reviewable content is the trees those commits name. Branch names and MR metadata are Entry labels, not identity. Diff is always over this two-tree pair (possibly folded from a commit selection), never “the MR” as a third identity.
_Avoid_: commit range (as identity), MR, branch tip (as identity), tree OID pair (as identity), per-commit review surface (as a second identity)

**Branch Browser**:
A read-only local branch/commit chooser that changes the displayed commit history and folds a default Comparison—without checking out or modifying the worktree. One kind of Entry into a Comparison; not the only Entry (e.g. MR labels).
_Avoid_: branch switcher, checkout picker, the whole app shell, Comparison identity, LoadedComparison

**Review**:
A user's ongoing work against one Comparison, holding draft comments and surviving reopen of the same Comparison. Independent of whatever Entry currently drives the main window's Comparison until the user explicitly opens that Comparison for review again.
_Avoid_: session (unless talking implementation), review result, review content (use Export), live-linked review

**DraftComment**:
A locally stored comment attached to a position within a Review; not published to any remote (MR path stays read-only toward the forge).
_Avoid_: note, annotation, discussion, published comment

**Export**:
A plain-text projection of a Review for the clipboard; default form is narrative snippets only where DraftComments attach (path, sides, line numbers, context, comment body), not the full patch.
_Avoid_: review content, review result, report, unified diff (as the default Export)

**SuggestedAnchor**:
A candidate re-binding offered for an UnresolvedAnchor when exactly one fuzzy match exists; never applied until the user confirms.
_Avoid_: auto-relocated anchor, silent fix

**ViewOptions**:
Display/diff-computation knobs (e.g. ignore whitespace) that may change hunk boundaries without changing Comparison identity.
_Avoid_: comparison settings, review settings

**ChangedPath**:
One path-level change inside a Comparison: repo-relative path, status (add/delete/modify), and line addition/deletion counts under current ViewOptions.
_Avoid_: file row, change entry, diff file, file list item

**Hunk**:
A contiguous algorithm-produced change block within one file of a Comparison under given ViewOptions; the stable unit comments may name.
_Avoid_: change region (as identity), semantic region, diff block (ambiguous)

**Alignment**:
An ordered description of how old and new file lines correspond: equal, insert, delete, or replace (replace may be many-to-many; pairwise line maps are optional refinement only).
_Avoid_: virtual row (view projection), line matrix (rendering), mapping (vague)

**Anchor**:
The attachment of a DraftComment to a place in a Comparison: either a file-level attachment (path only, no line span) or a line attachment (path, side old/new, line span, optional owning Hunk). v1 keys files by path only—no rename crossing. Not a ChangedPath (changeset listing).
_Avoid_: position (vague), location, cursor

**UnresolvedAnchor**:
An Anchor that could not be reliably re-located after Comparison or ViewOptions change; still kept on the Review but excluded from default Export. May carry at most one SuggestedAnchor.
_Avoid_: broken comment, orphan, deleted comment

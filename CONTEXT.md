# ReviewFox

Local clipboard-oriented code review over a Git two-tree comparison: browse structured diffs, attach draft comments to positions, export plain text for pasting elsewhere.

## Language

**Repository**:
A local Git worktree root identified by its canonical absolute path (symlinks resolved).
_Avoid_: project, clone, repo path (unspecified canonicalization), working copy

**Workspace**:
The user's current place in the app: which Repository is open and which single Entry is selected; not Comparison identity (OIDs). Survives relaunch as a set of Workspaces plus which one is last (persisted only; not a sidebar section) and which are pinned; last is opened on launch, and the selected Entry on launch is Branch Browser even when the previous selection was an MR or the Worktree. May also persist a last MR Entry label for return after the selected Entry is a Branch Browser selection—that memory is not the selected Entry and does not drive Comparison until the user selects an MR Entry again.
_Avoid_: session, last opened (as a domain term), MRU, concurrent Entries, entry mode

**Entry**:
How the user arrives at a Comparison inside a Workspace: a Branch Browser selection, a forge Merge Request (MR) / Pull Request label, or the Worktree. Exactly one Entry is selected in a Workspace at a time. An MR Entry may carry read-only remote context—title, description, author, status, source/target branch labels, and forge check state (pipeline / approval)—to help understand the commits; that context is not Comparison identity and is never a publish target. Discussion threads are out of this context for now. MR Entries for a Repository are discovered from the forge (project MR list) and may also be opened by URL/IID; the Comparison OID pair for an MR Entry is the forge-reported diff pair (`diff_refs`), not branch-tip guesswork. Typical use: Branch Entry for reviewing one's own work, MR Entry for reviewing someone else's—capability is not restricted by that story.
_Avoid_: review mode, session mode, PR/MR as Comparison identity, workflow (as a domain type), published discussion, simultaneous selection of more than one Entry

**Pin**:
A user-pinned Workspace shown in the Pin section and excluded from the Repositories list; not Comparison identity.
_Avoid_: favorite, bookmark, starred

**Comparison**:
A reviewable surface identified by `(repository, base, head)`. For a commit Comparison, base and head are commit object IDs; the reviewable content is the trees those commits name. For a root commit there is no base commit, so the base is the empty tree (every path is an addition). For a Worktree Comparison, base is the checkout's HEAD commit and head is the Worktree (on-disk contents), not a commit object ID. The same repository and the same HEAD commit are the same Worktree Comparison; a new HEAD commit is a different one. Its label is the HEAD commit's short id and `worktree` (`e7a2ab7..worktree`), which is also the Export header; a branch name is not part of the label. Branch names and MR metadata are Entry labels, not identity. Diff is always over this two-tree pair (possibly folded from a commit selection), never “the MR” or a hash of the dirty checkout as another identity.
_Avoid_: commit range (as identity), MR, branch tip (as identity), tree OID pair (as identity), per-commit review surface (as a second identity), content hash of the dirty checkout (as identity)

**Branch Browser**:
A read-only local branch/commit chooser that changes the displayed commit history and folds a default Comparison—without checking out or modifying the checkout. One kind of Entry into a Comparison; not the only Entry (MR labels, Worktree).
_Avoid_: branch switcher, checkout picker, the whole app shell, Comparison identity, LoadedComparison

**Worktree**:
An Entry for a Repository's current checkout against that checkout's HEAD: one on-disk surface (staged and unstaged together, unignored untracked paths as additions, gitignored paths omitted, unmerged paths as on-disk text), not the branch selected in the Branch Browser, and not a commit. Its Comparison is `(repository, that HEAD commit, the Worktree)`.
_Avoid_: local changes, dirty files, working copy, unpushed commits, staged and unstaged as two review surfaces, conflict view

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
The position relationships for one file under ViewOptions: an ordered list of equal (paired preimage/postimage line numbers), insert (postimage lines plus the preimage-side insertion point between two lines, or file start/end), delete (preimage lines plus the postimage-side deletion point), or replace (one change block of preimage lines and postimage lines, including a same-line textual edit; never a delete of the preimage line plus an insert of the postimage line). Pairwise preimage-line→postimage-line only when that refinement is reliable—never implied by padding alone.
_Avoid_: virtual row (view projection), line matrix (rendering), mapping (vague), 位置关系 (as a second domain object beside Alignment), change (as a fourth Alignment op)

**Anchor**:
The attachment of a DraftComment to a place in a Comparison: either a file-level attachment (path only, no line span) or a line attachment (path, side preimage/postimage, line span, optional owning Hunk). v1 keys files by path only—no rename crossing. Not a ChangedPath (changeset listing).
_Avoid_: position (vague), location, cursor

**UnresolvedAnchor**:
An Anchor that could not be reliably re-located after Comparison or ViewOptions change; still kept on the Review but excluded from default Export. May carry at most one SuggestedAnchor.
_Avoid_: broken comment, orphan, deleted comment

**UI Font**:
The user-chosen family and size for UI text: labels, buttons, lists, fields. Family falls back to the default when the stored one is not installed; size is the body size other UI text sizes keep their offset from.
_Avoid_: app font, system font, sans font

**Code Font**:
The user-chosen family and size for code: Diff text, Diff line numbers and monospace chrome/meta text (hashes, paths, section headers). Only Diff text follows its size; per-pane A−/A+ override that size for the session only.
_Avoid_: mono font, buffer font, editor font, diff font (as a separate setting)

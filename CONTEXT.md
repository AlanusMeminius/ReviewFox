# ReviewFox

Local code review over a Git two-tree comparison: browse structured diffs, attach local comments through Anchors, export plain text for pasting elsewhere, and manage their Publications on supported forge MR Entries.

GitLab Publication scope is agreed in ADR-0018. Local persistence and Publication creation, editing/deletion, batch publishing and uncertain-result recovery are implemented with controlled tests; [real-instance acceptance remains pending](docs/gitlab-publication-acceptance.md). Review persistence stores credential-free originating targets, local comments, Anchors and creation context; target-scoped Publication records retain native correspondence and pending operations. Product direction and deferred capabilities are tracked in [docs/product-direction.md](docs/product-direction.md).

## Language

**Repository**:
A local Git worktree root identified by its canonical absolute path (symlinks resolved). Git's worktree is this checkout directory, not an Entry.
_Avoid_: project, clone, repo path (unspecified canonicalization), working copy, Worktree

**Workspace**:
The user's current place in the app: which Repository is open and which single Entry is selected; not Comparison identity (OIDs). Survives relaunch as a set of Workspaces plus which one is last (persisted only; not a sidebar section) and which are pinned. Opening a Workspace restores its selected Entry, which is an MR Entry label when one is selected and Branch Browser otherwise; last is the Workspace opened on launch. Uncommitted is not a persisted selected Entry, so an open never restores it. May also persist a last MR Entry label for return after the selected Entry is Branch Browser—that memory is not the selected Entry and does not drive Comparison until the user selects an MR Entry again.
_Avoid_: session, last opened (as a domain term), MRU, concurrent Entries, entry mode

**Entry**:
How the user arrives at a Comparison inside a Workspace: a Branch Browser selection, a forge Merge Request (MR) / Pull Request label, or Uncommitted. Exactly one Entry is selected in a Workspace at a time. An MR Entry may carry read-only remote context—title, description, author, status, source/target branch labels, and forge check state (pipeline / approval)—to help understand the commits; that context is not Comparison identity and is never a publish target. The context arrives only together with the `diff_refs` Comparison; a failed open leaves the Entry without it. Discussion threads are out of this context for now. MR Entries for a Repository are discovered from the forge (project MR list) and may also be opened by URL/IID; the Comparison OID pair for an MR Entry is the forge-reported diff pair (`diff_refs`), not branch-tip guesswork. Typical use: Branch Entry for reviewing one's own work, MR Entry for reviewing someone else's—capability is not restricted by that story.
_Avoid_: review mode, session mode, PR/MR as Comparison identity, workflow (as a domain type), published discussion, simultaneous selection of more than one Entry

**MR Inbox**:
A cross-Repository discovery surface for GitLab MRs, filtered by Repository and recent update time. Its selected preview is not a selected Entry or Comparison; opening a result selects that Repository's MR Entry.
_Avoid_: Workspace, Review, MR Entry (for the preview itself)

**Pin**:
A user-pinned Workspace shown in the Pin section and excluded from the Repositories list; not Comparison identity.
_Avoid_: favorite, bookmark, starred

**Comparison**:
A reviewable surface identified by `(repository, base, head)`. For a commit Comparison, base and head are commit object IDs; the reviewable content is the trees those commits name. For a root commit there is no base commit, so the base is the empty tree (every path is an addition). For an Uncommitted Comparison, base is the checkout's HEAD commit and head is the on-disk tree, not a commit object ID. The same repository and the same HEAD commit are the same Uncommitted Comparison; a new HEAD commit is a different one. Its label is the HEAD commit's short id and `uncommitted` (`e7a2ab7..uncommitted`), which is also the Export header; a branch name is not part of the label. Branch names and MR metadata are Entry labels, not identity. Diff is always over this two-tree pair (possibly folded from a commit selection), never “the MR” or a hash of the dirty checkout as another identity.
_Avoid_: commit range (as identity), MR, branch tip (as identity), tree OID pair (as identity), per-commit review surface (as a second identity), content hash of the dirty checkout (as identity)

**Branch Browser**:
A read-only local branch/commit chooser that changes the displayed commit history and folds a default Comparison—without checking out or modifying the checkout. One kind of Entry into a Comparison; not the only Entry (MR labels, Uncommitted).
_Avoid_: branch switcher, checkout picker, the whole app shell, Comparison identity, LoadedComparison

**Uncommitted**:
An Entry for a Repository's current checkout against that checkout's HEAD: one on-disk surface (staged and unstaged together, unignored untracked paths as additions, gitignored paths omitted, unmerged paths as on-disk text), not the branch selected in the Branch Browser, and not a commit. Its Comparison is `(repository, that HEAD commit, the on-disk tree)`.
_Avoid_: worktree, working copy, local changes, dirty files, Changes, unpushed commits, staged and unstaged as two review surfaces, conflict view

**Review**:
A user's ongoing work against one Comparison, holding draft comments and surviving reopen of the same Comparison. Independent of whatever Entry currently drives the main window's Comparison until the user explicitly opens that Comparison for review again.
_Avoid_: session (unless talking implementation), review result, review content (use Export), live-linked review

**DraftComment**:
A locally authored, editable comment attached through an Anchor to a file or a selected line range within one Comparison, rather than to an entire change block. May have a Publication on its Review's originating forge MR; its local Anchor continues to describe the Comparison the author reviewed. The local comment and its remote counterpart have distinct lifecycles: deleting an unpublished DraftComment is local, while deleting one with a Publication must also account for the remote deletion.
_Avoid_: note, annotation, discussion, published comment

**Export**:
A plain-text projection of a Review for the clipboard, primarily for a coding agent to read. Each DraftComment's target (side and selected line range) and body are distinct from its related change context: unified diff excerpts may include both sides without extending the Anchor, and comments on the same block share one excerpt rather than repeating it. Context reflects the file versions the author saw when creating the comment, including actual whitespace changes, rather than later on-disk edits. It is not the full patch.
_Avoid_: review content, review result, report, full patch (as the default Export)

**Publication**:
The remote counterpart of a DraftComment on the forge MR from which its Review originated. First publication is explicit; saving a published comment's body or deleting it synchronizes that change with its remote counterpart. Only full GitLab MR diff versions, including historical versions, have a Publication target in the first release. Branch Browser and Uncommitted Reviews cannot be manually bound to an MR. The comment refers to the version actually reviewed: GitLab may track its remote attachment into a newer version or retain it as outdated, without changing the local Review's Comparison or Anchor. Outdated does not mean resolved. The first release reads only counterparts ReviewFox created; it does not import other remote comments or threads.
_Avoid_: Export, publishing to the currently selected MR, manual MR binding, outdated as a synonym for resolved

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

**TextSelection**:
A character span on one side (preimage or postimage) of the current ChangedPath's diff text. It is not an Anchor and does not attach a DraftComment.
_Avoid_: caret, cursor, line selection, Anchor

**Identifier**:
A maximal run of Unicode letters, digits, and `_`, excluding Han, Hiragana, Katakana, and Hangul. Bounds a double-click TextSelection and an OccurrenceHighlight.
_Avoid_: wrap word, token

**OccurrenceHighlight**:
The other visible whole-identifier occurrences, on both sides of the current ChangedPath, of a TextSelection that is exactly one Identifier. Folded lines are outside it.
_Avoid_: word mark, find hit, search occurrence

**UnresolvedAnchor**:
An Anchor that could not be reliably re-located after Comparison or ViewOptions change; still kept on the Review but excluded from default Export. May carry at most one SuggestedAnchor.
_Avoid_: broken comment, orphan, deleted comment

**UI Font**:
The user-chosen family and size for UI text: labels, buttons, lists, fields. Family falls back to the default when the stored one is not installed; size is the body size other UI text sizes keep their offset from.
_Avoid_: app font, system font, sans font

**Code Font**:
The user-chosen family and size for code: Diff text, Diff line numbers and monospace chrome/meta text (hashes, paths, section headers). Only Diff text follows its size; per-pane A−/A+ override that size for the session only.
_Avoid_: mono font, buffer font, editor font, diff font (as a separate setting)

**Software Theme**:
The light or dark appearance of the application UI: its surfaces, labels, fields, controls, scrollbars, and window material. Independent of the Code Theme selected for Diff.
_Avoid_: dark mode (as a name for code colors), Code Theme

**Code Theme**:
One palette for Diff code: the paper behind unchanged lines, the line-number color, syntax foregrounds, the band fills behind added, deleted, and replaced lines (including the stronger mark on words that differ inside a replace), search-hit marks, the selection wash, and comment markers. A light theme and a dark theme that share a name are two Code Themes; that shared name is not something the user selects.
_Avoid_: theme family, brand (as a selectable object), color scheme pair, Software Theme

**Code Theme Pairing**:
Which Code Theme is in effect while the Software Theme is light, and which is in effect while it is dark. The two choices are independent.
_Avoid_: theme family, pairing by shared name

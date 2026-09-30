# The third Entry kind is named Uncommitted

The Entry in ADR-0013 is named Uncommitted. Worktree stays Git's name for the checkout directory a Repository identifies. The Comparison label suffix in ADR-0014 is `uncommitted` (`{HEAD short}..uncommitted`), in the title bar and the Export header. Behavior in ADR-0013 and ADR-0014 is unchanged. Geometry files written before the rename store this Comparison under the key `worktree`; that key still loads.

**Considered options:** keep the name Worktree and disambiguate in the glossary; call the Entry working copy, dirty, or Changes. Rejected: Worktree already names the checkout, working copy is that same directory, dirty is a state of an Entry that still exists when the checkout is clean, and Changes is the path-list island on every Entry.

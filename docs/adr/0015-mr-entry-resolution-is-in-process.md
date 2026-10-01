# MR Entry resolution is in-process

**Later decision:** ADR-0018 removes the global read-only-forge restriction for Publication. This ADR's in-process MR resolution and seams remain; publication is a separate boundary and does not turn Entry navigation into a write operation.

Opening an MR Entry resolves its read-only forge context and its `diff_refs` Comparison together; a failed open leaves the Entry without that context. The resolution is one in-process module. The UI thread only applies a ready value onto the Branch Browser and the Workspace, or a failure (a user-visible message and an optional Settings target). Under the module, the forge seam is three ordered steps—resolve the project and the remote URL, fetch the MR, list commits—and the object-fetch seam is a single call with the deduped commit ids plus `diff_refs` base, head, and start when present. Each seam has a production adapter (GitLab HTTP; system git, ADR-0009) and an in-memory adapter, so tests exercise order and short-circuit through the module's interface. ADR-0006 stays: the Comparison is still `diff_refs`, and the forge stays read-only.

**Considered options:** one outer adapter whose test double returns a canned ready value or failure. Rejected: the short-circuit between steps would hide inside that adapter, and the module interface could not see it.

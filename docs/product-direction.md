# ReviewFox product direction

The product direction is established: review code locally with a capable diff UI, keep local comments and Export useful, and complete the comment lifecycle on the originating code-hosting MR/PR. GitLab is the first integration; GitHub and Bitbucket are intended later platforms. Remaining discussion refines interaction, compatibility, and recovery details, rather than restarting the product definition.

## Benchmarks and architectural continuity

GitLab is the behavioral reference for version-aware diff comments, editing/deletion, and outdated discussions. tuicr demonstrates a local review tool publishing through forge adapters. Its source is cloned at `/Users/alanus/Dev/ThirdParty/tuicr`, inspected at `9175dc95b97a0a7dd290d43d38385745a7fb7d40`.

Use those references selectively: do not copy tuicr's aggregate GitHub-shaped IDs, whole-batch retry after partial success, or substitution of latest version refs for reviewed line positions. GitLab native position tracking is a useful server capability; it does not require the local detached Review to move with the MR.

Comparison remains `(repository, base, head)`. Entry determines how the user arrives; Publication has an explicit originating forge target. Local Branch Browser and Uncommitted have no corresponding remote publication model and will never be manually bound to an MR.

## Agreed first release: GitLab Publication

- Publish local DraftComments explicitly. After publication, saving the body and deleting the comment synchronize the corresponding remote change.
- Publish only from full GitLab MR diff versions, including historical versions. Submit the version actually reviewed and follow GitLab's tracking/outdated behavior.
- Support existing single-side single-line and single-section multiline selections on added, removed, and unchanged lines. Cross-section ranges are excluded for now.
- Read only counterparts created by ReviewFox, including changes/deletions made to those counterparts through the website. Detect divergent local/remote edits rather than silently overwrite them.
- Persist comments, correspondence, and pending work; display failed or uncertain operations explicitly and do not blindly repeat creates after partial success.
- Keep a narrow publishing boundary, implement only GitLab, and preserve room for later platform-specific adapters.

Decision: [ADR-0018](adr/0018-mr-comment-publication-preserves-reviewed-version.md). Implementation scope: [spec](../.scratch/gitlab-comment-publishing/spec.md). Research: [reference investigation](../.scratch/gitlab-comment-publishing/research.md).

## Follow-up directions

These entries preserve discussed options and their benchmarks. “Agreed direction” expresses product intent; “candidate” records an option, not an implementation promise or a scheduled release. Only the first-release scope above is authorized for implementation planning.

| Capability | Direction status | Reference and intended shape | Remaining decision / tracker |
| --- | --- | --- | --- |
| GitHub support | Agreed direction, later | MR/PR-originated review and comment lifecycle, with GitHub's native publication model | Authentication, review vs comment operations, positioning and migration. [09](../.scratch/gitlab-comment-publishing/issues/09-github-support.md) |
| Bitbucket support | Agreed direction, later | Same local review continuity through a platform adapter | Cloud vs Data Center, authentication and API semantics. [10](../.scratch/gitlab-comment-publishing/issues/10-bitbucket-support.md) |
| Import own website-created comments | Candidate, deferred | Continue editing one's comments across website and app | Historical placement, adoption into a local Review, and ownership. [11](../.scratch/gitlab-comment-publishing/issues/11-import-own-remote-comments.md) |
| Full remote discussions | Candidate, deferred | Read/reply to threads and reflect resolution state, like GitLab | UI placement, replies, permissions and resolved/outdated distinction. [12](../.scratch/gitlab-comment-publishing/issues/12-remote-discussions.md) |
| MR commit-specific comments | Candidate, deferred | GitLab has MR commit-comment context; distinguish it from a full MR diff | Single commit vs arbitrary subset; appropriate target semantics. [13](../.scratch/gitlab-comment-publishing/issues/13-commit-specific-publication.md) |
| Private remote drafts / submit review | Candidate, deferred | GitLab draft notes; tuicr uploads them for website submission | Which drafts the app owns; never bulk-publish unrelated website/client drafts. [14](../.scratch/gitlab-comment-publishing/issues/14-remote-drafts.md) |
| File-level comments | Candidate, deferred | GitLab supports a file position; local Anchor has a file variant but no creation UI | Entry point, server-version compatibility and rendering. [15](../.scratch/gitlab-comment-publishing/issues/15-file-comments.md) |
| Cross-section multiline ranges | Deferred, exploratory | GitLab website restricts ranges across `@@` sections | REST acceptance and rendering require verification; do not silently split/collapse. [16](../.scratch/gitlab-comment-publishing/issues/16-cross-section-ranges.md) |
| OAuth / multiple connections | Candidate, deferred | ADR-0006 deferred OAuth; additional platforms may require richer connection handling | Account selection, host binding and migration. [17](../.scratch/gitlab-comment-publishing/issues/17-connections-and-oauth.md) |

## First-release detail decisions and remaining acceptance work

The first-release product details were confirmed through the design interview. Tickets 01–07 are implemented and controlled tests pass; actual server/version and website behavior remain [pending live acceptance](gitlab-publication-acceptance.md). The agreed behaviors and remaining compatibility evidence are:

- Single-comment Publish and Publish all drafts (N) are implemented. Definite failures retain visible records and require manual retry after network recovery/restart.
- Conflict behavior is agreed: adopt website-only edits; simultaneous edits offer Adopt website body / Overwrite with local body; website deletion preserves local work and requires explicit republishing. Show a remotely changed body before reconfirming local deletion.
- Refresh on open, manually and before update/delete; no periodic polling. Unsaved inputs survive refresh.
- Compatibility direction is agreed: validate the actual configured self-managed instance/version first and retain standard GitLab.com API compatibility. Historical/multiline/context positions still need real-instance acceptance; no untested blanket minimum-version promise.
- Lost-create recovery uses a hidden operation marker, omitted from local editing/Export. A unique match recovers correspondence; no match remains uncertain. Offer Check again, View on GitLab, and explicit Republish with duplicate-risk confirmation. Multiple matches show candidates without automatically selecting or deleting.

The design contract is complete in the [spec](../.scratch/gitlab-comment-publishing/spec.md). Eight approved end-to-end tickets now track implementation: [01](../.scratch/gitlab-comment-publishing/issues/01-reopen-mr-review.md) starts with a recoverable MR Review; [07](../.scratch/gitlab-comment-publishing/issues/07-recover-uncertain-operations.md) completes uncertain-result recovery; [08](../.scratch/gitlab-comment-publishing/issues/08-validate-self-managed-gitlab.md) performs real-instance acceptance. Actual version/test-environment setup remains an acceptance prerequisite, not an undecided product direction. The earlier layered tasks are [archived](../.scratch/gitlab-comment-publishing/archive-layer-tickets/README.md). Later candidates remain separate from the confirmed v1 scope.

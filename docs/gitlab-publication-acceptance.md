# GitLab Publication acceptance

**Status: controlled checks passed; real-instance acceptance not verified.** Report date: 2026-10-02. Application code baseline: `350ff93` (tickets 01–07 and whole-integration review corrections). This report does not complete ticket 08's live criteria.

The intended primary baseline is the configured self-managed host `gitlab.lan.alanusmeninius.com`. Its actual GitLab version, edition, reachable test MR, test account and permitted cleanup scope have not been established. The test-MR question has no answer yet. No private-instance requests, keychain/credential inspection, app launch or live writes were performed for this report. No minimum GitLab version or GitLab.com live compatibility is claimed.

## Completed controlled verification

At the application baseline above, `cargo check`, `git diff --check`, the two-axis implementation and whole-integration reviews and `cargo test -- --test-threads=1` passed. The full suite reported **500 passed, 0 failed, 4 ignored**; the focused Publication suite reported **39 passed**. The four ignored cases are existing manual syntax/diff timing benchmarks, not skipped Publication checks.

The tests use fake HTTP credentials/responses and isolated temporary Review/Publication stores. They exercise public Publication operations and OpenReview persistence, including delayed requests and cancellation. The native UI is implemented and compiles; these checks do not prove actual macOS interaction, GitLab website rendering or a particular server's API behavior.

| Controlled behavior | Representative evidence in [Publication tests](../src/publication.rs) |
| --- | --- |
| Explicit creation, hidden local marker, durable native IDs and target isolation | `explicit_publish_creates_single_line_with_marker_and_reopens_without_duplicate`; `unchanged_lines_use_both_raw_patch_coordinates_and_targets_keep_separate_receipts` |
| Added/deleted/context ranges, offsets, historical refs, rename/whitespace, collapsed and limited data | `publish_preserves_added_deleted_and_context_range_endpoints_in_gitlab_raw_section`; `expanded_context_ranges_use_each_gap_offset_and_validate_both_captured_blobs`; `renamed_whitespace_only_historical_ranges_use_original_refs_and_server_tracked_placement`; `collapsed_version_retrieval_stays_pinned_and_reports_distinct_file_limits` |
| Same-native-ID edits/deletes, website conflicts, active input and newer local work | `conflicting_saved_bodies_require_explicit_adoption_and_refresh_preserves_active_editor`; `deletion_queued_during_create_preserves_receipt_and_never_removes_newer_shared_work` |
| Mixed success/failure/unknown/unsupported batch, explicit retry and close/cancel | `batch_mixed_outcomes_retry_only_definite_failure_and_preserve_each_draft`; `batch_cancellation_preserves_inflight_receipt_latest_edit_and_queued_delete`; `cancelling_during_position_read_never_starts_a_create_after_window_close` |
| Exact author/marker pagination, zero/unique/multiple matches, withdrawal of unsent deletion intent, explicit duplicate-risk confirmation | `checking_lost_create_pages_exact_marker_author_and_restores_b_without_syncing_c`; `zero_and_multiple_matches_require_duplicate_confirmation_and_retain_old_attempts` |
| Read-only uncertain update/delete checks, B versus newer C, verified absence and late reads | `checking_unknown_update_confirms_sent_b_preserves_c_or_exposes_website_conflict`; `checking_unknown_delete_requires_verified_absence_and_keeps_reads_readonly`; `verified_absence_after_lost_update_keeps_sent_evidence_and_requires_explicit_recreate`; `a_late_check_for_the_old_native_comment_cannot_modify_a_recreated_counterpart` |

The remaining local Review, Anchor and Export regression tests passed as part of the full suite. This is controlled evidence, not a substitute for the live observations below.

## Live acceptance gaps

Every row below is **not verified on the actual server**; no live failures or server-specific limitations have yet been observed.

| Required observation | Missing evidence |
| --- | --- |
| Primary self-managed baseline and standard API compatibility | Actual tested version/edition, authorized project/MR and stable author ID; no GitLab.com live run |
| Ordinary and multiline added/deleted/context positions | Server accepts payloads and website shows the intended side and both endpoints, including unchanged endpoint classification |
| Rename, ignore-whitespace and expanded context | Actual line-code/path behavior and server unfolding window; canonical historical blob retrieval |
| Complete historical versions after MR advance | Actual three-SHA version acceptance, tracking/outdated behavior, invalid historical position errors and independent resolution state |
| Collapsed, truncated, empty, rename-only or too-large diff responses | Actual version-pinned unified retrieval behavior and concrete limit responses where reproducible |
| Published body lifecycle and native UI | Same note ID on saves/deletes, website-only adoption/deletion, conflict choices, unsaved input, reopen and captured target isolation |
| Marker rendering and recovery | Marker retained in raw note body, hidden in website rendering, preserved by app updates; actual marker removal/copy and paginated recovery |
| Batch and manual recovery | Live per-item outcomes and confirmed IDs; combine live success with existing controlled faults for response loss, close/restart, read-only checking and explicit retries |

## Implemented scope and limits

Publication targets only captured full GitLab MR Reviews; local branches, Uncommitted and commit subsets have no Publish target or manual binding. Local authored work remains Comparison-owned; correspondence is target-scoped. GitHub and Bitbucket remain future directions.

Ranges stay on one side and within one verified GitLab raw section. Cross-section selections are rejected without splitting, shortening or relocating. Expanded context requires verification against both captured blobs and the adapter's bounded unfolding window. Version-pinned unified retrieval is attempted for omitted collapsed data; missing, truncated or limited historical text retains the draft and Export with an explanation. File-level publication is deferred. These are implemented safeguards, not measured server-version support claims.

Unknown outcomes never imply failure or automatic replay. When an uncertain create also has an unsent deletion intent, Keep comment explicitly withdraws that intent while preserving uncertainty and recovery evidence; it restores the separate duplicate-risk confirmation before Republish. Sent or uncertain deletions cannot be withdrawn this way. Markers can be edited/copied and are not a server uniqueness constraint; republishing can duplicate an earlier create. Known-note checks observe current state, not proof of which request caused it. The pre-read before PUT/DELETE is not an atomic conditional write. Remote placement, outdated availability, resolution and body synchronization are independent. A missing outdated field is shown as unavailable, not inferred from resolution.

## Isolated live procedure

Before starting, record the explicitly designated project/MR URL, stable test account ID, permission to create/edit/delete test comments, permission for fixture commits/pushes if needed, and the exact cleanup scope. Configure the matching host and PAT through the app's intended Settings flow; do not dump the PAT/keychain or redirect a production credential to a fault stub. Authentication alone does not establish MR write permission. Use a test PAT with `api` scope for writes; verify `read_api` browsing separately if available.

1. Record the application commit, date, actual server version (GitLab Help or an authorized metadata read), project/MR/account, full version IDs and captured base/start/head SHAs. Build a small isolated fixture with added, deleted and unchanged lines, changed offsets, two raw sections, an expandable gap, a rename and a whitespace-only change. Preserve two full MR versions.
2. In the app, create/edit/delete unpublished drafts and verify no website counterpart. Publish single lines and supported ranges on each side, including expanded context and rename/ignore-whitespace display. Inspect exact website endpoints and native discussion/note IDs. Try a cross-section range and confirm it remains local with a reason. Change the main-window target and verify an open Review still targets its captured MR.
3. Advance the fixture MR after capturing the first Review, then publish that complete historical version. Observe tracking and an outdated attachment where the fixture produces them; preserve the original local Anchor. Inspect resolution separately. Try an invalid/unavailable historical position and record the actual error. Record collapsed/limited cases only if observed or reproducibly prepared.
4. Save a published body and verify its native ID is unchanged. Modify/delete it on the website, then reopen or refresh. Test active unsaved input, simultaneous local/website edits, both conflict choices and reconfirmation before deleting a changed website body. Website deletion must retain local work until explicit Publish recreates it.
5. Inspect only the test note's raw body and rendered website comment: the exact marker must persist in raw content, be hidden in rendering and absent from local editing/Export. Verify app updates preserve it. Remove/copy a marker on controlled notes; zero/multiple candidates must stay uncertain and offer inspection. Confirm Republish warns about duplicates and keeps earlier attempt evidence.
6. Exercise Publish all drafts and Cancel/close/reopen. Confirm successful native IDs do not repeat and unsupported/failed/unknown comments retain individual results. Use a separate permitted read-only test account/token for an actual permission failure where practical; retain the existing controlled fault tests for unavailable failure conditions rather than altering production permissions or manufacturing outages.
7. Combine the live paths with existing controlled response-loss/cancellation cases. Check again must issue no POST/PUT/DELETE. Recovering sent B must leave newer C pending; a recovered create with pending deletion must require a separate explicit delete action. Capture actual observations rather than claiming the server was made to reproduce every artificial fault.
8. Remove only the comments, copied markers, branches/fixture data explicitly authorized for cleanup. Verify each test comment by recorded native ID; do not delete unrelated discussions or automatically choose ambiguous candidates. Record remaining fixtures and cleanup status. Repeat the local checks if acceptance uncovers code changes.

For each scenario record: live or controlled; application/server versions; target and captured refs; expected/observed result; native IDs and sanitized links/screenshots; passed, failed or not verified; limitation or repair needed; cleanup state. Do not record PAT headers or unrelated private comment bodies. Until this procedure has real observations, live acceptance remains pending and ticket 08 must remain open.

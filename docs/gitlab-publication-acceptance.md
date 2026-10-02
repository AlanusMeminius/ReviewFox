# GitLab Publication acceptance

**Status: controlled checks passed; initial live comment creation verified by API; full live acceptance pending.** Report date: 2026-10-02. Application code baseline: `350ff93` (tickets 01–07 and whole-integration review corrections). This report does not complete ticket 08's live criteria.

The primary baseline is the configured self-managed host `gitlab.lan.alanusmeninius.com`. Initial controlled verification did not access that instance; follow-up fixture provisioning and authenticated read-only verification are recorded below. The observed server is GitLab CE 18.9.1, with test MR `yangkai/test!1` and account `yangkai` (ID 8). Full cleanup and lifecycle acceptance remain pending. No minimum GitLab version or GitLab.com live compatibility is claimed.

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

### Follow-up local verification (2026-10-02)

Revalidated checkout `72656439aa60c5bf3dabd82bb7da45a90bdb42f1` on `feature/gitlab-comment-publication`, with a clean working tree before checks. `cargo fmt --all -- --check`, `git diff --check`, `cargo check --quiet`, `cargo build --quiet` and `cargo test --quiet -- --test-threads=1` passed: **500 passed, 0 failed, 4 ignored**. The local test log is `/tmp/reviewfox-publication-revalidation.log` (temporary, not a durable repository artifact).

An existing `target/debug/reviewfox` process was observed and left running. This follow-up did not launch the newly built executable or exercise native UI interactions; the existing process is not evidence that the new build was exercised. No private-instance requests or live writes were performed. The designated test MR/account and allowed comment creation/edit/deletion/cleanup scope were requested again and remain pending. Ticket 08 remains `needs-info`.

## Live acceptance gaps

### Test fixture provisioned (2026-10-02)

The user designated `http://gitlab.lan.alanusmeninius.com/yangkai/test.git` and requested a clone and test MR. Clone: `/Users/alanus/Dev/reviewfox-gitlab-test`. Git push successfully created MR **!1**, using Draft title/option, from `reviewfox/publication-test-20261002` into the isolated `reviewfox/publication-base-20261002` branch. Server-returned URL: `http://10.11.6.78:20080/yangkai/test/-/merge_requests/1`; configured-host URL: `http://gitlab.lan.alanusmeninius.com/yangkai/test/-/merge_requests/1`.

Remote refs verified: base `9b8e1cc3fdcddd22e2bb46fd7ec7a080551ea6b9`; source and `refs/merge-requests/1/head` both `7a6183a3c5e7ab1faf4fa000f8afe0e0bffd4f66`. Default `main` remains `e35a33f14bef4b59eccf4770b97f1ce7452dcea5`. These are Git refs, not a verified API `diff_refs` triple. The fixture contains additions/deletions, multiline replacements, separated sections and expandable context, a 96%-similar rename with a changed line, and a whitespace-only change. `git diff --check` passed; the clone is clean. Capture this first full MR version before advancing the source branch for historical-version acceptance.

An unauthenticated MR API GET returned 404, so MR metadata/Draft state was not independently read back. The provisioned MR resolves the missing test-target prerequisite; app credential/account identity, server version, native UI and actual Publication behavior remain unverified. No test comments have been created yet. Retain the fixture branches and MR for testing; ticket 08 remains open.

### User-created comments checked against GitLab (2026-10-02)

After the user reported adding comments and syncing in ReviewFox, read the matching local Review/Publication snapshots and performed authenticated GET requests to the configured host using its existing ReviewFox credential. No credential was printed or persisted, and no comments were changed by this verification. `/version` returned `18.9.1`, revision `95bf6656b5a`, `enterprise: false`; `/user` returned `yangkai`, ID 8. MR !1 is open and Draft. Its API base/start/head refs match the recorded fixture refs above. The running app binary's build identity was not independently established.

| Local comment | Local Anchor | Live note ID | Observation |
| --- | --- | --- | --- |
| 1 | `added.txt`, postimage 1–2 | 22972 | Published; body, author, position and refs match |
| 2 | `deleted.txt`, preimage 1–3 | 22973 | Published; body, author, position and refs match |
| 3 | `lines.txt`, postimage 20–22 | 22974 | Published; body, author, position and refs match |
| 4 | `rename-after.txt`, postimage 2–5 | 22975 | Published; old/new rename paths and unchanged range endpoints match |
| 5 | `rename-before.txt`, preimage 2–7 | None recorded | Local `Unsupported`: expanded range exceeds the current bounded unfolding rule; draft retained |
| 6 | `whitespace.txt`, postimage 2 | 22976 | Published; body, author, position and refs match |

Paths in the table are under `reviewfox-fixtures/`. For all five published comments, a fresh GET of the stored discussion returned the stored note ID and the exact persisted remote position. Each raw body equals the local body plus its expected operation marker. Local bodies contain no marker, and no pending edits/deletions or conflicts are recorded. All five notes are unresolved; the responses omit outdated status. The 2–7 expanded range is rejected locally by the existing end-line-minus-three rule; this is an application limit, not an observed server rejection. The accepted 2–5 range gives live evidence for a shorter expanded unchanged range on a rename.

This establishes initial API publication for these five examples and durable local receipts. Website visual rendering/marker hiding, duplicate scanning, edits/deletes, conflicts, restart behavior, historical versions and uncertain recovery were not exercised in this check. Keep all six local comments and the five remote notes for follow-up testing; ticket 08 remains open.

### Shortened range and saved website DOM (2026-10-02)

A later read of the same local Review shows comment 5 removed and comment 7 created with body `test`, anchored to `rename-before.txt` preimage 2–4. Comment 7 is Published as note **22977**, discussion `3e29fe33a131d71c42e93b7a95829a3c6e984525`. A fresh authenticated discussion GET confirms the body plus exact operation marker, author 8, reviewed refs, old/new rename paths, and unchanged endpoints 2–4 all match its local receipt. No pending edit/delete/conflict is recorded. The old unsupported Publication record for comment 5 still exists on disk, but no active DraftComment 5 remains; it is not an outstanding draft in the current Review. Six active local comments now have published receipts.

Inspected the user-supplied Safari archive `/Users/alanus/Downloads/Draft: ReviewFox GitLab Publication 验收测试 (!1) · Merge requests · yangkai : test · GitLab.webarchive`, whose main URL ends in `/merge_requests/1#note_22977`. Parsed its saved HTML/DOM without executing archived scripts. The saved discussion markup contains all six notes (22972–22977), their expected visible bodies and corresponding diff text. Multiline labels are `+1 to +2`, `-1 to -3`, `+20 to +22`, `2 to 5`, and `2 to 4`; the whitespace note is shown after new line 2. The new `test` note explicitly displays `Comment on lines 2 to 4`, with matching unchanged rename context. Unchanged lines show paired old/new coordinates rather than a plus/minus side label.

No `reviewfox:operation` marker occurs in the saved main HTML; the six rendered note-body elements contain only the expected visible text. Together with raw API marker checks, this provides evidence that operation markers are hidden in this saved website view. This is archived DOM evidence, not a fresh browser screenshot or verification of interactive layout, edit/delete controls, or later website changes. No remote comments or local app records were modified during this check.

### Website-edit marker leakage and correction (2026-10-02)

The user reported successful app-to-website body edits, app-to-website deletion, and website-to-app body refresh. These are user-observed live results; same-native-ID edit tracing and detailed deletion records were not independently captured in this step. The website-to-app path exposed a defect: note 22977's operation marker had become `<!--reviewfox:operation=5baf6511-3fdf-4c87-83f3-ab668b9bac82-->`, without the spaces emitted by ReviewFox. The local comment, confirmed receipt body and observed website body all contained that compact marker. The existing literal replacement recognized only the spaced form.

Corrected `PublicationRecord::visible_body` to accept whitespace changes inside markers while requiring the complete current or prior operation ID. Unrelated HTML comments, other IDs, ID suffixes, malformed markers and extra comment content remain intact. Marker recovery association rules are unchanged. Regression coverage first failed on a real Publication read/reconcile path, then passed with the correction; it covers website adoption, cleanup of already-adopted polluted bodies during an ordinary refresh, Export exclusion, persisted reopen and marker matching boundaries. Active editing/conflict safeguards remain in the existing reconciliation path. No live app stores or remote comments were edited by the fix.

Validation of this working-tree fix: **502 passed, 0 failed, 4 ignored** in the serial full suite; formatting, `cargo check`, development build and `git diff --check` passed. The updated binary is built, but the user's running app must use that build and refresh the comment to complete native UI revalidation. Existing unsaved edits remain subject to the normal conflict/input-preservation flow. Full ticket 08 acceptance remains pending.

### Best-effort cleanup after confirmed creation (2026-10-02)

Following the user-confirmed design, an ordinary successful create now saves native IDs before immediately attempting a body-only PUT without the operation marker. Cleanup remains within the original publish task and busy state; errors/unconfirmed responses only produce diagnostics and retain Published. New edits/deletions and cancellation skip unsent cleanup, while results arriving after sent cleanup preserve newer local intent. Single-comment publication now observes window-close cancellation, as batches already did. Known-note body updates omit the marker. Check again, refresh and reopen never trigger cleanup, including after recovery of an unknown create.

Controlled verification: `confirmed_creation_cleans_marker_only_after_saving_native_identity`, `cleanup_failure_keeps_publication_confirmed_and_reopen_does_not_replay_it`, `cancel_edit_or_delete_during_create_skips_unsent_cleanup`, `inflight_cleanup_preserves_newer_work_and_interruption_keeps_the_receipt`, and the updated same-note update/unknown recovery regressions. Publication tests: **45 passed**. Serial full suite: **506 passed, 0 failed, 4 ignored**. `cargo check`, development build, format check and `git diff --check` passed. Logs: `/tmp/reviewfox-cleanup-focused.log` and `/tmp/reviewfox-cleanup-final.log` (temporary local artifacts). Earlier live observations in this report predate this change; the new cleanup behavior still needs native UI/server revalidation. No live test comments were modified during implementation.

The table below lists the broader remaining acceptance gaps; the API, archived-DOM and user-observed results above do not complete every scenario.

| Required observation | Missing evidence |
| --- | --- |
| Primary self-managed baseline and standard API compatibility | CE 18.9.1, `yangkai/test!1`, author 8 now observed; broader API compatibility and GitLab.com remain untested |
| Ordinary and multiline added/deleted/context positions | Server accepts payloads and website shows the intended side and both endpoints, including unchanged endpoint classification |
| Rename, ignore-whitespace and expanded context | Actual line-code/path behavior and server unfolding window; canonical historical blob retrieval |
| Complete historical versions after MR advance | Actual three-SHA version acceptance, tracking/outdated behavior, invalid historical position errors and independent resolution state |
| Collapsed, truncated, empty, rename-only or too-large diff responses | Actual version-pinned unified retrieval behavior and concrete limit responses where reproducible |
| Published body lifecycle and native UI | Same note ID on saves/deletes, website-only adoption/deletion, conflict choices, unsaved input, reopen and captured target isolation |
| Marker rendering, cleanup and recovery | Raw/rendered checks above predate cleanup; verify immediate best-effort removal after native IDs persist, later marker-free updates, cleanup failures and actual marker removal/copy with paginated recovery |
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
5. Inspect only the test note's raw body and rendered website comment: ordinary confirmed creation should remove the marker after saving native IDs, and later app-written body updates must not reattach it. Use controlled response loss/cleanup failures for retained-marker cases: the marker remains hidden in rendering and absent from local editing/Export. Remove/copy a marker on controlled uncertain notes; zero/multiple candidates must stay uncertain and offer inspection. Confirm Republish warns about duplicates and keeps earlier attempt evidence; Check again and reopening never send cleanup writes.
6. Exercise Publish all drafts and Cancel/close/reopen. Confirm successful native IDs do not repeat and unsupported/failed/unknown comments retain individual results. Use a separate permitted read-only test account/token for an actual permission failure where practical; retain the existing controlled fault tests for unavailable failure conditions rather than altering production permissions or manufacturing outages.
7. Combine the live paths with existing controlled response-loss/cancellation cases. Check again must issue no POST/PUT/DELETE. Recovering sent B must leave newer C pending; a recovered create with pending deletion must require a separate explicit delete action. Capture actual observations rather than claiming the server was made to reproduce every artificial fault.
8. Remove only the comments, copied markers, branches/fixture data explicitly authorized for cleanup. Verify each test comment by recorded native ID; do not delete unrelated discussions or automatically choose ambiguous candidates. Record remaining fixtures and cleanup status. Repeat the local checks if acceptance uncovers code changes.

For each scenario record: live or controlled; application/server versions; target and captured refs; expected/observed result; native IDs and sanitized links/screenshots; passed, failed or not verified; limitation or repair needed; cleanup state. Do not record PAT headers or unrelated private comment bodies. Until this procedure has real observations, live acceptance remains pending and ticket 08 must remain open.

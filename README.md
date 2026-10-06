# ReviewFox

Local clipboard-oriented code review over a Git two-tree comparison: browse structured diffs, attach draft comments, and export plain text for pasting elsewhere.

## GitLab MR inbox

Choose **Merge requests** above Pin and Repositories to browse MRs from all added Repositories matching the configured GitLab host, including pinned ones. The titlebar filters by multiple Repositories and update time: today, two days, three days, one week, or one calendar month. Day ranges include today in local time; all MR states are included.

Click an MR to preview its description and checks. Double-click or press Enter to open its existing MR Entry and Comparison. Return through **Merge requests** to keep the filters and selection. Refresh reloads the selected range; failures are reported per Repository so other results remain available.

## GitLab comments

Implementation is covered by controlled tests; actual self-managed GitLab version and website behavior remain [pending live acceptance](docs/gitlab-publication-acceptance.md). No tested minimum server version is currently claimed.

Publish a local DraftComment explicitly from its complete GitLab MR Review. A PAT with `api` scope and permission to comment on that MR enables writing; `read_api` supports browsing.

The Review toolbar offers **Publish all drafts (N)** for its captured MR. It reserves each draft independently and shows each result on that comment. A new batch retries only definite failures; successful, uncertain and known unsupported items are excluded. Cancel or closing the window stops unsent items for manual retry. A request already sent retains its receipt or uncertain result. Reopening never continues a batch. Editing or deleting while a create is in flight preserves the later intent.

Single-side ranges publish within one GitLab raw diff section, preserving their endpoints and the version actually reviewed. Rename paths and whitespace classification come from that version's server diff. Expanded unchanged context is checked against both captured file versions; short ranges within the server's unfolding window are supported. A longer expanded range that cannot be verified as one section stays local with a concrete explanation. Ranges crossing server sections are never split or shortened.

When local rename detection shows separate Add/Delete files, missing verification text is read using the server's old/new paths and the reviewed commits. This leaves the original Anchor, editor and Export context unchanged.

Collapsed text is requested again in unified format for the same version. If GitLab still omits it, or reports a file/line limit or a rename-only file without a text section, the comment and Export remain available locally. Placement and outdated capability are reported separately from body synchronization and resolution; an unavailable outdated field is shown explicitly. The range contract follows the [Discussions API](https://docs.gitlab.com/api/discussions/#parameters-for-multiline-comments) and [current upstream position schema](https://gitlab.com/gitlab-org/gitlab/-/blob/master/app/validators/json_schemas/position.json); context endpoint and rename behavior still need the planned test-instance acceptance check.

Saving a published body updates its existing GitLab note. Opening a Review or clicking Refresh GitLab reads its known counterparts. Website-only edits are adopted; simultaneous local and website edits show both bodies for an explicit choice. Active editor input is preserved. Failed updates require a manual retry; uncertain outcomes retain the sent body and original correspondence. Reopening never sends writes.

First publication includes an operation marker for recovering a lost response. Once the returned native IDs are saved locally, ReviewFox immediately attempts a body-only update to remove the marker as part of the same publish operation. Cleanup failure still leaves the comment Published; it does not trigger a retry or another creation. A queued edit/deletion, batch cancellation or window close skips cleanup that has not started. Later body updates use the same native IDs without adding a marker. Reopening, refreshing and Check again never send cleanup writes, even when they recover an uncertain creation. Markers may remain until a later explicit body update; removing one on the website does not break a known association.

Deleting a published comment checks the same GitLab note first. A changed website body requires confirmation again; a website-deleted note retains local work and needs an explicit Publish to recreate it. Delete failures retain the correspondence for manual retry. A delete requested during a live create or update is saved and serialized after that request; uncertainty blocks another write. Confirmed deletion clears the local comment only when its body has not changed, no editor is active, and no other MR still owns a counterpart. Newer local work is retained.

For an uncertain result, **Check again** only reads GitLab. An uncertain create scans all discussion pages on the captured MR for the exact saved marker and original author. One match restores that native comment; no matches or multiple matches remain uncertain. Candidate links and **View on GitLab** support inspection. **Republish…** requires confirmation that a duplicate may result and retains the earlier attempt for tracking. Markers are editable and copyable content, so this does not guarantee exactly-once creation.

Checking an uncertain update confirms the sent body without declaring a newer local body synchronized; a different website body enters the conflict choices. Checking an uncertain delete requires verified absence. Permission and read errors preserve uncertainty. Verified absence after an uncertain update retains the sent body and old correspondence, allowing a new explicit Publish. Checking never automatically sends a pending update or deletion, including a deletion queued during a recovered create.

If deletion was queued during an uncertain create, **Keep comment** explicitly withdraws the unsent deletion intent while retaining the unknown attempt and candidates. Republish still requires duplicate-risk confirmation. This action cannot cancel a deletion that was already sent.

The read before an update or delete detects ordinary conflicts but is not an atomic conditional write: a website edit can still arrive between reading and writing. Remote placement and resolution are separate from synchronization; outdated status is shown only if the server supplies it.

## Requirements

- macOS
- Rust (see `rust-toolchain.toml`)
- Git

## Build / install

```bash
./install.sh
```

Or build only:

```bash
cargo build --release
```

The GitLab PAT lives in the login keychain. `cargo run` (via
`.cargo/config.toml`) and `./install.sh` sign the build, bundle id
`com.reviewfox.app`.

Set `REVIEWFOX_TEAM_ID` to the Apple Development certificate OU (Team ID).
The private key stays in the login keychain; do not commit a `.p12`. The
first time the app reads the token, click **Always Allow**. Later rebuilds
signed by that certificate should not prompt.

Leave `REVIEWFOX_TEAM_ID` unset to self-sign (`REVIEWFOX_SIGN_ID`, or ad-hoc
`-`). A self-signed build asks for the keychain password again after each
rebuild. If `REVIEWFOX_TEAM_ID` is set but no matching certificate exists,
signing fails.

## License

MIT. See [LICENSE](LICENSE). Third-party file icons: [LICENSE-THIRD-PARTY.md](LICENSE-THIRD-PARTY.md).

# ReviewFox

Local clipboard-oriented code review over a Git two-tree comparison: browse structured diffs, attach draft comments, and export plain text for pasting elsewhere.

## GitLab comments

Publish a local DraftComment explicitly from its complete GitLab MR Review. A PAT with `api` scope and permission to comment on that MR enables writing; `read_api` supports browsing.

Saving a published body updates its existing GitLab note. Opening a Review or clicking Refresh GitLab reads its known counterparts. Website-only edits are adopted; simultaneous local and website edits show both bodies for an explicit choice. Active editor input is preserved. Failed updates require a manual retry; uncertain outcomes retain the sent body and original correspondence. Reopening never sends writes.

The pre-update read detects ordinary conflicts but is not an atomic conditional update: a website edit can still arrive between reading and writing. Remote placement and resolution are separate from synchronization; outdated status is shown only if the server supplies it.

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

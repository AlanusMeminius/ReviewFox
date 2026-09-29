# ReviewFox

Local clipboard-oriented code review over a Git two-tree comparison: browse structured diffs, attach draft comments, and export plain text for pasting elsewhere.

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

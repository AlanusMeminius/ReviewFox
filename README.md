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

To avoid Keychain password prompts on every rebuild when reading the GitLab
PAT, create a local self-signed **Code Signing** certificate named
`ReviewFox Dev` in Keychain Access, then:

```bash
export REVIEWFOX_SIGN_ID="ReviewFox Dev"
```

`cargo run` (via `.cargo/config.toml`) and `./install.sh` both use this
identity with bundle id `com.reviewfox.app`. On first run, click **Always
Allow**; later rebuilds should not prompt.

## License

MIT. See [LICENSE](LICENSE). Third-party file icons: [LICENSE-THIRD-PARTY.md](LICENSE-THIRD-PARTY.md).

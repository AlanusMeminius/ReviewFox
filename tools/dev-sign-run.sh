#!/usr/bin/env bash
# Cargo runner: sign the binary with a stable identity so Keychain ACL
# survives rebuilds. Set REVIEWFOX_SIGN_ID (e.g. "ReviewFox Dev"); unset
# falls back to ad-hoc (-), same as unsigned for ACL purposes.
set -euo pipefail
codesign --force --sign "${REVIEWFOX_SIGN_ID:--}" --identifier com.reviewfox.app "$1" 2>/dev/null || true
exec "$@"

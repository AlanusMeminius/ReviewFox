#!/usr/bin/env bash
# Cargo runner. REVIEWFOX_TEAM_ID selects an Apple Development cert so the
# Keychain requirement survives rebuilds. Unset, the binary is self-signed.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/sign-identity.sh"
codesign --force --sign "$(reviewfox_sign_id)" --identifier com.reviewfox.app "$1"
exec "$@"

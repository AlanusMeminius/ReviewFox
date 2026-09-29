#!/usr/bin/env bash
# Resolves the codesign identity.
# REVIEWFOX_TEAM_ID (certificate OU) selects the Apple Development cert.
# When it is unset, self-sign: REVIEWFOX_SIGN_ID, or ad-hoc "-" if that is
# unset too. The Team ID is not stored in the repo. The private key stays
# in the login keychain.

# Prints a codesign identity. Returns 1 when REVIEWFOX_TEAM_ID is set but
# no matching certificate exists.
reviewfox_sign_id() {
  if [[ -z "${REVIEWFOX_TEAM_ID:-}" ]]; then
    printf '%s\n' "${REVIEWFOX_SIGN_ID:--}"
    return 0
  fi
  local found
  found="$(
    security find-certificate -a -Z -p "$HOME/Library/Keychains/login.keychain-db" 2>/dev/null \
      | python3 -c '
import re, subprocess, sys
team = sys.argv[1]
raw = sys.stdin.read()
for block in re.split(r"(?=SHA-1 hash: )", raw):
    hashed = re.search(r"SHA-1 hash: ([0-9A-F]+)", block)
    start = block.find("-----BEGIN")
    if not hashed or start < 0:
        continue
    subject = subprocess.run(
        ["openssl", "x509", "-noout", "-subject"],
        input=block[start:],
        text=True,
        capture_output=True,
    ).stdout
    if re.search(r"OU=" + re.escape(team) + r"(?![A-Z0-9])", subject):
        print(hashed.group(1))
        raise SystemExit(0)
raise SystemExit(1)
' "$REVIEWFOX_TEAM_ID"
  )" || return 1
  printf '%s\n' "$found"
}

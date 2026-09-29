#!/usr/bin/env bash
# Build a complete "ReviewFox.app" bundle as a build artifact under
# target/release/bundle/, then install it into /Applications and relaunch.
# Run from the repo root: ./install.sh
#
# REVIEWFOX_TEAM_ID selects an Apple Development cert (tools/sign-identity.sh).
# Unset, the bundle is self-signed.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/tools/sign-identity.sh"

APP_NAME="ReviewFox"
BUNDLE="target/release/bundle/$APP_NAME.app"
INSTALLED="/Applications/$APP_NAME.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cargo build --release

# Assemble the bundle entirely from repo sources.
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$BUNDLE/Contents/Info.plist"
cp assets/icon.icns "$BUNDLE/Contents/Resources/icon.icns"
cp target/release/reviewfox "$BUNDLE/Contents/MacOS/reviewfox"
codesign --force --sign "$(reviewfox_sign_id)" --identifier com.reviewfox.app "$BUNDLE"

# Install: quit the running app, then replace the bundle wholesale.
osascript -e "quit app \"$APP_NAME\"" 2>/dev/null || true
sleep 1
rm -rf "$INSTALLED"
ditto "$BUNDLE" "$INSTALLED"

open -a "$APP_NAME"
echo "installed $APP_NAME $VERSION to $INSTALLED"

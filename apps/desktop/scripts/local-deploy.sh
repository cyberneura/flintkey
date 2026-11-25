#!/bin/sh
# Builds, signs and notarizes the .app and puts it in /Applications. Does not stop a running Flintkey.
set -eu

cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo "macOS is required." >&2; exit 1; }
# notarize-app.sh requires a signature with this certificate, so it cannot be changed by an environment variable.
SIGN_IDENTITY="Developer ID Application: Cyberneura K.K. (2YN5TLNQ9J)"
NOTARY_PROFILE="${NOTARY_PROFILE:-cyberneura-notary}"
DEST_DIR="${DEST_DIR:-/Applications}"
DEST_DIR="$(cd "$DEST_DIR" && pwd -P)"
product_name="$(node -p "require('./src-tauri/tauri.conf.json').productName")"
# This is a Cargo workspace, so build output goes to target at the repository root.
target_dir="$(cd ../.. && pwd -P)/target"
bundle="$target_dir/release/bundle/macos/${product_name}.app"
dest="$DEST_DIR/${product_name}.app"
[ "$bundle" != "$dest" ] || { echo "Build and installation directories must differ." >&2; exit 1; }

# Without notarization credentials, fail before spending time on the build.
xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" --output-format json >/dev/null
(
  # Notarize once after the build using the Keychain credentials. Do not let Tauri notarize.
  unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
  APPLE_SIGNING_IDENTITY="$SIGN_IDENTITY" CARGO_TARGET_DIR="$target_dir" \
    pnpm exec tauri build --bundles app --config '{"bundle":{"macOS":{"hardenedRuntime":true}}}'
)
bash scripts/notarize-app.sh "$bundle" "$NOTARY_PROFILE"

# Stage on the same volume, then swap in with mv. On failure, restore the previous version.
staging="$(mktemp -d "$DEST_DIR/.notarized-deploy.XXXXXX")"
cleanup() {
  if [ ! -e "$dest" ] && [ ! -L "$dest" ] && [ -e "$staging/previous.app" ]; then
    if ! mv "$staging/previous.app" "$dest"; then
      echo "Restore the previous app from $staging/previous.app." >&2
      return
    fi
  fi
  rm -rf "$staging"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
ditto "$bundle" "$staging/$product_name.app"
codesign --verify --deep --strict "$staging/$product_name.app"
xcrun stapler validate "$staging/$product_name.app"
if [ -e "$dest" ] || [ -L "$dest" ]; then mv "$dest" "$staging/previous.app"; fi
mv "$staging/$product_name.app" "$dest"
rm -rf "$bundle"
echo "Installed notarized app: $dest"
echo "Quit $product_name from the menu bar and open it again to use the new version."

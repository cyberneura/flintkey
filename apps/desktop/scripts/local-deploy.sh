#!/bin/sh
# .app をビルドして署名・公証し、/Applications に置く。起動中の Flintkey は止めない。
set -eu

cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo "macOS is required." >&2; exit 1; }
# notarize-app.sh がこの証明書での署名を要求するので、環境変数では変えられない。
SIGN_IDENTITY="Developer ID Application: Cyberneura K.K. (2YN5TLNQ9J)"
NOTARY_PROFILE="${NOTARY_PROFILE:-cyberneura-notary}"
DEST_DIR="${DEST_DIR:-/Applications}"
DEST_DIR="$(cd "$DEST_DIR" && pwd -P)"
product_name="$(node -p "require('./src-tauri/tauri.conf.json').productName")"
# Cargo のワークスペースなので、ビルド結果はリポジトリ直下の target に出る。
target_dir="$(cd ../.. && pwd -P)/target"
bundle="$target_dir/release/bundle/macos/${product_name}.app"
dest="$DEST_DIR/${product_name}.app"
[ "$bundle" != "$dest" ] || { echo "Build and installation directories must differ." >&2; exit 1; }

# 公証の認証情報が無いなら、ビルドに時間を使う前に失敗させる。
xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" --output-format json >/dev/null
(
  # 公証はビルドのあとに Keychain の認証情報で 1 回だけ行う。Tauri に公証させない。
  unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
  APPLE_SIGNING_IDENTITY="$SIGN_IDENTITY" CARGO_TARGET_DIR="$target_dir" \
    pnpm exec tauri build --bundles app --config '{"bundle":{"macOS":{"hardenedRuntime":true}}}'
)
bash scripts/notarize-app.sh "$bundle" "$NOTARY_PROFILE"

# 同じボリュームに一度置いてから mv で入れ替える。失敗したら前の版を戻す。
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

#!/usr/bin/env bash
# Prints the version being released. It is written in two places, the Cargo workspace and the desktop
# package.json; they must agree, or the app and its release would carry different numbers.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo_version=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' Cargo.toml)
npm_version=$(node -p "require('./apps/desktop/package.json').version")
if [ -z "$cargo_version" ] || [ "$cargo_version" != "$npm_version" ]; then
  echo "::error::Cargo.toml says '${cargo_version}' and apps/desktop/package.json says '${npm_version}'" >&2
  exit 1
fi
echo "$cargo_version"

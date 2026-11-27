#!/usr/bin/env bash
# Decides whether VERSION may be released now and prints true or false. Exits 1 when it cannot tell
# (an API failure or an unexpected answer), with the reason on stderr.
#
# release.yml calls it twice: in plan (whether to build) and again right before publishing, because
# "Re-run failed jobs" reuses the old plan result. A newer version published in between would
# otherwise be rolled back by `--latest`.
#
# Needs GH_TOKEN, GH_REPO (owner/repo) and VERSION (X.Y.Z).
set -euo pipefail

: "${GH_REPO:?GH_REPO is required}"
: "${VERSION:?VERSION is required}"

# Versions are compared with sort -V, which is only right for X.Y.Z.
semver='^[0-9]+\.[0-9]+\.[0-9]+$'
if ! [[ "$VERSION" =~ $semver ]]; then
  echo "::error::version is not X.Y.Z: '${VERSION}'" >&2
  exit 1
fi

http_status() {
  gh api "$1" --silent --include 2>/dev/null | head -n 1 | awk '{print $2}' || true
}

# A draft counts as not released: a failed run leaves one behind to be filled again, and publish asks
# this with the run's own draft in place. Only 404 means "not found"; reading a rate limit or an outage
# that way would build and publish a released version a second time.
status=$(http_status "repos/${GH_REPO}/releases/tags/v${VERSION}")
case "$status" in
  404) ;;
  200)
    draft=$(gh api "repos/${GH_REPO}/releases/tags/v${VERSION}" --jq '.draft')
    if [ "$draft" != "true" ] && [ "$draft" != "false" ]; then
      echo "::error::could not read whether v${VERSION} is a draft (got '${draft}'). Not guessing." >&2
      exit 1
    fi
    if [ "$draft" = "false" ]; then
      echo "v${VERSION} is already released." >&2
      echo false
      exit 0
    fi
    ;;
  *)
    echo "::error::could not tell whether v${VERSION} exists (HTTP ${status:-none}). Not guessing." >&2
    exit 1
    ;;
esac

# An unreleased version older than the latest release is not released either: publishing uses --latest,
# which would roll the latest back, and the Homebrew tap with it. This happens when a version bump is
# reverted, or when queued runs start in a different order from their pushes.
status=$(http_status "repos/${GH_REPO}/releases/latest")
case "$status" in
  404)
    echo "Releasing v${VERSION} (no release yet)." >&2
    echo true
    exit 0
    ;;
  200) ;;
  *)
    echo "::error::could not read the latest release (HTTP ${status:-none}). Not guessing." >&2
    exit 1
    ;;
esac

latest=$(gh api "repos/${GH_REPO}/releases/latest" --jq '.tag_name')
latest="${latest#v}"
if ! [[ "$latest" =~ $semver ]]; then
  echo "::error::latest release tag is not vX.Y.Z: 'v${latest}'. Not guessing." >&2
  exit 1
fi

if [ "$(printf '%s\n%s\n' "$latest" "$VERSION" | sort -V | tail -n 1)" != "$VERSION" ]; then
  echo "::warning::v${VERSION} is older than the latest release v${latest}; not releasing it." >&2
  echo false
  exit 0
fi

echo "Releasing v${VERSION} (latest is v${latest})." >&2
echo true

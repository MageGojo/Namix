#!/usr/bin/env bash
# Upload an immutable local release, then ask the server-side release manager
# to start the candidate, wait for its private ready signal, atomically switch
# current, and drain the old process.
set -euo pipefail

VERSION=${1:?usage: ops/deploy-release.sh X.Y.Z}
: "${NAMIX_DEPLOY_HOST:?set NAMIX_DEPLOY_HOST (for example deploy@HOST)}"
: "${NAMIX_DEPLOY_ROOT:?set NAMIX_DEPLOY_ROOT (absolute project path on server)}"
: "${NAMIX_DEPLOY_PORT:=3000}"
: "${NAMIX_NX:=nx}"

case "$VERSION" in
  ''|*[!0-9.]*) echo "version must contain only digits and dots" >&2; exit 2 ;;
esac
case "$NAMIX_DEPLOY_PORT" in
  ''|*[!0-9]*) echo "NAMIX_DEPLOY_PORT must be numeric" >&2; exit 2 ;;
esac
case "$NAMIX_DEPLOY_ROOT" in
  /*) ;;
  *) echo "NAMIX_DEPLOY_ROOT must be an absolute path" >&2; exit 2 ;;
esac

LOCAL_RELEASE="$PWD/dist/$VERSION"
[[ -f "$LOCAL_RELEASE/app" ]] || { echo "missing release: $LOCAL_RELEASE" >&2; exit 2; }
[[ -f "$LOCAL_RELEASE/MANIFEST.json" ]] || { echo "missing release manifest: $LOCAL_RELEASE/MANIFEST.json" >&2; exit 2; }
INCOMING="$NAMIX_DEPLOY_ROOT/dist/.incoming-$VERSION"
RELEASE="$NAMIX_DEPLOY_ROOT/dist/$VERSION"

echo "uploading $VERSION; server nx preflight will validate platform and Action seal public key before exec"

ssh "$NAMIX_DEPLOY_HOST" sh -s -- "$NAMIX_DEPLOY_ROOT" "$VERSION" <<'REMOTE_PREPARE'
set -eu
root=$1 version=$2
[ -d "$root" ] || { echo "remote project root missing: $root" >&2; exit 2; }
[ ! -e "$root/dist/$version" ] || { echo "release already exists: $version" >&2; exit 2; }
key="$root/dist/data/storage/action_seal.key"
[ -f "$key" ] || { echo "remote Action seal key missing: $key (provision the shared 0600 key before deploy)" >&2; exit 2; }
mkdir -p "$root/dist/.incoming-$version"
REMOTE_PREPARE

# The live release directory is never a transfer target.
rsync --archive --checksum --delay-updates --delete \
  "$LOCAL_RELEASE/" "$NAMIX_DEPLOY_HOST:$INCOMING/"

ssh "$NAMIX_DEPLOY_HOST" sh -s -- \
  "$NAMIX_DEPLOY_ROOT" "$VERSION" "$NAMIX_DEPLOY_PORT" "$NAMIX_NX" <<'REMOTE_ACTIVATE'
set -eu
root=$1 version=$2 port=$3 nx=$4
incoming="$root/dist/.incoming-$version"
release="$root/dist/$version"
[ -d "$incoming" ] || { echo "incoming release missing" >&2; exit 2; }
[ ! -e "$release" ] || { echo "release already exists" >&2; exit 2; }
mv "$incoming" "$release"
cd "$root"
"$nx" update --ver "$version" --port "$port"
REMOTE_ACTIVATE

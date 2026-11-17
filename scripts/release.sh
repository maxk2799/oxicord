#!/bin/sh
# Build the Mac download and publish it to a download server.
#   OXICORD_SERVER=user@host scripts/release.sh "What changed" "Another change"
# Each argument is one line of release notes. Optional: OXICORD_SSH_KEY, OXICORD_SSH_OPTS,
# OXICORD_REMOTE_DIR (default /srv/downloads/oxicord).
set -eu
cd "$(dirname "$0")/.."
: "${OXICORD_SERVER:?set OXICORD_SERVER to user@host}"
DIR="${OXICORD_REMOTE_DIR:-/srv/downloads/oxicord}"
KEY="${OXICORD_SSH_KEY:+-i $OXICORD_SSH_KEY}"
OPTS="${OXICORD_SSH_OPTS:-}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

scripts/bundle-macos.sh --dist
codesign --verify --strict target/dist/Oxicord.app

# what the download page shows: version, date and notes
notes=$(printf '%s\n' "$@")
python3 -c 'import json, sys, time
print(json.dumps({"version": sys.argv[1], "date": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "notes": sys.argv[2]}), end="")' \
    "$VERSION" "$notes" > target/dist/release.json

# shellcheck disable=SC2086
scp -q $KEY $OPTS target/dist/Oxicord-Mac.zip target/dist/release.json "$OXICORD_SERVER:"
# shellcheck disable=SC2086
ssh $KEY $OPTS -o BatchMode=yes "$OXICORD_SERVER" "set -e
sudo install -d -m 0755 '$DIR'
sudo install -m 0644 Oxicord-Mac.zip '$DIR/'
sudo install -m 0644 release.json '$DIR/'   # last, so the page never names a version before its file is there
rm -f Oxicord-Mac.zip release.json"
echo "Published Oxicord $VERSION"

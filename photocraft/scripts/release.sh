#!/usr/bin/env bash
# Prepare a GitHub release for the port's update channel.
# Creates SHA256SUMS.txt next to the APK and prints the manual publishing
# steps. Does NOT publish anything (the GitHub token is dead; publishing is
# a documented later step for Anthony).
set -euo pipefail

PORT_DIR="$HOME/workspace/photocraft-android"
YOUR_FILES="$HOME/workspace/your_files"

VERSION_NAME="$(grep -oP 'pub const VERSION_NAME: &str = "\K[^"]+' "$PORT_DIR/upstream/apps/photocraft-android/src/lib.rs")"
APK="PhotoCraft-${VERSION_NAME}-universal.apk"
[ -f "$YOUR_FILES/$APK" ] || { echo "build first: scripts/build-apk.sh" >&2; exit 1; }

cd "$YOUR_FILES"
sha256sum "$APK" > SHA256SUMS.txt
cat SHA256SUMS.txt

TAG="photocraft-v${VERSION_NAME}"
cat <<EOF

Release checklist (manual — GitHub token is dead):
  1. Re-auth gh (device flow), then create the repo: areilly111/CraftApps-Android
  2. Tag: $TAG   (release title: "PhotoCraft Android ${VERSION_NAME}")
  3. Upload assets: $APK and SHA256SUMS.txt
  4. Paste the upstream PhotoCraft release notes into the body.
  5. The in-app updater polls https://api.github.com/repos/areilly111/CraftApps-Android/releases?per_page=100,
     filters tags starting with "photocraft-", and offers the *-universal.apk asset automatically.
EOF

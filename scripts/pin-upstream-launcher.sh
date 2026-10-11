#!/usr/bin/env bash
# Check the upstream craft-launcher for new releases and bump the Android port.
# Usage: scripts/pin-upstream-launcher.sh [tag]   (e.g. scripts/pin-upstream-launcher.sh v0.3.0)
#
# NOTE: unlike the six app ports, the launcher port is a clean reimplementation
# ("modeled on artcraft-launcher-core's catalog") — its upstream/ contains NO
# upstream source, only the port crate, so there is no checkout to re-pin and
# no patches to re-apply. This script:
#   1. fetches upstream tags and reports what's new since the port's base
#      (v0.2.0), so the maintainer can decide whether the port's catalog/UI
#      logic needs updating to match
#   2. optionally bumps the port's VERSION_NAME when given a tag
#
# If upstream changed its release JSON shape, catalog fields, or install flow,
# update launcher/upstream/apps/craftlauncher-android/src/{catalog,releases,ui}.rs
# by hand — there is no mechanical patch for a reimplementation.
#
# Everything is LOCAL. This script never pushes.
set -euo pipefail

UPSTREAM_REF="$HOME/workspace/craftapps-android/upstreams/craft-launcher"
PORT_LIB="$HOME/workspace/craftapps-android/launcher/upstream/apps/craftlauncher-android/src/lib.rs"
BASE_TAG="v0.2.0"

[ -f "$PORT_LIB" ] || {
    echo "FATAL: launcher port crate not found at $PORT_LIB" >&2
    exit 1
}

cd "$UPSTREAM_REF"
git fetch --tags origin 2>/dev/null || echo "(offline: using cached tags)"

echo "Port base: $BASE_TAG"
echo "Upstream tags:"
git tag --sort=-v:refname | head -10

if [ $# -eq 1 ]; then
    TAG="$1"
    echo ""
    echo "=== changes in upstream since $BASE_TAG (for manual port review) ==="
    git log --oneline "$BASE_TAG..$TAG" 2>/dev/null | head -20 || echo "(no log available)"
    echo ""
    echo "Review the log above. If the port's catalog/releases/ui logic needs"
    echo "changes to match, edit launcher/upstream/apps/craftlauncher-android/src/ by hand."
    echo ""
    echo "Unified versioning: VERSION_NAME is managed at release time"
    echo "(see scripts/release.sh), NOT bumped here."
else
    echo ""
    echo "Run with a tag to review changes and optionally bump the port version."
fi

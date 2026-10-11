#!/usr/bin/env bash
# Unified release for CraftApps Android.
#
# All 8 apps share one version (e.g. 0.1.0). This script:
#   1. Verifies every port crate's VERSION_NAME matches the release version.
#   2. Collects the 8 APKs from ~/workspace/your_files/ and writes SHA256SUMS.txt.
#   3. Creates the git tag v<version> and (optionally) the GitHub release.
#
# Usage: scripts/release.sh <version> [--publish]
#   e.g. scripts/release.sh 0.1.0 --publish
#
# Without --publish it only verifies + stages assets locally.
# With --publish it creates the tag and GitHub release (needs `gh` auth).
#
# Release flow (per Anthony's process):
#   1. Build + hardware-test every app: scripts/build-apk.sh <app> for all 8.
#   2. Confirm working on device.
#   3. Run this script to cut the unified release.
set -euo pipefail

VERSION="${1:?usage: scripts/release.sh <version> [--publish]}"
PUBLISH="${2:-}"
MONOREPO="$HOME/workspace/craftapps-android"
YOUR_FILES="$HOME/workspace/your_files"
TAG="v$VERSION"

# Per-app metadata: <dir>|<AppName>|<crate>
APPS=(
    "photocraft|PhotoCraft|photocraft-android"
    "designcraft|DesignCraft|designcraft-android"
    "vectorcraft|VectorCraft|vectorcraft-android"
    "lightcraft|LightCraft|lightcraft-android"
    "pdfcraft|PdfCraft|pdfcraft-android"
    "effectcraft|EffectCraft|effectcraft-android"
    "filmcraft|FilmCraft|filmcraft-android"
    "launcher|CraftLauncher|craftlauncher-android"
)

echo "== verifying VERSION_NAME across all 8 ports =="
for entry in "${APPS[@]}"; do
    IFS='|' read -r dir _ crate <<< "$entry"
    lib="$MONOREPO/$dir/upstream/apps/$crate/src/lib.rs"
    vn="$(grep -oP 'pub const VERSION_NAME: &str = "\K[^"]+' "$lib")"
    if [ "$vn" != "$VERSION" ]; then
        echo "MISMATCH: $dir has VERSION_NAME=$vn, want $VERSION" >&2
        echo "Fix with: sed -i 's/VERSION_NAME: &str = \"$vn\"/VERSION_NAME: &str = \"$VERSION\"/' $lib" >&2
        exit 1
    fi
    echo "  $dir: $vn OK"
done

echo "== collecting APKs =="
cd "$YOUR_FILES"
APKS=()
for entry in "${APPS[@]}"; do
    IFS='|' read -r _ name _ <<< "$entry"
    apk="${name}-${VERSION}-universal.apk"
    if [ ! -f "$apk" ]; then
        echo "MISSING: $YOUR_FILES/$apk — build it first: scripts/build-apk.sh <app>" >&2
        exit 1
    fi
    APKS+=("$apk")
    echo "  $apk ($(du -h "$apk" | cut -f1))"
done

sha256sum "${APKS[@]}" > SHA256SUMS.txt
echo "wrote SHA256SUMS.txt"

if [ "$PUBLISH" != "--publish" ]; then
    echo ""
    echo "Dry run complete. To publish:"
    echo "  scripts/release.sh $VERSION --publish"
    exit 0
fi

echo "== creating git tag $TAG =="
cd "$MONOREPO"
if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "tag $TAG already exists" >&2
    exit 1
fi
git tag -a "$TAG" -m "CraftApps Android $VERSION (unified release)"
git push origin "$TAG"

echo "== creating GitHub release =="
gh release create "$TAG" \
    --repo areilly111/CraftApps-Android \
    --title "CraftApps Android $VERSION" \
    --notes "Unified release: all 8 apps at $VERSION.

Built from the CraftApps-Android monorepo. See README.md for install instructions." \
    "${APKS[@]/#/$YOUR_FILES/}" "$YOUR_FILES/SHA256SUMS.txt"

echo ""
echo "Released $TAG with ${#APKS[@]} APKs."
echo "The in-app updaters will pick this up automatically (unified tag scheme)."

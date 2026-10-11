#!/usr/bin/env bash
# Generic build script for CraftApps Android ports.
# Usage: build-apk.sh <app>  (e.g. build-apk.sh designcraft)
#
# Pipeline:
#   1. build the Rust cdylib for arm64-v8a + x86_64 (direct cargo, NDK clang)
#   2. copy the .so files into the Gradle project's jniLibs
#   3. assemble the signed release APK with Gradle (suite keystore)
#   4. copy it to ~/workspace/your_files/<Name>-<version>-universal.apk
set -euo pipefail

APP="${1:?usage: build-apk.sh <app> (designcraft|vectorcraft|lightcraft|pdfcraft|filmcraft|effectcraft|launcher)}"
MONOREPO="$HOME/workspace/craftapps-android"
APP_DIR="$MONOREPO/$APP"
UPSTREAM="$APP_DIR/upstream"
ANDROID="$APP_DIR/android"
YOUR_FILES="$HOME/workspace/your_files"

# Per-app metadata
case "$APP" in
    designcraft)  APP_NAME="DesignCraft";  CRATE="designcraft-android";  LIB="libdesigncraft_android.so" ;;
    vectorcraft)  APP_NAME="VectorCraft";  CRATE="vectorcraft-android";  LIB="libvectorcraft_android.so" ;;
    lightcraft)   APP_NAME="LightCraft";    CRATE="lightcraft-android";   LIB="liblightcraft_android.so" ;;
    pdfcraft)     APP_NAME="PdfCraft";      CRATE="pdfcraft-android";     LIB="libpdfcraft_android.so" ;;
    filmcraft)    APP_NAME="FilmCraft";     CRATE="filmcraft-android";    LIB="libfilmcraft_android.so" ;;
    effectcraft)  APP_NAME="EffectCraft";   CRATE="effectcraft-android";  LIB="libeffectcraft_android.so" ;;
    launcher)     APP_NAME="CraftLauncher"; CRATE="craftlauncher-android"; LIB="libcraftlauncher_android.so" ;;
    *) echo "unknown app: $APP" >&2; exit 1 ;;
esac

export PATH="$HOME/.cargo/bin:$PATH"
export TMPDIR=/var/tmp
export ANDROID_HOME="$HOME/workspace/android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export JAVA_HOME="$HOME/workspace/android-toolchain/jdk-17"
export PATH="$JAVA_HOME/bin:$PATH"
NDK="$ANDROID_HOME/ndk/27.0.12077973"
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
# Shared build cache (all apps share eframe 0.36 deps)
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/opt/pc-android-target}"
mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR"
# NDK C compiler for crates with C deps (aws-lc-sys etc.)
export CC_aarch64_linux_android="$TOOLCHAIN/aarch64-linux-android29-clang"
export AR_aarch64_linux_android="$TOOLCHAIN/llvm-ar"

# --- version: single source of truth is VERSION_NAME in the Rust crate ---
# Unified versioning: all apps share one version (e.g. 0.1.0).
# versionCode = major*10000 + minor*100 + patch  (0.1.0 -> 100, monotonic).
VERSION_NAME="$(grep -oP 'pub const VERSION_NAME: &str = "\K[^"]+' "$UPSTREAM/apps/$CRATE/src/lib.rs")"
VERSION_CODE="$(python3 -c "
v='$VERSION_NAME'
major, minor, patch = (int(x) for x in v.split('.'))
print(major * 10000 + minor * 100 + patch)")"
APK_NAME="${APP_NAME}-${VERSION_NAME}-universal.apk"
echo "== $APP_NAME Android $VERSION_NAME (versionCode $VERSION_CODE) =="

# --- 1. Rust cdylib for both ABIs ---
for TARGET in aarch64-linux-android x86_64-linux-android; do
    case "$TARGET" in
        aarch64-linux-android) CLANG_TARGET="aarch64-linux-android29" ;;
        x86_64-linux-android)  CLANG_TARGET="x86_64-linux-android29" ;;
    esac
    LINKER="$TOOLCHAIN/${CLANG_TARGET}-clang"
    [ -x "$LINKER" ] || { echo "missing NDK linker: $LINKER" >&2; exit 1; }
    UPPER="$(echo "$TARGET" | tr '[:lower:]' '[:upper:]' | tr '-' '_')"
    LOWER="$(echo "$TARGET" | tr '-' '_')"
    echo "== cargo build --release -p $CRATE --target $TARGET =="
    cd "$UPSTREAM"
    # shellcheck disable=SC2086
    # NB: cc-rs wants the lowercase CC_<target> (case-sensitive); cargo wants
    # the uppercase CARGO_TARGET_<TARGET>_* variants. Set both.
    env "CARGO_TARGET_${UPPER}_LINKER=$LINKER" \
        "CC_${UPPER}=$TOOLCHAIN/${CLANG_TARGET}-clang" \
        "CC_${LOWER}=$TOOLCHAIN/${CLANG_TARGET}-clang" \
        "AR_${UPPER}=$TOOLCHAIN/llvm-ar" \
        "AR_${LOWER}=$TOOLCHAIN/llvm-ar" \
        cargo build --release -p "$CRATE" --target "$TARGET"
    cd - >/dev/null
done

# --- 2. stage the .so files for Gradle ---
for ABI in arm64-v8a x86_64; do
    case "$ABI" in
        arm64-v8a) TARGET="aarch64-linux-android" ;;
        x86_64)    TARGET="x86_64-linux-android" ;;
    esac
    DEST="$ANDROID/app/src/main/jniLibs/$ABI"
    mkdir -p "$DEST"
    cp "$CARGO_TARGET_DIR/$TARGET/release/$LIB" "$DEST/"
    echo "staged $ABI: $(du -h "$DEST/$LIB" | cut -f1)"
done

# --- 3. Gradle assembleRelease (suite keystore, no proxy) ---
# The Hatch CA for the JDK (proxy TLS interception) — harmless if already present.
if ! "$JAVA_HOME/bin/keytool" -list -keystore "$JAVA_HOME/lib/security/cacerts" \
    -storepass changeit -alias hatch-egress-ca >/dev/null 2>&1; then
  "$JAVA_HOME/bin/keytool" -importcert -noprompt -trustcacerts \
    -alias hatch-egress-ca -file /usr/local/share/ca-certificates/hatch-egress-ca.crt \
    -keystore "$JAVA_HOME/lib/security/cacerts" -storepass changeit
fi
# NOTE: sandbox reaches Maven Central / Google Maven directly; do NOT proxy Gradle.
export GRADLE_OPTS="-Djava.net.preferIPv4Stack=true"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$HOME/.gradle}"
echo "== gradle assembleRelease =="
"$HOME/workspace/android-toolchain/gradle-8.7/bin/gradle" --no-daemon \
    -p "$ANDROID" assembleRelease \
    -PversionCode="$VERSION_CODE" -PversionName="$VERSION_NAME"

# --- 4. deliver ---
BUILT="$ANDROID/app/build/outputs/apk/release/app-release.apk"
[ -f "$BUILT" ] || { echo "APK not produced at $BUILT" >&2; exit 1; }
mkdir -p "$YOUR_FILES"
cp "$BUILT" "$YOUR_FILES/$APK_NAME"
echo "== delivered: $YOUR_FILES/$APK_NAME ($(du -h "$YOUR_FILES/$APK_NAME" | cut -f1)) =="
"$ANDROID_HOME/build-tools/34.0.0/aapt" dump badging "$YOUR_FILES/$APK_NAME" | head -8

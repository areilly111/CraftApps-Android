#!/usr/bin/env bash
# Full pipeline for the PhotoCraft Android port:
#   1. build the Rust cdylib for arm64-v8a + x86_64 (direct cargo, NDK clang)
#   2. copy the .so files into the Gradle project's jniLibs
#   3. generate the persistent keystore on first run (never the debug key)
#   4. assemble the signed release APK with Gradle
#   5. copy it to ~/workspace/your_files/PhotoCraft-<version>-universal.apk
#
# Environment quirks of this sandbox are handled here (cargo on PATH, Android
# SDK/NDK locations, IPv4-only Gradle, target dir on the roomy filesystem).
set -euo pipefail

PORT_DIR="$HOME/workspace/photocraft-android"
UPSTREAM="$PORT_DIR/upstream"
ANDROID="$PORT_DIR/android"
YOUR_FILES="$HOME/workspace/your_files"

export PATH="$HOME/.cargo/bin:$PATH"
export TMPDIR=/var/tmp
export ANDROID_HOME="$HOME/workspace/android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export JAVA_HOME="$HOME/workspace/android-toolchain/jdk-17"
export PATH="$JAVA_HOME/bin:$PATH"
NDK="$ANDROID_HOME/ndk/27.0.12077973"
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
# Build cache lives on the roomy overlay filesystem (/home/hatch is 99% full).
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/opt/pc-android-target}"
mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR"

# --- version: single source of truth is VERSION_NAME in the Rust crate ---
VERSION_NAME="$(grep -oP 'pub const VERSION_NAME: &str = "\K[^"]+' "$UPSTREAM/apps/photocraft-android/src/lib.rs")"
# 0.6.0-port1 -> 60001  (upstream_numeric * 100 + port_build)
VERSION_CODE="$(python3 -c "
v='$VERSION_NAME'
up, port = v.split('-port')
major, minor, patch = (int(x) for x in up.split('.'))
print((major * 10000 + minor * 100 + patch) * 100 + int(port))")"
APK_NAME="PhotoCraft-${VERSION_NAME}-universal.apk"
echo "== PhotoCraft Android $VERSION_NAME (versionCode $VERSION_CODE) =="

# --- 1. Rust cdylib for both ABIs (direct cargo; no cargo-ndk needed) ---
for TARGET in aarch64-linux-android x86_64-linux-android; do
    case "$TARGET" in
        aarch64-linux-android) CLANG_TARGET="aarch64-linux-android29" ;;
        x86_64-linux-android)  CLANG_TARGET="x86_64-linux-android29" ;;
    esac
    LINKER="$TOOLCHAIN/${CLANG_TARGET}-clang"
    [ -x "$LINKER" ] || { echo "missing NDK linker: $LINKER" >&2; exit 1; }
    UPPER="$(echo "$TARGET" | tr '[:lower:]' '[:upper:]' | tr '-' '_')"
    echo "== cargo build --release -p photocraft-android --target $TARGET =="
    echo "   linker: CARGO_TARGET_${UPPER}_LINKER=$LINKER"
    cd "$UPSTREAM"
    # shellcheck disable=SC2086
    env "CARGO_TARGET_${UPPER}_LINKER=$LINKER" \
        cargo build --release -p photocraft-android --target "$TARGET"
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
    cp "$CARGO_TARGET_DIR/$TARGET/release/libphotocraft_android.so" "$DEST/"
    echo "staged $ABI: $(du -h "$DEST/libphotocraft_android.so" | cut -f1)"
done

# --- 3. persistent keystore (generated once; losing it breaks future updates) ---
KEYSTORE_DIR="$PORT_DIR/keystore"
KEYSTORE="$KEYSTORE_DIR/photocraft-android.keystore"
PROPS="$KEYSTORE_DIR/keystore.properties"
if [ ! -f "$KEYSTORE" ]; then
    echo "== generating persistent keystore =="
    mkdir -p "$KEYSTORE_DIR"
    STOREPASS="$(python3 -c 'import secrets; print(secrets.token_urlsafe(24))')"
    keytool -genkeypair -v \
        -keystore "$KEYSTORE" -storetype PKCS12 \
        -alias photocraft -keyalg RSA -keysize 4096 -validity 9125 \
        -storepass "$STOREPASS" -keypass "$STOREPASS" \
        -dname "CN=PhotoCraft Android Port, OU=Personal, O=Personal, L=Winnipeg, C=CA" \
        >/dev/null
    chmod 600 "$KEYSTORE"
    cat > "$PROPS" <<EOF
storeFile=$KEYSTORE
storePassword=$STOREPASS
keyAlias=photocraft
keyPassword=$STOREPASS
EOF
    chmod 600 "$PROPS"
    echo "keystore generated at $KEYSTORE (mode 600). BACK IT UP — losing it means updates can never be installed again."
else
    echo "== using existing keystore $KEYSTORE =="
fi

# --- 4. Gradle assembleRelease (IPv4-only: the sandbox blackholes IPv6 loopback) ---
# The egress proxy TLS-intercepts; the JDK must trust the Hatch CA or every
# dependency download dies with "connection reset / SSL misconfiguration".
if ! "$JAVA_HOME/bin/keytool" -list -keystore "$JAVA_HOME/lib/security/cacerts" \
    -storepass changeit -alias hatch-egress-ca >/dev/null 2>&1; then
  "$JAVA_HOME/bin/keytool" -importcert -noprompt -trustcacerts \
    -alias hatch-egress-ca -file /usr/local/share/ca-certificates/hatch-egress-ca.crt \
    -keystore "$JAVA_HOME/lib/security/cacerts" -storepass changeit
fi
# NOTE (2026-10-10): the sandbox reaches Maven Central / Google Maven directly;
# routing Gradle through the egress proxy breaks dependency downloads (the
# proxy password rotates and the proxy resets unauthenticated connections).
# Keep ~/.gradle/gradle.properties free of proxy systemProps.
export GRADLE_OPTS="-Djava.net.preferIPv4Stack=true"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$HOME/.gradle}"
echo "== gradle assembleRelease =="
"$HOME/workspace/android-toolchain/gradle-8.7/bin/gradle" --no-daemon \
    -p "$ANDROID" assembleRelease \
    -PversionCode="$VERSION_CODE" -PversionName="$VERSION_NAME"

# --- 5. deliver ---
BUILT="$ANDROID/app/build/outputs/apk/release/app-release.apk"
[ -f "$BUILT" ] || { echo "APK not produced at $BUILT" >&2; exit 1; }
mkdir -p "$YOUR_FILES"
cp "$BUILT" "$YOUR_FILES/$APK_NAME"
echo "== delivered: $YOUR_FILES/$APK_NAME ($(du -h "$YOUR_FILES/$APK_NAME" | cut -f1)) =="
"$ANDROID_HOME/build-tools/34.0.0/aapt" dump badging "$YOUR_FILES/$APK_NAME" | head -8

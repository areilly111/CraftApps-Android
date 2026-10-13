# CraftApps-Android Patches

This document records every patch, fix, and modification applied to the upstream
ArtCraft app sources to produce the Android builds. When updating to a new upstream
version, re-apply each patch listed here.

**Workflow (user directive 2026-10-12):** For any new build, always pull the latest
upstream release from `github.com/storytold/<app>` first, then re-apply the patches
below before building.

---

## Global patches (all apps)

### 1. Unified versioning (v0.1.0)
- **File:** `scripts/build-apk.sh`
- **Change:** `VERSION_CODE = 100000 + major*10000 + minor*100 + patch`
  (0.1.0 → 100100). The 100000 base ensures the code is above all legacy
  per-app versionCodes (max was 80001 for VectorCraft).
- **Why:** Old apps had higher versionCodes (e.g. FilmCraft 50001); without the
  base offset Android treats the new APK as a downgrade and forces uninstall.
- **Applies to:** all 8 apps.

### 2. Bluetooth keyboard modifier fix
- **Files:** `<app>/android/app/src/main/java/app/<pkg>/android/*Activity.java`
- **Change:** Added `dispatchKeyEvent` override that:
  1. Tracks Ctrl/Shift/Alt presses in a `modifierState` bitmask.
  2. For non-modifier keys, synthesizes a corrected `KeyEvent` with proper
     `META_CTRL_ON` / `META_SHIFT_ON` / `META_ALT_ON` flags when our tracked
     state has modifiers missing from the event's meta state.
- **Why:** winit's Android backend drops modifier meta state from Bluetooth
  keyboards; Ctrl+C and other combos did nothing.
- **Applies to:** all 8 apps (PhotoCraft, DesignCraft, VectorCraft, LightCraft,
  PdfCraft, FilmCraft, EffectCraft, Launcher).
- **Date:** 2026-10-12.

### 3. Memory cache caps (mobile)
- **Files:** `<app>/upstream/apps/<app>-android/src/lib.rs`
- **Change:** `set_cache_budget(256 MB)` and `set_plan_budget(256 MB)`.
  (Raised from 128 MB on 2026-10-11 per user request.)
- **Why:** Desktop defaults caused OOM kills on Android.
- **Applies to:** FilmCraft, EffectCraft (the video apps). Check others if OOMs appear.

---

## PhotoCraft

### Patches to `photocraft/upstream/`
_(Android app lives at `photocraft/upstream/apps/photocraft-android/` — ours, not upstream's)_

### 4. Keystore signing setup
- **File:** `photocraft/keystore/keystore.properties` (git-ignored)
- **Change:** Points at persistent keystore
  `~/workspace/photocraft-android/keystore/photocraft-android.keystore`.
- **Why:** Gradle's debug keystore lives in uid-0 `/root/.android/` (ephemeral);
  a fresh key per VM breaks update installs ("signatures do not match").
- **Cert SHA-256:** `eafc23ac9cc1c2cc948326408ce764bdf8d7e7fb9be24ea536347501c6cc7efc`
- **Note:** `build-apk.sh` must ensure this file exists before building; secrets
  are never committed.

### 5. TOOL_TYPE_PALM removal
- **File:** `photocraft/upstream/apps/photocraft-android/src/...` (see commit 98f8726)
  Actually: `photocraft/android/app/src/main/java/app/photocraft/android/PhotoCraftActivity.java`
- **Change:** Removed reference to nonexistent `MotionEvent.TOOL_TYPE_PALM`.
- **Why:** Build failure — constant doesn't exist in the Android SDK.
- **Date:** 2026-10-12 (commit 98f8726).

### 6. Pen side-channel (stylus support)
- **File:** `PhotoCraftActivity.java` — `dispatchTouchEvent` override
- **Change:** Forwards pen tool-type / barrel-button state changes to Rust via
  `nativeOnPenEvent` JNI. winit drops tool type (stylus/finger/eraser).
- **Rust side:** `photocraft/upstream/apps/photocraft-android/src/pen.rs`
- **Why:** PhotoCraft is the stylus reference implementation.
- **Status:** Pressure smoothing APIs exist but not yet wired to stroke pipeline;
  right-click is a logging placeholder.

### 7. Keyboard JNI bridge (Rust side)
- **File:** `photocraft/upstream/apps/photocraft-android/src/keyboard.rs`
- **Change:** `nativeOnModifierState` JNI entry storing modifier bitmask in an
  `AtomicI32`. Currently unused (Java synthesizes corrected events instead);
  kept for debugging.
- **Date:** 2026-10-12.

---

## FilmCraft

### Patches to `filmcraft/upstream/`

### 8. Mobile cache caps 256 MB
- **File:** `filmcraft/upstream/apps/filmcraft-android/src/lib.rs`
- **Change:** `inner.frames.set_cache_budget(256 MB)` and `set_plan_budget(256 MB)`.
- **Why:** Prevent OOM on video decode. Raised 128→256 MB on 2026-10-11.

### 9. MediaCodec scaffolding (NOT yet functional)
- **Files:**
  - `filmcraft/upstream/crates/platform/src/mediacodec.rs` (new)
  - `filmcraft/upstream/crates/platform/src/lib.rs` (`mod mediacodec`,
    `mediacodec_factory`, Android `register()` arm)
  - `filmcraft/upstream/crates/platform/Cargo.toml` (`ndk = "0.9"` for Android)
- **Change:** `MediaCodecDecoder` struct implementing `VideoDecoder`; factory
  registered for `video/avc`, `video/hevc`, `video/x-vnd.on2.vp9`, `video/av01`.
  `decode()` currently returns `CodecError::Unsupported` so `HybridDecoder`
  falls back to software.
- **Why:** Software-only decoding is too slow for full-res playback (user
  confirmed 1/8 playback resolution works, full res freezes). Hardware decoding
  via NDK `AMediaCodec` is the proper fix.
- **Status (2026-10-12):** Scaffolding compiles for `aarch64-linux-android`
  (`cargo check` green). Full NDK implementation (create/configure codec,
  feed input, dequeue output, YUV→RGB) still TODO.
- **Note:** EffectCraft uses `filmcraft-codecs` too and will benefit automatically
  once the decoder is real.

---

## LightCraft

### 10. Catalog lock disabled on Android
- **File:** `lightcraft/upstream/crates/catalog/src/lock.rs`
- **Change:** Disabled the file lock (was warning on shared storage).
- **Why:** Android shared storage doesn't support the lock; spurious warning.
- **Status:** Needs hardware confirmation the warning is gone.
- **Date:** 2026-10-12.

---

## EffectCraft

### 11. Android audio support
- **Change:** Added Android audio module (old build had none).
- **Why:** User reported old build silent and slow/glitchy.
- **Status:** Needs hardware confirmation.

### 12. Mobile cache caps 256 MB
- Same as FilmCraft #8; applied to EffectCraft's media pool.

---

## Launcher
- No app-specific patches beyond the global ones (#1, #2).

## DesignCraft / VectorCraft / PdfCraft
- No app-specific patches beyond the global ones (#1, #2).

---

## Build environment notes (see ~/AGENTS.md)

- Gradle needs `GRADLE_OPTS="-Djava.net.preferIPv4Stack=true"`,
  `GRADLE_USER_HOME=$HOME/.gradle`, `ANDROID_HOME=$HOME/workspace/android-sdk`,
  `--no-daemon`, `kotlin.compiler.execution.strategy=in-process`.
- Never proxy Gradle (breaks Maven Central downloads).
- `build-apk.sh` pins all of the above; PhotoCraft documents the proxy note.
- APK native libs need manual 4096 page-alignment (see `~/workspace/gamehacker/merge_apk.py`);
  don't run `zipalign` after.
- Bump `versionCode` on every shipped build (now automatic via #1).

---

## Version history

- **v0.1.0 (2026-10-12):** First unified release. All 8 apps, versionCode 100100.
  Known issues: no MediaCodec (software video decode only), Bluetooth keyboard
  combos broken (fix coded after release).

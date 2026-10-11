# DesignCraft for Android (tablet)

A native Android port of [storytold's DesignCraft](https://github.com/storytold/designcraft)
(v0.5.0, pure-Rust page-layout app targeting InDesign parity) for tablets, with
an in-app self-updater. Built on the shared `craftapps-shared` shell crate
(updater, GPU cascade, `/sdcard/CraftApps` storage, SAF, JNI) — the android crate
is a thin shell over it plus DesignCraft's app-specific services, file browser
and eframe shell.

**Status: 0.5.0-port1 built, hardware-UNVERIFIED.** No Android hardware was
available at build time — `cargo check`/`clippy` are clean for
`aarch64-linux-android`, but the port has never run on a real tablet.

## What was built

- **`upstream/apps/designcraft-android/`** — a Rust `cdylib`
  crate, member of the upstream workspace. `android_main` (android-activity 0.6
  `native-activity` contract) builds the Android platform services and runs the
  *same* `DesignApp` as the desktop through `eframe::run_native` with
  `NativeOptions::android_app`.
- **Upstream mods** (in `upstream/`, mirroring the PhotoCraft
  port's Android diff, adapted — DesignCraft's ui-egui has no eframe dep, no
  stylus engine and no `prefs_ui.rs`):
  - `crates/ui-egui/src/lib.rs`: Android-only `AndroidHooks` /
    `AndroidStorageHooks` + `Services::android_hooks`.
  - `crates/ui-egui/src/menus.rs`: Help → "Check for Updates…" (Android-only
    menu item + `UI_COMMANDS` entry + dispatch; enabled without an open doc).
  - `crates/ui-egui/src/dialogs.rs`: Preferences › Storage section
    (Android-only) driving the shell hooks. No Stylus section: DesignCraft has
    no pen/stylus support upstream (a Stylus section would be decoration).
  - `xtask/src/layers.rs`: `("android", Class::Exempt)` registration.
- **Services** (`apps/designcraft-android/src/services.rs`): `pick_open` is left
  unset so opens take the web path — `open_async` opens the in-app egui browser
  and the chosen file arrives through `Services::inbox` (works for Open and
  Place alike). `pick_save` reserves the suggested name in the current media
  location (DesignCraft's save API is synchronous, so there is no save dialog
  on Android); `read`/`write` are storage-aware (SAF content URIs + plain
  paths). Engine document saves (`.designcraft`) need a real file path, so
  Save As to the SD card is declined with an explanation — exports reach
  `services.write`, which handles content URIs.
- **`android/`** — Gradle project (`app.designcraft.android`, minSdk 29,
  arm64-v8a + x86_64, versionCode 50001, versionName 0.5.0-port1). The Rust
  `.so` files are prebuilt and staged into `jniLibs`; one tiny Java
  `ContentProvider` (`ApkProvider`) serves downloaded APKs to the system
  installer. Signed with the **suite keystore**
  (`craftapps-android/keystore/craftapps-suite.keystore`) — new apps share one
  suite key (per the suite decision), unlike PhotoCraft which keeps its own.
- **Updater** — Help → "Check for Updates…" (plus auto-check): polls the shared
  `areilly111/CraftApps-Android` release feed, filters tags starting with
  `designcraft-`, downloads the `-universal.apk` with progress, verifies
  SHA-256 against the release's `SHA256SUMS.txt`, then fires the system install
  dialog.
- **Storage** — `/sdcard/CraftApps/Media/` is the default open/save location
  (in-app browser rooted there), `/sdcard/CraftApps/Settings/DesignCraft/`
  holds `ui.json`/`prefs.json`/recovery/updater state. Needs "All files access"
  (sideload-only); first launch walks the user to the toggle, or they can
  choose internal storage. Preferences › Storage offers the SD card (persisted
  SAF tree) as the media location; "Browse other locations…" in the Open
  browser goes through the Storage Access Framework.
- **GPU cascade** — Vulkan → OpenGL ES → CPU, via the shared `gpu` module.
  DesignCraft renders its canvas on the CPU (vello); the GPU only composites
  the eframe interface, so the canvas-attach step just records the adapter for
  the log and installs a device error watcher. (DesignCraft has no performance
  GPU prefs upstream, so the default cascade always applies.)

## Versioning

`VERSION_NAME` in `upstream/apps/designcraft-android/src/lib.rs`
is the single source of truth, e.g. `0.5.0-port1`. `scripts/build-apk.sh`
derives `versionCode = upstream_numeric * 100 + port_build` → `50001`. Bump the
port number (`-port2`, …) for every rebuild — Android treats a non-increasing
versionCode as a downgrade and refuses the update.

## Rebuilding

```sh
scripts/build-apk.sh   # (to be added; mirrors photocraft's)
```

Pipeline: cargo builds the cdylib for both ABIs (NDK clang directly) → `.so`
files staged into `android/app/src/main/jniLibs` → Gradle `assembleRelease`
signed with the suite keystore → signed APK.

## Known port1 limits

- Hardware-unverified (see top).
- No save dialog: saves/exports go straight to the media folder with the
  suggested name (DesignCraft's `pick_save` is synchronous).
- File ▸ Place With Options does nothing on Android (it calls `pick_open`
  directly, which the port leaves unset); the regular Place… dialog works.
- The shared updater's install intent takes the per-app provider authority
  (`install_apk(path, authority)`), so the manifest declares only
  `app.designcraft.android.apkprovider`.
- No Stylus section (no pen support upstream); no tilt/twist/eraser-end.
- Crash recovery, prefs: same as desktop, rooted at the port's config dir.

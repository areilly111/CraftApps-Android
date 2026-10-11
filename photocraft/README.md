# PhotoCraft for Android (tablet)

A native Android port of [storytold's PhotoCraft](https://github.com/storytold/photocraft)
(v0.6.0, pure-Rust Photoshop clone) for tablets, with an in-app self-updater.

**Status: v1.1 built, hardware-UNVERIFIED.** No Android hardware was available at
build time — the APK is signed and structurally valid (`aapt dump badging`
passes, both ABIs link), but v1.1 has never run on a real tablet. v1.0 was
verified on hardware ("sick works great"); v1.1 needs the same smoke test.

## What was built

- **`upstream/apps/photocraft-android/`** — a Rust `cdylib` crate, member of
  the upstream workspace. `android_main` (android-activity 0.6
  `native-activity` contract) builds the Android platform services and runs
  the *same* `PhotocraftApp` as the desktop through `eframe::run_native`
  with `NativeOptions::android_app`. No upstream crate was modified, except
  one registration line in `xtask/src/layers.rs`.
- **Pen support is free.** winit's Android backend forwards `MotionEvent`
  pressure as `egui::Event::Touch { force }`, which
  `photocraft_ui_egui::Stylus` already reads with no cfg gates. Tilt, twist
  and the eraser end are dropped by winit 0.30 — v1.1 work.
- **`android/`** — Gradle project (`app.photocraft.android`, minSdk 29,
  arm64-v8a + x86_64). The Rust `.so` files are prebuilt and staged into
  `jniLibs`; one tiny Java `ContentProvider` (`ApkProvider`) serves
  downloaded APKs to the system installer.
- **Updater** (v1.1) — Help → "Check for Updates…" (plus auto-check toggle in
  Preferences): checks the shared `areilly111/CraftApps-Android` release
  feed, picks the newest release tagged `photocraft-…`, downloads the
  `-universal.apk` with progress, verifies SHA-256 against the release's
  `SHA256SUMS.txt`, then fires the system install dialog. The repo doesn't
  exist yet, so until it's published the updater says so honestly.
- **Storage** (v1.1) — `/sdcard/CraftApps/Media/` is the default open/save
  location (in-app browser rooted there), `/sdcard/CraftApps/Settings/PhotoCraft/`
  holds prefs/recovery/presets, `/sdcard/CraftApps/Plugins/` the default
  plug-in folder. Needs "All files access" (sideload-only, no Play policy
  issue); first launch walks the user to the toggle. SAF open/save covers
  locations outside `/CraftApps`; Settings → Storage offers the SD card
  (persisted SAF tree) as the media location.
- **Stylus** (v1.1) — dedicated Preferences section: pressure toggle/curve
  plus a live pressure test area. Pressure is live on Android; tilt/twist/
  eraser-end are dropped by winit 0.30 (future work).
- **GPU cascade** (v1.1) — Vulkan → OpenGL ES → CPU canvas, Vulkan default,
  with a Preferences override (Auto/Vulkan/GLES/CPU).
- Crash recovery, prefs, brush presets: same as desktop, rooted at the
  app's internal `PhotoCraft/` dir via `PHOTOCRAFT_CONFIG_DIR`.

## Versioning

`VERSION_NAME` in `upstream/apps/photocraft-android/src/lib.rs` is the single
source of truth, e.g. `0.6.0-port1`. `scripts/build-apk.sh` derives
`versionCode = upstream_numeric * 100 + port_build` → `60001`. Bump the port
number (`-port2`, …) for every rebuild, even for packaging-only changes —
Android treats a non-increasing versionCode as a downgrade and refuses the
update.

## Rebuilding

```sh
scripts/build-apk.sh
```

Pipeline: cargo builds the cdylib for both ABIs (NDK clang directly, no
cargo-ndk) → `.so` files staged into `android/app/src/main/jniLibs` →
persistent keystore generated on first run (`keystore/`, mode 600, gitignored
— **back it up; losing it means updates can never be installed**) → Gradle
`assembleRelease` → signed APK copied to
`~/workspace/your_files/PhotoCraft-<version>-universal.apk`.

Re-pin to a new upstream tag:

```sh
scripts/pin-upstream.sh v0.7.0   # fetch, checkout, re-apply layers line, bump VERSION_NAME
scripts/build-apk.sh
```

Everything stays local: no repo is created, nothing is pushed. When the
GitHub token works again, `scripts/release.sh` prints the manual publishing
checklist (create `areilly111/CraftApps-Android`, upload the APK +
`SHA256SUMS.txt` to a release tagged `photocraft-v<upstream>-port<build>`;
the updater picks it up automatically).

## GPU fallback (deliberate deviation)

The desktop relaunches the process with a CPU adapter when wgpu init fails.
A `NativeActivity` cannot relaunch its process (and winit event loops must
not be recreated in-process), so the port instead picks the best working adapter
up front, as an explicit cascade: **Vulkan → OpenGL ES → CPU canvas**
(`apps/photocraft-android/src/gpu.rs`).

- The instance is built with `Backends::VULKAN | Backends::GL` (both backend
  features are compiled in: eframe's `wgpu` feature → `egui-wgpu/default` →
  `wgpu/default`, which enables `vulkan` *and* `gles`), and the adapter selector
  orders Vulkan first, then GLES, then any non-CPU adapter, then anything
  (SwiftShader) — instead of relying on wgpu's enumeration order.
- Preferences › Performance › GPU backend (`auto`/`vulkan`/`gl`/`cpu`, applies at
  next launch) is honoured: the persisted `preferences.json` is read before the
  wgpu instance exists. A software adapter never drives the GPU canvas — the
  window draws with it while the canvas composites on the CPU (the same
  CPU-canvas path as `PHOTOCRAFT_CPU_CANVAS=1`).
- The cascade module is written for reuse: per-app identity (`GpuAppInfo`) and the
  canvas attach step (`GpuCanvasHost` trait) are parameters, so the six sibling
  ports share it verbatim.

## What's new in v1.1 (0.6.0-port2, versionCode 60002)

- **Updates under Help.** The floating "↓ Updates" button is gone; Help →
  "Check for Updates…" opens the same dialog (Android-only menu item, desktop
  untouched). The auto-check toggle stays in Preferences.
- **Shared suite storage.** Documents default to `/sdcard/CraftApps/Media/`;
  prefs, crash recovery, brush presets and updater state live in
  `/sdcard/CraftApps/Settings/PhotoCraft/` (per-app subfolders keep the future
  apps from colliding). Both are created on first run. This needs
  `MANAGE_EXTERNAL_STORAGE` ("All files access"): on first launch without it, a
  setup screen explains why (PhotoCraft keeps your work in `/CraftApps/Media`,
  shared across the craft apps) with a button to the system settings page.
  Sideload-only, so Play policy is irrelevant. With all-files access, Rust
  `std::fs` reads/writes these paths directly — no SAF needed there.
- **File dialogs** default to `/sdcard/CraftApps/Media/` (in-app egui browser).
  "Open from elsewhere" / "Save a copy elsewhere" go through the Storage Access
  Framework (`ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT` via
  `PhotoCraftActivity extends NativeActivity` + `onActivityResult` → JNI) for
  locations outside `/CraftApps`.
- **SD card option.** Preferences › Storage › "Tablet storage" (default) or
  "SD card…". The SD card opens `ACTION_OPEN_DOCUMENT_TREE`; the tree URI is
  persisted with `takePersistableUriPermission`, and media I/O goes through
  `DocumentsContract` against the persisted tree. Real implementation, no stub.
- **Monorepo updater.** One channel for the whole suite:
  `areilly111/CraftApps-Android` (doesn't exist yet — the updater says so
  honestly on 404/empty). Tags are per-app (`photocraft-v0.6.0-port2`); the
  updater lists releases, filters `tag_name` starting with `photocraft-`, parses
  `(upstream, port)` and compares against the compiled `VERSION_NAME`.
  Asset: `PhotoCraft-0.6.0-port2-universal.apk` + `SHA256SUMS.txt` (verified
  before install), then the system install dialog as before.
- **Stylus section.** Preferences › Stylus (Android only): the pen pressure
  toggle (links the shared `tools.useTabletPressure` pref), a pressure response
  curve (Soft / Linear / Firm — an Android-only pref, hidden on desktop, applied
  at the single choke point `Stylus::pressure()`), and a live pen-test box that
  shows the pressure the app sees in real time, with peak-hold. Honest about
  limits: tilt, twist and the eraser end aren't reported by the platform (winit
  0.30 drops them) — no fake toggles.
- **Plug-ins.** Sandboxed WebAssembly (`photocraft-plugins`, wasmi 2.0
  `portable-dispatch`, pure Rust — no native code, so it's portable by design;
  14/14 host tests pass). The "Additional Plug-ins Folder" pref defaults to
  `/sdcard/CraftApps/Plugins/` on Android (created on first run); plug-ins there
  load at startup and appear under Filter › Plug-ins. "Install Plug-in…" has a
  "Browse…" button that picks a `.wasm` through the port's file picker
  (`/CraftApps` files install by path; SAF picks from elsewhere install as
  base64) — no path-typing dead end.
- **Release profile.** The Android build uses the workspace `release` profile
  (thin LTO, `codegen-units = 1`, opt-level 3); `build-apk.sh` passes no
  dev-profile flags. Mobile-hostile shared defaults were *not* changed (flagged
  for the user instead): `performance.memoryUsageMb` defaults to 8192 (8 GB —
  more than most tablets have) and `performance.cacheTileSize` to 8192 (a
  256 MB GPU tile).

### Porting checklist for the sibling apps

Each future port (DesignCraft, EffectCraft, PdfCraft, LightCraft, FilmCraft,
VectorCraft) repeats this verification: the updater and stylus-settings modules
are written once and reused verbatim (per-app values — display name, package,
`VERSION_NAME`, tag prefix, settings subfolder — arrive as parameters; see
`GpuAppInfo`/`GpuCanvasHost` in `gpu.rs` and `StorageAppInfo` in `storage.rs`).
Plugin systems per app (checked 2026-10-10): PhotoCraft `plugins`, EffectCraft
`plugin` + `script`, VectorCraft `plugins` — all sandboxed WASM via wasmi
(portable, no native code); DesignCraft, PdfCraft, LightCraft, FilmCraft have
no plugin crates. Each port must re-verify: the plugin crate is in its
dependency graph, the plug-in folder default points at
`/sdcard/CraftApps/Plugins/`, Install works through the file picker, and
installed plug-ins appear in the menus.

## Known v1 limits

- Hardware-unverified (see top).
- File browser is sandboxed to the app's external files dir; no system file
  picker, no "New folder" yet.
- No tilt/twist/eraser-end (winit 0.30 drops them); pressure works.
- No clipboard images, no Photoshop shortcut import (desktop-only concepts).
- The update channel repo doesn't exist yet (publishing is a later step).

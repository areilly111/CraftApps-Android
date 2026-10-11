# EffectCraft for Android (tablet)

A native Android port of [storytold's EffectCraft](https://github.com/storytold/effectcraft)
(v0.7.0, pure-Rust motion-graphics / compositing app targeting After Effects parity)
for tablets, with an in-app self-updater. Built on the shared `craftapps-shared`
shell crate (updater, GPU cascade, `/sdcard/CraftApps` storage, SAF, JNI) — the
android crate is a thin shell over it plus EffectCraft's app-specific file
browser and eframe shell.

**Status: 0.7.0-port1, `cargo check`/`clippy` clean for `aarch64-linux-android`,
hardware-UNVERIFIED.** No Android hardware was available at build time — the
port has never run on a real tablet.

## What was built

- **`upstream/apps/effectcraft-android/`** — a Rust `cdylib` crate, member of
  the upstream workspace (excluded from `default-members`, like the
  vectorcraft-android port). `android_main` (android-activity 0.6
  `native-activity` contract) builds the Android platform services and runs the
  *same* `EffectcraftApp` as the desktop through `eframe::run_native` with
  `NativeOptions::android_app`. `VERSION_NAME = "0.7.0-port1"`.
- **Upstream mods** (in `upstream/`, mirroring the PhotoCraft port's Android
  diff, adapted — EffectCraft has no `Services` struct; the engine owns all
  file I/O through `Session::execute` commands):
  - `crates/ui-egui/src/lib.rs`: Android-only `AndroidHooks` /
    `AndroidStorageHooks` + `Hooks::android_hooks` (`#[cfg(target_os =
    "android")]`).
  - `crates/ui-egui/src/menus.rs`: Help → "Check for Updates…" appended to the
    Help menu on Android only (the item lives outside the engine's menu tree —
    it needs no engine command), dispatched through the new
    `help.checkUpdates` invoke arm, enabled without a project.
  - `crates/ui-egui/src/panels/settings.rs`: Settings → Storage page
    (Android-only) driving the shell hooks (shared folders / SD-card option).
  - `crates/ui-egui/Cargo.toml`: eframe's `accesskit` feature is now enabled
    on every platform *except* Android — eframe 0.36.2 `compile_error!`s on
    `accesskit` + `android-native-activity` together, and the port needs the
    latter. Nothing in ui-egui uses accesskit APIs; the desktop app enables
    the feature itself.
  - `Cargo.toml`: `apps/effectcraft-android` excluded from `default-members`
    (its android-activity/JNI deps don't build for host targets).
- **File I/O** (`apps/effectcraft-android/src/`): EffectCraft's `Hooks` file
  pickers are synchronous (`Fn` returning picked paths), so a hook opens the
  in-app browser (`file_browser.rs`) and reports "cancelled" for that
  invocation; the shell (`shell.rs`) reruns the matching engine command with
  the picked paths once the browser answers:
  - Open Project → `file.open {"path"}`; Import → `file.import {"paths",
    "background": true}` (the desktop's background-import behaviour).
  - Save / Save As (`.ecproj`) and the other save dialogs (frame, preview,
    Lottie, presets, templates, collect-files, watch folder) open the browser;
    the default name recovers the original command (`file.saveAs`,
    `comp.saveFrameAs`, `render.saveCurrentPreview`, …). Render Queue output
    and other direct callers get immediate default paths under
    `EffectCraft/{Projects,Exports}/` and handle them themselves.
  - `content://` URIs (system picker choices, SD-card location) are staged
    into the cache as plain files the engine can read.
  - Scripting (boa) and WebAssembly effect plug-ins (wasmi) come from
    `effectcraft_host::session()`, exactly like the desktop — both runtimes
    are pure Rust and check clean for `aarch64-linux-android`.
  - Settings, shortcut presets, crash recovery and the `Plug-ins` folder live
    under `effectcraft_host::config_dir()` (shared `/sdcard/CraftApps/Settings`
    when granted, else the app's private dir); UI state persists to
    `ui.json` on exit.
- **GPU** (`EcHost` newtype for the shared `GpuCanvasHost` trait — the orphan
  rule forbids implementing it for the upstream app type directly): Vulkan →
  GLES → CPU cascade for the window; the adapter is recorded for Help →
  System Compatibility Report, the device is watched for errors, and the
  `GpuFailureBridge` from the desktop `main.rs` is installed. The viewer's GPU
  compositor initializes itself through the app's own `init_gpu`.
- **`android/`** — Gradle project (`app.effectcraft.android`, minSdk 29,
  arm64-v8a + x86_64, versionCode 70001, versionName 0.7.0-port1). The Rust
  `.so` files are prebuilt and staged into `jniLibs`; `EffectCraftActivity`
  (a `NativeActivity` subclass) forwards SAF picker results to the
  `Java_app_effectcraft_android_EffectCraftActivity_nativeOnActivityResult`
  JNI entry; one tiny Java `ContentProvider` (`ApkProvider`) serves downloaded
  APKs to the system installer. Signed with the **suite keystore**
  (`craftapps-android/keystore/`) shared by all CraftApps ports (updates keep
  working across the suite).
- **Updater** — Help → "Check for Updates…" (plus auto-check): polls the
  shared `areilly111/CraftApps-Android` release feed, filters tags starting
  with `effectcraft-`, downloads the `-universal.apk` with progress, verifies
  it against `SHA256SUMS.txt`, and hands it to the installer via
  `app.effectcraft.android.apkprovider`.

## Verification

- `cargo check -p effectcraft-plugin --features wasm --target
  aarch64-linux-android` — clean (wasmi is pure Rust with portable dispatch).
- `cargo check -p effectcraft-script --target aarch64-linux-android` —
  clean (boa 0.22 compiles for Android; no non-portable code found).
- `cargo check -p effectcraft-android --target aarch64-linux-android` —
  clean, zero warnings.
- `cargo clippy --no-deps -p effectcraft-android -p effectcraft-ui-egui
  --target aarch64-linux-android -- -D warnings` — clean. (A full-workspace
  clippy run still trips two pre-existing `chunks_exact_to_as_chunks` lints
  in `crates/media/src/convert.rs` under this toolchain's newer clippy —
  unrelated to the port, left untouched.)
- Desktop (host) `cargo check -p effectcraft-ui-egui` — clean (the Android
  mods are `#[cfg(target_os = "android")]`-gated; the accesskit change keeps
  the feature on for host builds).

## Known port1 limitations

- Specialised file picks (roto/face model installers, shortcut-preset
  browser, LUT picker, …) answer "cancelled" — only project open, media
  import and the save dialogs are wired to the browser.
- No audio preview device (the desktop's cpal backend has no Android path
  yet); no system clipboard text hook; `ctx.open_url` links rely on eframe's
  Android backend.
- "Save elsewhere…" (SAF create-document) is not offered: saves always land
  on plain filesystem paths the engine can write.

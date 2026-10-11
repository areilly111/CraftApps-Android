# CraftApps Launcher for Android

An unofficial Android port of [storytold/craft-launcher](https://github.com/storytold/craft-launcher):
one place to install, update and open every app in the Craft App Suite,
rebuilt for the Android ports.

## What it does

A grid of app cards — PhotoCraft, DesignCraft, VectorCraft, LightCraft,
PdfCraft, EffectCraft, FilmCraft, plus the launcher itself — each showing its
icon color, install status, installed version, latest release version and
release date, with **Install / Update / Open** buttons.

- **Releases**: polled from the single `areilly111/CraftApps-Android` repo,
  filtered by per-app tag prefix (`photocraft-v…`, `designcraft-v…`, …).
  The repo is private, so the unauthenticated poll 404s — the launcher shows
  "release info unavailable" instead of erroring. No tokens embedded.
- **Install**: downloads the `-universal.apk`, verifies its SHA-256 against
  the release's `SHA256SUMS.txt`, then hands it to the system installer
  (`REQUEST_INSTALL_PACKAGES`; the system shows its own confirmation dialog).
- **Open**: fires the app's launch intent via PackageManager.
- **Status**: installed `versionCode`/`versionName` per package via
  PackageManager (the manifest's `<queries>` block keeps this working on
  Android 11+).

## Layout

- `upstream/` — Rust workspace; `apps/craftlauncher-android` is the cdylib
  (`android_main` → eframe/egui UI). Reuses `craftapps-shared` for the JNI
  bridge, HTTP, SHA-256 and the APK install intent.
- `android/` — Gradle project (`app.craftlauncher.android`,
  `CraftLauncherActivity`, `ApkProvider`).

## Build

```sh
scripts/build-apk.sh launcher   # → ~/workspace/your_files/CraftLauncher-<version>-universal.apk
```

Signed with the suite keystore (`keystore/craftapps-suite.keystore`).
versionCode `10001` = `1.0.0-port1`.

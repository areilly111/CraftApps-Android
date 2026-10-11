# PdfCraft for Android (tablet)

A native Android port of [storytold's PdfCraft](https://github.com/storytold/pdfcraft)
(v0.5.0, pure-Rust PDF reader/editor targeting Acrobat parity) for tablets, with
an in-app self-updater. Built on the shared `craftapps-shared` shell crate
(updater, GPU cascade, `/sdcard/CraftApps` storage, SAF, JNI) — the android crate
is a thin shell over it plus PdfCraft's file browser and eframe shell.

**Status: 0.5.0-port1, hardware-UNVERIFIED.** No Android hardware was available
at build time — `cargo check`/`clippy` are clean for `aarch64-linux-android`,
but the port has never run on a real tablet.

## What was built

- **`upstreams/pdfcraft/apps/pdfcraft-android/`** — a Rust `cdylib`
  crate, member of the upstream workspace. `android_main` (android-activity 0.6
  `native-activity` contract) builds the Android platform services and runs the
  *same* `PdfCraftApp` as the desktop through `eframe::run_native` with
  `NativeOptions::android_app`.
- **Upstream mods** (in `upstreams/pdfcraft/`, Android-gated with
  `#[cfg(target_os = "android")]` — desktop and web builds are unaffected):
  - `crates/ui-egui/src/pickers.rs`: on Android, rfd has no backend (it doesn't
    even compile there), so `ask()` queues an `AndroidPick` (kind + the dialog's
    filters, suggested name and title) for the shell's file browser instead of
    spawning an rfd panel. The shell answers through
    `PdfCraftApp::answer_android_pick`, which runs the pick's own follow-up
    closure behind the engine's panic guard — every flow (Open, Combine,
    Insert, Save As, exports, …) works unchanged. `rfd::AsyncFileDialog` is
    replaced by a `pickers::FileDialog` shim that records the dialog metadata on
    Android and aliases rfd's type elsewhere (call sites compile unchanged on
    every platform).
  - `crates/ui-egui/src/lib.rs`: `android_write_uri` / `android_read_uri` hooks
    (SAF `content://` read/write), `android_update_hook` (Help ▸ Check for
    updates), `android_open_url` (links in the system browser), and
    `write_picked_file`, which routes every save-dialog write: `content://`
    URIs go through the shell's SAF writer, plain paths through the atomic
    file write.
  - `crates/ui-egui/src/updates.rs`: Help ▸ Check for updates calls
    `android_update_hook` on Android instead of the desktop release dialog.
  - `crates/ui-egui/Cargo.toml`: `rfd` and `arboard` are gated out on Android
    (neither compiles there); eframe's `accesskit` feature is gated out on
    Android (it's a compile error with `android-native-activity` —
    `egui::accesskit` itself stays available).
  - `create_ui.rs`, `zoom_snap.rs`: clipboard features follow the web
    (`None`) on Android — no system clipboard backend.
- **File I/O design** — PdfCraft has no `Services` struct to map (its
  `PdfCraftApp::new()` takes no arguments and the UI calls rfd directly), so
  the port answers the app's own dialog requests: the shell's [`FileBrowser`]
  shows the equivalent of each queued pick. Answers are plain filesystem paths,
  except a SAF "save elsewhere" choice, which arrives as a `content://` URI
  string that `write_picked_file` routes through the shell's SAF writer.
  "Browse other locations…" (Open) stages the picked file's bytes into the
  app cache and answers the staged path.
- **`pdfcraft/android/`** — Gradle project (`app.pdfcraft.android`, minSdk 29,
  arm64-v8a + x86_64, versionCode 50001, versionName 0.5.0-port1). The Rust
  `.so` files are prebuilt and staged into `jniLibs`; one tiny Java
  `ContentProvider` (`ApkProvider`) serves downloaded APKs to the system
  installer; `PdfCraftActivity` (a `NativeActivity` subclass) forwards SAF
  picker results to Rust. Signed with the **suite keystore**
  (`craftapps-android/keystore/craftapps-suite.keystore`) — new apps share one
  suite key (per the suite decision), unlike PhotoCraft which keeps its own.
- **Updater** — Help ▸ Check for updates (plus auto-check): polls the shared
  `areilly111/CraftApps-Android` release feed, filters tags starting with
  `pdfcraft-`, downloads the `-universal.apk` with progress, verifies SHA-256
  against the release's `SHA256SUMS.txt`, then fires the system install dialog.
- **Storage** — `/sdcard/CraftApps/Media/` is the default open/save location
  (in-app browser rooted there), `/sdcard/CraftApps/Settings/PdfCraft/` holds
  `app.ron`/recovery/updater state. Needs "All files access" (sideload-only);
  first launch walks the user to the toggle, or they can choose internal
  storage. A floating Storage button opens the shared-folders settings (tablet
  storage vs. SD-card folder via the SAF tree picker).
- **GPU cascade** — Vulkan → OpenGL ES → CPU, via the shared `gpu` module
  (`PcHost` newtype, since the orphan rule forbids implementing the shared
  trait for the upstream app type). The adapter is logged, a software renderer
  is announced to the user (like the desktop), and the device is watched for
  errors instead of panicking.

## Limitations (port1)

- The system clipboard has no backend on Android: Create ▸ Clipboard reports
  "The clipboard has no image or text", like the web build.
- A document saved through the system picker ("Save elsewhere…") keeps a
  `content://` URI as its path: later saves go back through SAF, and Open
  Recent reopens it, but the last-session auto-reopen skips it (it only
  reopens plain files).
- Printing: no print spooler exists on Android, so the printer list is empty and
  Print falls back to "Save as PDF" through the file browser (the desktop's
  no-printer path).

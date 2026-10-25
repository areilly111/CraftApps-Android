# AGENTS.md — CraftApps Android Ports

This file is for AI agents working on the CraftApps Android ports. It captures
project-specific lessons, conventions, and gotchas. The workspace-level
`~/AGENTS.md` has general environment lessons; this file is CraftApps-specific.

## Project Overview

**Repository:** `https://github.com/areilly111/CraftApps-Android`
**Local:** `~/workspace/craftapps-android/` (VM) → `~/muse/CraftApps/craftapps-android/` (homelab)

Five individual Android APKs (NOT a unified suite APK):
- **PhotoCraft** (`photocraft/`) — Photoshop clone, the reference port
- **VectorCraft** (`vectorcraft/`) — Vector editor
- **LightCraft** (`lightcraft/`) — Photo editor
- **DesignCraft** (`designcraft/`) — Design tool
- **PdfCraft** (`pdfcraft/`) — PDF editor

**FilmCraft is EXCLUDED** until its file situation is properly solved (user directive).

Each port = `<app>/upstream/apps/<app>-android/` (Rust cdylib crate) + `<app>/android/` (Gradle project).

## Documentation Rule (user directive)
**Every fix must be documented: what was broken, how it was fixed, and where.**
When you fix something:

1. Add the fix to the fix log under `docs/` (one file per release:
   `docs/fixes-<version>.md` — symptom → root cause → the change, file by file →
   how to verify it → what is still open).
2. Add a short, durable lesson to **this** file (the "HARD-WON LESSONS" and
   the "Known Issues & TODO" lists), so the next agent finds it without
   reading the whole log.
3. If the fix changes an upstream patch, update `docs/android-upstream-patches.md`
   (the list that must be re-applied when re-pinning upstream) and `PATCHES.md`.
4. If it changes a documented protocol (the JNI bridge, the picker bridge, the
   pen runtime), update that protocol's section here too.

A fix that is not written down will be re-broken.

## Critical Build Rules

### Build on the HOMELAB, never the VM
- The VM (2 CPU / 7.7 GB) OOMs on release builds and gets restarted, killing long builds.
- Homelab: 12 CPUs, 15 GB RAM. Rust release build ~6 min.
- SSH: `~/workspace/muse-relay/relay.py` (args: `22223 100.112.120.121 22`) → `ssh -i /home/hatch/.ssh/id_ed25519 -o StrictHostKeyChecking=no -p 22223 anthony@127.0.0.1`
- **The relay dies on runtime restarts AND spontaneously — relaunch before use.**
- Build command (on homelab):
  ```
  cd ~/muse/CraftApps/craftapps-android
  export PATH=$HOME/.cargo/bin:$PATH
  export CARGO_TARGET_DIR=$HOME/muse/CraftApps/build-target
  export ANDROID_HOME=$HOME/muse/CraftApps/dependencies/android-sdk
  export JAVA_HOME=$HOME/muse/CraftApps/dependencies/jdk-17
  ./scripts/build-apk.sh <app>
  ```

### Sync: use `scp` for individual files, NOT full-tree rsync
- Full-tree rsync VM→homelab hangs/errors (IO code 11).
- `scp` works but is slow (~5 min for 142 MB APK).
- **Always pass the FULL destination file path** (directory-only destinations fail).

### Versioning (user directive: incremental builds)
- Same package name + same signing key + **higher versionCode** every build.
- Formula (in `scripts/build-apk.sh`): `100000 + major*10000 + minor*100 + patch`
- Version source: `pub const VERSION_NAME: &str` in `<app>/upstream/apps/<app>-android/src/lib.rs`
- **Bump VERSION_NAME before every build you ship.**

### Build one at a time
- Parallel builds fight over the shared cargo lock and RAM (user directive).

### Pull latest upstream before every build
- Fetch latest release from `storytold/<app>` GitHub into `<app>/upstream/`
- Re-apply Android patches on top. Never build from stale upstream.

## Architecture

### Shared Code (`shared/`)
- `shared/src/pen.rs` — Stylus settings, pressure pipeline, button remap, JNI bridge (`handle_pen_event`, `on_gesture`)
- `shared/src/gestures.rs` — Multi-finger tap recognition (the "stylus mode" gestures), fed by egui's touch events
- `shared/src/modifiers.rs` — Ctrl/Shift/Alt tracking (winit sends no `ModifiersChanged` on Android)
- `shared/src/gpu.rs` — HW acceleration cascade (Vulkan → GLES → CPU)
- `shared/src/saf.rs` — Storage Access Framework integration
- `shared/src/picker_bridge.rs` — File-based picker bridge (see below)

**Write pen/stylus code ONCE for PhotoCraft, reuse verbatim** (user directive).

The shared crate builds on the host (`cargo test` in `shared/`), so the pure logic (pen settings, gestures, modifiers) is unit-tested there. The host build needs the `x11` dev-dependency for eframe — without it winit refuses to compile off Android.

### Per-App Structure
```
<app>/
  upstream/                    # Upstream source (NEVER commit our changes here without documenting)
    apps/<app>-android/       # Our Android cdylib crate
      src/
        lib.rs                # VERSION_NAME, entry point
        pen.rs                # JNI bridge (nativeOnPenEvent, nativeOnGesture)
        shell.rs              # AndroidShell: eframe wrapper, menus, dialogs
        keyboard.rs           # Modifier key JNI bridge
        file_browser.rs       # The in-app file browser (PhotoCraft)
  android/                    # Gradle project
    app/
      src/main/java/app/<app>/android/
        <App>Activity.java    # NativeActivity subclass
      libs/                   # Local JARs (if any)
```

### JNI Bridge Protocol
Java (`<App>Activity.java`) → Rust (`pen.rs`):

**Heads up: on a `NativeActivity` the Java side never receives input events (see "THE NATIVEACTIVITY INPUT TRAP" below). The hooks below exist for a future non-`NativeActivity` shell; they do not fire today.**

1. **`nativeOnPenEvent(action, toolType, buttonState, actionButton, x, y)`**
   - Called from `dispatchTouchEvent` for pen-relevant events
   - `action`: MotionEvent action masked
   - `toolType`: `getToolType()` (0=unknown, 1=finger, 2=stylus, 3=mouse, 4=eraser, 5=palm)
   - `buttonState`: `getButtonState()` bits
   - Rust: `craftapps_shared::pen::handle_pen_event(...)` → `on_pen_event(...)`

2. **`nativeOnGesture(fingers, x, y)`**
   - Called for 3-finger or 4-finger taps (stylus mode)
   - Rust: `craftapps_shared::pen::on_gesture(...)`
   - **Superseded:** the taps are recognised in Rust now (`shared/src/gestures.rs`).

3. **`nativeOnModifierState(modifiers)`**
   - Bitmask: bit 0=Ctrl, bit 1=Shift, bit 2=Alt
   - Java tracks via `dispatchKeyEvent`; Rust stores in atomic
   - **Superseded:** the modifiers are tracked in Rust now (`shared/src/modifiers.rs`).

### File Picker Bridge (CRITICAL)
**Do NOT use JNI for SAF pickers.** The old JNI path (`launchForResult` via JNI) fails silently when called from the eframe render thread.

**Use the file-based bridge** (`shared/src/picker_bridge.rs`):
1. Rust writes `<bridge>/request.json`:
   ```json
   {"id": 1, "kind": "open"|"save"|"folder", "mimes": [...], "name": "...", "multiple": false}
   ```
2. Java `Activity` polls every 400ms via Handler, launches `ACTION_OPEN_DOCUMENT`/`ACTION_CREATE_DOCUMENT`/`ACTION_OPEN_DOCUMENT_TREE` with `startActivityForResult` directly (no JNI)
3. Java writes `response.json`:
   ```json
   {"id": 1, "cancelled": false, "paths": ["/data/.../inbox/1/0-photo.png"], "uri": "content://...", "name": "photo.png"}
   ```
   - **open** → the picked files are copied into `<bridge>/inbox/<id>/` and their local paths are returned
   - **save** → the *document URI* is returned (the app writes through SAF, so the file lands where the user chose)
   - **folder** → the tree URI is returned
4. Rust polls `picker_bridge::poll_response()`

**The bridge MUST be started.** It is started in `PhotoCraftActivity.onCreate` (after `super.onCreate`, which is what loads the native library). `NativeActivity` *does* have a usable `onCreate` — the old note claiming otherwise was wrong.

**The response must be polled every frame**, not only while a dialog is pending: handing the reply over to the system picker closes the browser, so the reply waits in its own slot (`FileBrowser::picker`) and the poll happens unconditionally in `FileBrowser::show`.

**Bridge directory:** `<external files dir>/.picker` (the app's own `getExternalFilesDir(null)`), passed to Rust through the `CRAFTAPPS_BRIDGE_DIR` environment variable. Always writable, needs no "All files access", and private to the app. The old `/sdcard/CraftApps/.picker` remains only as a fallback.

## THE NATIVEACTIVITY INPUT TRAP (read this before touching any input code)
**A `NativeActivity` never sees input events in Java.** Established 2026-10-19 from the AOSP sources:

- `NativeActivity.onCreate` calls `getWindow().takeInputQueue(this)`.
- AOSP `PhoneWindow.takeInputQueue` just stores the callback, and AOSP documents the contract as: *"Take ownership of this window's InputQueue. The window will no longer read and dispatch input events from the queue; it is your responsibility to do so."*
- The native side (winit) owns the queue, so the Java view hierarchy — and the Activity — get **no touch, key or generic-motion events**.

Consequences (all of these were broken until 0.4.0):
- `dispatchTouchEvent` / `dispatchKeyEvent` / `dispatchGenericMotionEvent` overrides **never run**. Every Java-side pen side-channel, gesture detector and modifier tracker was dead code.
- Multi-finger gestures now live in **Rust** (`shared/src/gestures.rs`), fed by egui's `Event::Touch` stream (winit → egui-winit → egui). That stream carries the pointer id, the input device id, the position and the pressure — everything a tap needs.
- Keyboard modifiers now live in **Rust** (`shared/src/modifiers.rs`): winit's Android backend never sends `ModifiersChanged`, so egui's modifier state is always empty; the Ctrl/Shift/Alt keys are tracked from the key events and published into egui's input state (Ctrl is published as egui's `command`, as egui asks off macOS).
- The tool type (`TOOL_TYPE_STYLUS`/`ERASER`/`PALM`) and the barrel-button state are **not** in winit's events. They come from the **vendored android-activity patch** (see "The S Pen's Side Button DOES Work" above): the patch records them as the motion events pass through, and the port feeds them to the shared pen runtime.

**Do not add new Java input hooks.** They will never fire.

## S Pen / Stylus (HARD-WON LESSONS)

### The S Pen's Side Button DOES Work on the Tab S6 Lite
**The old note here said the opposite. It was wrong** (corrected 2026-10-19).

- The Tab S6 Lite's S Pen is a passive Wacom EMR stylus (no battery, no
  Bluetooth), so Samsung's **S Pen Remote SDK** (`com.samsung.android.sdk.penremote`)
  cannot work — that SDK needs a Bluetooth S Pen and, per Samsung's own
  documentation, "devices of the Note series".
- **But the button is still delivered to apps** the standard Android way, and
  drawing apps use it: `MotionEvent.getButtonState()` with
  `BUTTON_STYLUS_PRIMARY`, and `MotionEvent.getToolType()`.
- On the Tab S6 Lite the button is signalled by *replacing* the motion action
  with a proprietary variant: **211 / 213 / 212 / 214** = down / move / up /
  cancel with the button held. The NDK has no name for those, so they arrive
  as `MotionAction::__Unknown`, and **winit drops every event it does not
  recognise** — which is why the stroke used to stop while the button was
  held, and why the button never appeared.
- The old test ("`BUTTON_STYLUS_PRIMARY`, `ACTION_BUTTON_PRESS`,
  `KEYCODE_STYLUS_BUTTON_PRIMARY`, codes 211-214 — none fire") was run through
  the Java `dispatchTouchEvent`, **which never runs** (see the input trap
  above). It proved nothing.

**The fix (0.4.1):** android-activity is vendored with a patch
(`patches/android-activity-0.6.1-stylus.patch`) that

1. maps 211/213/212/214 back to down/move/up/cancel, so the events keep
   flowing while the button is held, and
2. records the stylus data (the button bits, the tool type, the position) of
   every motion event, because the app never sees the `MotionEvent` itself.

The port feeds that to the shared pen runtime, so the **Stylus Settings →
Pen buttons** remapping (Undo, Eyedropper, tool wheel, …) now works, as do
the eraser end and palm rejection.

**The button works while the pen hovers too (0.4.3).** Three paths, all of
them needed:

1. The button bits are carried by *every* event while the button is held
   (`MotionEvent.getButtonState()`) — the shared runtime must read them for
   **every** action, not only for the touch actions.
2. Android 14+ also reports the buttons as **key events**
   (`KEYCODE_STYLUS_BUTTON_PRIMARY` = 308). egui drops unknown key codes,
   so the **vendored patch** translates them into the motion-event protocol.
3. The Stylus Settings show **"Pen hovering"** and the **last stylus event**
   (action, button bits, tool type) — the diagnostic. If the button does not
   work while hovering, read that line: `buttons 0x20` means the app has the
   press, `0x00` means the tablet did not report it.

`ACTION_HOVER_EXIT` (10) means the pen **left** the screen — it must clear
the hovering flag, not set it (that bug shipped in 0.4.2).

### The pen's pressure
Unchanged: it comes through winit's `Touch.force` (the `MotionEvent`
pressure), so it never needed the Java side-channel.

### Pen Event Forwarding
- The Java side-channel (`nativeOnPenEvent`) is **dead** on a `NativeActivity`
  — do not rely on it (see the input trap above). The live path is the
  vendored android-activity patch plus `AndroidShell::feed_stylus_state`.
- `dispatchHoverEvent` **cannot be overridden** on NativeActivity (javac error).
- `TOOL_TYPE_PALM` is API 35+; define as local constant `TOOL_TYPE_PALM = 5` for compileSdk 34.

### Stylus Settings
- Location: Edit → "Stylus Settings…" menu (NOT a floating button — user had them removed).
- Settings persist as `pen.json` in `/sdcard/CraftApps/Settings/<App>/`.
- Includes: pressure curve/sensitivity/smoothing, input mode, palm rejection, button remapping, stylus mode gestures.
- Button-state indicator dots at top (diagnostic — if they don't light up, events aren't arriving).

## Storage
- Shared: `/sdcard/CraftApps/Media/<App>/`, `/sdcard/CraftApps/Settings/<App>/`
- Requires `MANAGE_EXTERNAL_STORAGE` (all 5 apps have it in manifest).
- Standard permissions: `READ_EXTERNAL_STORAGE` (maxSdk 32), `WRITE_EXTERNAL_STORAGE` (maxSdk 29), `INTERNET`, `REQUEST_INSTALL_PACKAGES`.

## Keystore & Signing
- PhotoCraft keeps its shipped keystore (NEVER lose it — updates would fail).
- Suite apps share one suite keystore: `photocraft/android/app/keystore/craftapps-suite.keystore`
- Config: `photocraft/keystore/keystore.properties` (git-ignored)
- **Never commit keystores, keystore.properties, or secrets.**

## Upstream Patch Requirements
Document EVERY patch in `PATCHES.md` (recreate if missing). Minimum:
- eframe 0.36.2: remove `"accesskit"` from eframe features in `crates/ui-egui/Cargo.toml` (conflicts with `android-native-activity`)
- Each app's port structure
- All S Pen work
- The `menus.rs` Android-menu patch
- The picker bridge
- Any Samsung SDK integration (if applicable)

**Fix logs live in `docs/`** (`docs/README.md` is the index). `docs/fixes-0.4.0.md` documents the 0.4.0 fixes (the `NativeActivity` input trap, the Rust gesture detection, the picker bridge), `docs/fixes-0.4.1.md` the S Pen's side button, `docs/fixes-0.4.3.md` the hover support and the two missing upstream patches. `docs/audit-2026-10-19.md` is the audit that found them. `docs/android-upstream-patches.md` lists the patches that must be re-applied when re-pinning a new upstream tag.

**This file is the single source of truth.** The workspace-level `~/AGENTS.md` is a symlink to it — never let two copies drift apart.

## Code Rules (from upstream AGENTS.md)
- **No `unwrap`/`expect`/`panic`/`todo` in non-test code**
- Handle lock poisoning via `unwrap_or_else(PoisonError::into_inner)`
- **Clean-room only:** Reference repos are ideas/architecture only. NEVER copy code.
- Original code only (user directive: "Never copy code make sure code is original and ours")

## Gradle Gotchas
- **Do NOT proxy Gradle** through the egress proxy. Keep `~/.gradle/gradle.properties` free of proxy lines.
- Homelab-only: `scripts/build-apk.sh` wraps the `hatch-egress-ca` keytool import in a `[ -f ... ]` check. **A fresh rsync overwrites this — re-apply after syncing scripts/.**
- If Gradle daemon cache corrupts: `rm -rf ~/.gradle/daemon/8.7`
- A build's outer shell can exit 0 while recording `EXIT:101` — always check the `EXIT:` line and actual APK output.

## Testing & Delivery
- **Test before committing.** Push only completed, working, verified code.
- Hardware work is "built, awaiting your test" until the user's verdict — **never call it fixed first**.
- When user asks for an APK, send only the most recent build, zipped if asked.
- User gets APKs from homelab SMB (`~/muse/CraftApps/apks/`) and zips in chat.
- User watches builds live: `ssh -p 22223 anthony@127.0.0.1` then `tail -f /tmp/photocraft-build3.log`

## Known Issues & TODO
*(The full audit: `docs/audit-2026-10-19.md`; the doc index: `docs/README.md`.)*
- [x] Port stylus mode gestures to VectorCraft, LightCraft, DesignCraft, PdfCraft — the detector is in the shared crate (`shared/src/gestures.rs`); each port still has to call it from its shell
- [ ] Wire `poll_response()` into other 4 apps' file browsers — **their file dialogs hang after a pick**: the reply waits in the SAF pending slot, which the bridge's request codes never reach
- [ ] Start the picker bridge in VectorCraft, LightCraft and PdfCraft — they define `startPickerBridge()` but never call it, so their system picker never opens (DesignCraft does, in `onCreate`)
- [ ] Set `CRAFTAPPS_BRIDGE_DIR` in the other 4 apps — without it they share `/sdcard/CraftApps/.picker` (one directory for five apps) and need "All files access"
- [ ] Feed the stylus data to the pen runtime in the other 4 apps (`AndroidShell::feed_stylus_state`) and give their workspaces the same `[patch.crates-io] android-activity` entry — the S Pen's button, the eraser end and palm rejection do nothing in them
- [ ] Track the keyboard modifiers in the other 4 apps (`shared/src/modifiers.rs`) — every Ctrl/Shift/Alt shortcut is dead in them
- [ ] Move LightCraft's and PdfCraft's folder picker to the bridge (`saf::launch_open_document_tree_bridge`) — the JNI one launches the picker from the render thread, where it fails
- [x] Migrate folder picker to the file bridge (PhotoCraft: `PickerKind::Folder`)
- [x] Remove debug Toasts from PhotoCraftActivity
- [ ] Apply menu reorganization (Help → Check for Updates, Edit → Stylus Settings) to other 4 apps
- [ ] Rebuild LightCraft (overnight APK predates pen fixes)
- [x] Recreate `PATCHES.md` with full documentation — the port's patches are in `PATCHES.md`, the upstream re-pin list in `docs/android-upstream-patches.md`
- [ ] Force-push to GitHub (needs token from user)
- [x] Palm rejection and eraser-end detection — they need the tool type, which now comes from the vendored android-activity patch
- [ ] The S Pen's *hover* position is not known to the app (winit drops hover events): the button works while hovering, but the app cannot draw a hover cursor yet
- [ ] `shared/` builds on the host for `cargo test` only — a plain `cargo check` there fails (winit needs a window system; the `x11` dev-dependency is test-only)

## Reference Repos (ideas only, NEVER copy)
- `https://github.com/morte2437/photocraft-android` — File-based picker bridge pattern
- `https://github.com/2IIZ/spen_remote` — the Samsung **S Pen Remote SDK** (needs a Bluetooth S Pen; it cannot work on the Tab S6 Lite's passive pen. The *button* itself does work — see above)
- `https://github.com/artcraft-android/lightcraft-android` — SAF bridging pattern

## PdfCraft private storage and phone workspace
- Initialize PdfCraft storage and its Java/Rust picker bridge under getFilesDir();
  do not require shared-root/all-files access before showing the UI.
- ACTION_VIEW requires both cold-launch and onNewIntent handling. Keep its
  durable event queue separate from picker response.json and register Java
  native wake methods with System.loadLibrary, not only NativeActivity dlopen.
- Compact inspectors replace the canvas: keep text extraction queued for search.
- Bound dialogs and stack narrow preference rows/theme cards; a scroll frame
  alone does not make desktop rows fit. Test portrait, landscape and translations.
- See docs/fixes-pdfcraft-honor-mobile.md for verification and migration limits.

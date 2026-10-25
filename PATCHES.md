# Android port patches

Every change the Android ports make to the upstream PhotoCraft sources, and
why. The ports live in `<app>/upstream/` (a vendored copy of the upstream
repository) plus `<app>/android/` (the Gradle project) and `shared/`.

The ports are built from `storytold/photocraft` releases with these patches
re-applied on top; nothing here is upstream work.

**Related documents**

- `docs/fixes-0.4.0.md` — the fix log for the 0.4.0 release: the
  `NativeActivity` input trap (why the Java input hooks never fire), the Rust
  gesture detection, the keyboard modifiers and the picker bridge.
- `docs/fixes-0.4.1.md` — the fix log for the 0.4.1 release: the S Pen's
  side button (the vendored android-activity patch that recovers the button
  and the tool type).
- `docs/fixes-0.4.3.md` — the fix log for the 0.4.3 release: the S Pen's
  button while the pen hovers, the two missing upstream patches and the dead
  code of the old JNI picker path.
- `docs/android-upstream-patches.md` — the short list of upstream patches that
  must be re-applied when re-pinning a new upstream tag.

## 7. The vendored android-activity patch (0.4.1)

`android-activity` 0.6.1 is vendored (by `scripts/vendor-android-activity.sh`,
into the git-ignored `vendor/`) and patched with
`patches/android-activity-0.6.1-stylus.patch`, because the S Pen's side
button and the tool type live in the `MotionEvent` — the one piece of input
data winit drops, and the app cannot reach on its own (a `NativeActivity`
never sees input in Java).

The patch does two things:

1. maps Samsung's proprietary "the side button is held" actions
   (211/213/212/214) back to down/move/up/cancel, so the touch events keep
   flowing while the button is held, and
2. records the stylus data (button bits, tool type, action button, position)
   of every motion event, so the app can read it.

The upstream workspace points at the vendored copy with a `[patch.crates-io]`
entry.

---

## 1. `crates/ui-egui/Cargo.toml` — eframe's `accesskit` feature removed

```toml
# NOTE(android-port): eframe "accesskit" removed here; it conflicts with
# "android-native-activity" (eframe refuses to build with both).
```

`eframe` 0.36 enables `accesskit` by default. `accesskit` and
`android-native-activity` cannot be enabled at the same time (eframe's build
fails with a compile error), and the Android ports need
`android-native-activity` for the `NativeActivity` entry point. The feature is
removed in the ui-egui crate, which is the one that pulls eframe in for the
desktop app.

## 2. `apps/<app>-android/` — the Android port crate (new)

A new crate next to the desktop app: a `cdylib` that implements android-activity's
`native-activity` contract (`android_main`) and runs the same egui app through
`eframe::run_native` with `NativeOptions::android_app`.

| File | What it does |
|---|---|
| `lib.rs` | `android_main`, the Android services, the storage gate, the version |
| `shell.rs` | `AndroidShell`: the egui shell plus the Android-only UI (file browser, updater, stylus settings, tool wheel) |
| `pen.rs` | The JNI entries (`nativeOnPenEvent`, `nativeOnGesture`) |
| `keyboard.rs` | The modifier-key JNI entry |
| `file_browser.rs` | PhotoCraft's in-app file browser (answers the file dialog) |
| `services.rs` | The platform services: file dialogs, filesystem, codecs |

The crate does not inherit the workspace lints (`unsafe_code = "forbid"`
there would rule out the JNI this shell needs); it denies
unwrap/expect/panic/todo/unreachable like every other crate, and every
`unsafe` block carries a SAFETY comment.

## 3. `crates/ui-egui/src/menus.rs` — the Android menu entries

The desktop menu bar has no place on a tablet, so the Android build gets its
own entries and a queue the shell drains:

```rust
#[cfg(target_os = "android")]
pub enum AndroidMenuAction { OpenStylusSettings, CheckForUpdates }

#[cfg(target_os = "android")]
pub fn push_android_menu_action(action: AndroidMenuAction) { … }

#[cfg(target_os = "android")]
pub fn drain_android_menu_actions() -> Vec<AndroidMenuAction> { … }
```

`Edit ▸ Stylus Settings…` and `Help ▸ Check for Updates` push into the queue;
the shell (`AndroidShell::ui`) drains it each frame and opens the matching
window.

## 4. The stylus work (port-side, not upstream)

All of it lives in the port and the shared crate — the upstream app is not
touched:

- `shared/src/pen.rs` — the stylus settings (`pen.json`), the pressure curve,
  smoothing, palm rejection, the button remap and the pen-action queue.
- `shared/src/gestures.rs` — the multi-finger tap recognition (the "stylus
  mode" gestures). Fed by egui's `Event::Touch` stream, because a
  `NativeActivity` never delivers touch events to Java (AOSP:
  `takeInputQueue` takes the window's input queue away from the view
  hierarchy).
- `shared/src/modifiers.rs` — the Ctrl/Shift/Alt tracking, because winit's
  Android backend never sends `ModifiersChanged`, so egui's modifier state is
  always empty and every ⌘/Ctrl shortcut is dead without it.
- `shared/src/gpu.rs` — the graphics cascade (Vulkan → GLES → CPU canvas).
- `shared/src/storage.rs` — the suite's shared folders
  (`/CraftApps/Media`, `/CraftApps/Settings/<App>`) and the SAF tree option.
- `shared/src/saf.rs` — the Storage Access Framework: reading and writing
  content URIs, listing SAF trees, creating documents in a tree.
- `shared/src/picker_bridge.rs` + `PhotoCraftActivity.java` — the file picker
  bridge (below).

## 5. The file picker bridge

The system file pickers (`ACTION_OPEN_DOCUMENT`, `ACTION_CREATE_DOCUMENT`,
`ACTION_OPEN_DOCUMENT_TREE`) can only be launched from the UI thread, and the
app asks for a file from the eframe render thread, where a JNI call into the
Activity fails silently. So the request travels through a file instead:

1. Rust writes `<external files>/.picker/request.json`.
2. The Activity polls for it every 400ms (a `Handler` on the main looper),
   launches the picker itself and writes `<external files>/.picker/response.json`.
3. Rust polls `picker_bridge::poll_response()` every frame and answers the
   waiting dialog.

The bridge directory is the app's own external files directory, so it works
without "All files access" and stays private to the app.

## 6. The shared crate (`shared/`)

Code that is identical for all five apps lives in one crate: the stylus
settings, the gestures, the modifiers, the GPU cascade, the shared storage,
the SAF helpers, the picker bridge, the updater and the JNI bridge. Each app's
port is a thin shell over it.

## 8. FilmCraft (port 0.1.0)

FilmCraft was the last unported app (upstream 0.9.0, `storytold/filmcraft`).
It differs from the other five in how it asks for files: its dialog layer is
already asynchronous. `FilmcraftApp::pick` takes a `FileDialog` and a
continuation; a host that provides `HostHooks::run_dialog` (a closure that
returns an `mpsc::Receiver<Vec<String>>`) keeps the frame loop running while
the dialog is open, and `poll_native_dialog` runs the continuation on the
frame the answer arrives. The Android port is built around that hook, so no
thread ever blocks on a picker — least of all the render thread.

### The port crate (`filmcraft/upstream/apps/filmcraft-android/`)

| File | What it does |
|---|---|
| `lib.rs` | `android_main`; storage, pen settings, bridge dir (`CRAFTAPPS_BRIDGE_DIR`), GPU cascade; builds the `Session` (auto-save in the suite settings dir), hands eframe's wgpu state to `FilmcraftApp::set_wgpu` when the plan allows; JNI entries `nativeOnActivityResult` (→ `saf::activity_result_ffi`) and `nativeOnPenEvent` (→ `pen::handle_pen_event`, all six arguments) |
| `shell.rs` | The shared input stack around `FilmcraftApp`: `ModifierKeys` publish, the stylus feed (`drain_stylus_states` → `pen::on_pen_event`, finger/mouse skipped unless a stylus button bit is set), `GestureDetector` → `pen::on_gesture`, and pen-action dispatch (Undo/Redo → `session.execute("edit.undo"/"edit.redo")`, RightClick → a synthesized secondary click; ToolWheel is logged, FilmCraft has no tool-wheel UI). A tap that grew the undo history with an add/insert/place-style step is undone first, so the gesture is the only thing the tap does |
| `picks.rs` | `run_dialog` on the file picker bridge (see §5): a worker thread writes the request, polls `picker_bridge::poll_response()` for its own request id, and sends the paths on the channel. The synchronous pick hooks are deliberately left unset |
| `audio.rs` | `AndroidAudioOut`: the `AudioOut` trait over cpal (see below) |

The Gradle project (`filmcraft/android/`, package `app.filmcraft.android`,
`FilmCraftActivity`) is the shared one: the suite keystore, the same
permission set as the other ports, and the picker-bridge Activity (the bridge
starts in `onCreate`; the input hooks are documented dead code on a
`NativeActivity`).

### Picker answers: paths vs URIs

- **Open** — the Activity copies the picks into the bridge inbox, so the
  engine gets ordinary filesystem paths (multi-select works).
- **Save / Folder** — the bridge answers with a `content://` URI, but the
  engine reads and writes with `std::fs`. `picks.rs` resolves the URI to a
  real path when it is on the primary external-storage volume
  (`content://com.android.externalstorage.documents/document/primary:<path>`
  → `/sdcard/<path>`), which "All files access" covers. A save URI from any
  other provider falls back to the suite media folder
  (`/sdcard/CraftApps/Media/FilmCraft/<display name>`); a folder URI that
  cannot be resolved is dropped (the pick reads as cancelled) and logged.

### Audio

`AndroidAudioOut` implements `filmcraft_ui_egui::AudioOut` with cpal 0.16 —
its Android backend is AAudio (via the `ndk` crate), so there is no oboe
C++ build. F32/I16/U16 device formats are handled (the mixer always fills
f32; non-float formats are converted).

The historical bug is fixed by construction: `source_playback` calls
`sample_rate()` *before* `start()` when it starts source audio, and a zero
rate silences playback. `AndroidAudioOut::new()` pre-queries the default
output device's config and reports that rate (48 kHz stereo when no device
answers), so `sample_rate()` is correct before any stream exists.

### Upstream patches

- `filmcraft/upstream/Cargo.toml` — `[patch.crates-io]
  android-activity = { path = "../../vendor/android-activity" }` (same as
  every port; see §7).
- `filmcraft/upstream/crates/ui-egui/Cargo.toml` — eframe's `accesskit`
  feature (§1), restructured: the base eframe dependency moved into
  `cfg(not(target_os = "android"))` (with `accesskit`) and
  `cfg(target_os = "android")` (without) sections, because Cargo merges the
  features of a duplicated dependency instead of overriding them.

### Deliberately not in this build

- **Whisper / neural voices** — desktop-only extras; the features stay off.
- **Voice-over microphone input** — `session.voiceover.input` is left unset
  (desktop uses a cpal input stream; unported).
- **Stylus settings UI** — `pen::pen_settings_ui` is not surfaced: FilmCraft
  has no menu hook for port UI (the settings still load from
  `/sdcard/CraftApps/Settings/FilmCraft/pen.json`, and the gestures use the
  defaults until one is written by another port).
- **Auto-save stamps** use UTC (the `AutosaveConfig` default offset): the
  recovery file names read a few hours off local time.

## 9. EffectCraft (port 0.1.0, upstream v0.11.1)

Fresh port (2026-10-20), modeled on the FilmCraft port (§8). Upstream cloned
at tag `v0.11.1`; re-pinning follows `scripts/pin-upstream-effectcraft.sh`.

- **Upstream changes** (the only ones, per the pin script): root `Cargo.toml`
  `default-members` excludes the android crate, plus the vendored
  android-activity added to the existing `[patch.crates-io]`;
  `crates/ui-egui/Cargo.toml` eframe accesskit gate (§1 arrangement — the base
  eframe line became two target sections; `sys-locale` stays in its
  `cfg(not(wasm32))` section because `i18n.rs` uses it on every non-wasm
  target, Android included).
- **Port crate** `apps/effectcraft-android/`: `lib.rs` (android_main, storage,
  `EFFECTCRAFT_CONFIG_DIR` → suite settings dir so `effectcraft_host::config_dir`
  resolves, engine logging + session log, GPU failure bridge, hooks),
  `shell.rs` (modifiers, stylus drain, gestures + tap cleanup, pen dispatch;
  deliberately no `saf::poll_bridge()` — the bridge responses belong to the
  picks workers, as in FilmCraft), `picks.rs`, `audio.rs` (the desktop's cpal
  output adapted; default config queried before any stream, per the FilmCraft
  AAudio lesson).
- **File dialogs**: `file_dialog_async` (worker thread + bridge) serves the
  commands' dialogs; the synchronous hooks (panels: Render Queue output,
  Settings paths, shortcut editor) run the same bridge pick on the calling
  thread. Saves resolve the SAF URI to a primary-volume path for the engine's
  `std::fs` writes, with the suite media folder as the logged fallback.
- **Not ported**: the `--control` agent server, desktop window persistence,
  CLI launch arguments, the launch guard; `system_theme` hook unset (egui's
  reported appearance is used). The system font scan finds desktop font
  folders only — the bundled fonts are the working set on the tablet.

## 10. SoundCraft (port 0.1.0, upstream v0.10.0)

Fresh port (2026-10-20), modeled on the FilmCraft (§8) and EffectCraft (§9)
ports. Upstream cloned at tag `v0.10.0`; re-pinning follows
`scripts/pin-upstream-soundcraft.sh`.

- **Upstream changes** (the only ones, per the pin script): root `Cargo.toml`
  `default-members` excludes the android crate, and a `[patch.crates-io]`
  section points android-activity at the vendored copy. NO ui-egui patch:
  `soundcraft-ui-egui` depends on egui only, never eframe, so the §1
  accesskit gate does not apply anywhere in this tree.
- **Port crate** `apps/soundcraft-android/`: `lib.rs` (android_main, storage,
  `SOUNDCRAFT_CONFIG_DIR` → suite settings dir, bridge dir, pen settings,
  GPU cascade, prefs load, JNI entries `nativeOnActivityResult` +
  `nativeOnPenEvent` (all six arguments) + `nativeOnModifierState`),
  `shell.rs` (desktop host duties — prefs saved when the recent list
  changes and at exit, title, quit — plus modifiers, the pen pump +
  stylus drain, gestures, pen dispatch: Undo/Redo → `edit.undo`/`edit.redo`,
  RightClick synthesized; deliberately no `saf::poll_bridge()` — the
  services hooks own the bridge responses, as in FilmCraft/EffectCraft;
  also no gesture tap-cleanup: a tap makes no undoable edit of its own in
  a DAW), `services.rs` (below).
- **File dialogs**: SoundCraft's `Services` hooks are synchronous
  (`pick_open`/`pick_save`/`pick_folder`, called from the UI thread), so
  each launches the bridge picker and waits for the matching response on
  the calling thread (the DesignCraft arrangement): Open answers are the
  bridge inbox paths; Save/Folder resolve the SAF URI to a primary-volume
  path for the engine's `std::fs` I/O — a save that cannot be resolved
  falls back, logged, to `/sdcard/CraftApps/Media/SoundCraft/<name>`; a
  folder that cannot be resolved reads as cancelled. `.scraft` picks are
  extension-filtered after an unfiltered pick (no registered MIME type).
- **Audio**: unlike FilmCraft there is no port-side audio module at all —
  playback and recording are upstream `soundcraft_playback` (cpal 0.16,
  AAudio on Android). The shell constructs the `Player` exactly as the
  desktop host does (`Player::with_config(session, &prefs.audio)`; it
  queries the device itself and falls back to a silent clock, so the
  FilmCraft pre-query lesson does not apply), and recording opens in the
  app on first use (`Recorder::open_with(&ui.audio)`). Deviation from the
  suite permission set: the manifest adds `RECORD_AUDIO`, and
  `SoundCraftActivity` requests it once at start-up (framework
  `requestPermissions`; AAudio input is silent without the grant).
- **Preferences**: `ui.json` load/save is the port's own small
  implementation against the settings dir (the desktop's is in its binary
  crate). Deviation: a partially damaged file falls back to defaults —
  the desktop's docking-field recovery is not replicated.
- **Not ported**: the `--control` agent server, CLI arguments (`--demo`,
  files to open), drag-and-drop drops (Android reports none), the macOS
  menu bar. Audio device choice follows the desktop's named-device prefs;
  cpal on Android typically reports only the default device. Plug-in
  hosting (CLAP/VST3/AU) compiles but has no Android plug-ins to load.
  Stylus settings load from `/sdcard/CraftApps/Settings/SoundCraft/pen.json`
  with no in-app settings screen (as FilmCraft/EffectCraft).

## PdfCraft private storage, external open and compact Android UI
Retain the explicit private storage root, matching Java/Rust private bridge,
ACTION_VIEW import queue and wake hook, mobile workspace integration, reduced
canvas gutters, search demand outside the canvas, responsive preferences and
bounded dialogs/palette when re-pinning upstream. Other ports keep shared
storage and desktop/wide layouts. See docs/fixes-pdfcraft-honor-mobile.md and
docs/fixes-0.2.8-external-open.md for file-by-file notes and verification.
Keep the Android shell's layering-table registration and the explicit WASM
eframe dependency alongside the desktop/Android dependency split.

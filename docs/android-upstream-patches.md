# Android port upstream patches (for pin-upstream.sh scripts)

Each app's `upstream/` checkout carries small Android-specific patches that must
be re-applied when re-pinning to a new upstream tag. The port crate
(`apps/<app>-android/`) is untracked by upstream git and survives checkouts.

The shared crate (`shared/`) and the Gradle project (`<app>/android/`) are
outside the upstream checkout, so they survive a re-pin untouched.

**The 0.4.0 fixes need no new upstream patches** — the gesture detection, the
modifier tracking and the picker bridge all live in the port crate and the
shared crate. See `docs/fixes-0.4.0.md`.

## PhotoCraft (`photocraft/upstream`)
1. `xtask/src/layers.rs` — register `("android", Class::Exempt)` (one line)
2. `crates/ui-egui/Cargo.toml` — gate eframe's `accesskit` feature off on Android
   (compile error with android-native-activity)
3. `crates/ui-egui/src/view_cmds.rs` — `ViewOptions::hides_chrome()`: cfg-gated to
   return `false` on Android (menu bar always visible, incl. DeX windowed mode)
4. `Cargo.toml` (workspace root) — `[patch.crates-io] android-activity = { path =
   "../../vendor/android-activity" }`: the vendored android-activity with the
   CraftApps stylus patch (the S Pen's side button and the tool type).
   `scripts/vendor-android-activity.sh` recreates the vendored copy from the
   cargo registry and applies `patches/android-activity-0.6.1-stylus.patch`;
   `scripts/build-apk.sh` runs it. The vendored copy is a build artefact and
   is **not** committed. See `docs/fixes-0.4.1.md`.

## DesignCraft (`designcraft/upstream`)
1. `xtask/src/layers.rs` — android exempt registration (check exact line)
2. `crates/ui-egui/Cargo.toml` — eframe accesskit gate (check exact dep line)
3. Menu bar: drawn unconditionally — no patch needed

## VectorCraft (`vectorcraft/upstream`)
1. `xtask/src/layers.rs` — android exempt registration (check exact line)
2. `crates/ui-egui/Cargo.toml` — eframe accesskit gate (check exact dep line)
3. Menu bar: drawn unconditionally — no patch needed

## EffectCraft (`effectcraft/upstream`)
1. `xtask/src/layers.rs` — android exempt registration (check exact line)
2. `crates/ui-egui/Cargo.toml` — eframe accesskit gate (check exact dep line)
3. `crates/ui-egui/src/control.rs` — prefs load: `#[cfg(target_os = "android")]`
   forces `app.ui.show_menu_bar = true` after reading prefs (menu always visible)
4. Port crate: `apps/effectcraft-android/src/audio.rs` (new) — cpal/AAudio
   `AudioDevice` impl; wired via `app.hooks.audio_device` in `wire_hooks`

## FilmCraft (`filmcraft/upstream`)
1. `xtask/src/layers.rs` — android exempt registration (check exact line)
2. `crates/ui-egui/Cargo.toml` — eframe accesskit gate (check exact dep line)
3. Menu bar: `show_menu_bar` defaults true, never cleared in code — no patch needed

## LightCraft (`lightcraft/upstream`)
1. `xtask/src/layers.rs` — android exempt registration (check exact line)
2. `crates/ui-egui/Cargo.toml` — eframe accesskit gate (check exact dep line)
3. Menu bar: no fullscreen hiding found — no patch needed

## PdfCraft (`pdfcraft/upstream`)
1. `xtask/src/layers.rs` — android exempt registration (check exact line)
2. `crates/ui-egui/Cargo.toml` — eframe accesskit gate (check exact dep line)
3. Menu bar: no fullscreen hiding found — no patch needed

## Craft Launcher (`launcher/upstream`)
- Upstream is storytold/craft-launcher (not a storytold app repo); port reuses
  its core/ui-egui crates. No layers.rs or eframe patches currently needed.
- Port crate wires the shared GPU cascade in `lib.rs` (no upstream change).

## PdfCraft private storage, external open and compact Android UI
Retain the explicit private storage root, matching Java/Rust private bridge,
ACTION_VIEW import queue and wake hook, mobile workspace integration, reduced
canvas gutters, search demand outside the canvas, responsive preferences and
bounded dialogs/palette when re-pinning upstream. Other ports keep shared
storage and desktop/wide layouts. See docs/fixes-pdfcraft-honor-mobile.md and
docs/fixes-0.2.8-external-open.md for file-by-file notes and verification.
Keep the Android shell's layering-table registration and the explicit WASM
eframe dependency alongside the desktop/Android dependency split.

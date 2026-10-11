# Pen/Stylus Support — Porting Guide

The shared pen module (`craftapps-shared::pen`) gives every app pen settings,
pressure processing, and pen-button remapping. PhotoCraft is the reference
implementation. This guide covers adopting it in the other ports.

## What the user gets

- **Preferences › Stylus**: pressure on/off, response curve, sensitivity,
  smoothing, pen/touch input mode, palm rejection toggle, and per-button
  remapping with "press pen button to assign" capture.
- **Pen buttons**: barrel button(s) and eraser end trigger remapped actions
  (undo, redo, eraser toggle, eyedropper, pan-while-held, right-click).

## Why a Java side-channel exists

winit 0.30's Android backend turns every `MotionEvent` into a generic `Touch`
with pressure — it **drops** the tool type (stylus/finger/eraser/palm) and the
barrel-button state (`ACTION_BUTTON_PRESS/RELEASE` maps to "unhandled"). The
NDK exposes all of it, but winit owns the input queue exclusively.

Each port's `<App>Activity.java` overrides `dispatchTouchEvent` (which still
fires for `NativeActivity` subclasses) and forwards *only pen-relevant state
changes* to Rust via `nativeOnPenEvent`. See PhotoCraft's
`PhotoCraftActivity.java` — copy the `dispatchTouchEvent` override and the
native declaration, renaming the class.

## Checklist per app

1. **Cargo.toml**: add `craftapps-shared = { path = "../../../../shared" }`
   (same relative depth as the other ports).

2. **`<App>Activity.java`**: copy PhotoCraft's `dispatchTouchEvent` override +
   `private native void nativeOnPenEvent(int, int, int, int);`.

3. **`src/pen.rs`** (new, in the port): the mangled JNI entry forwarding to
   `craftapps_shared::pen::handle_pen_event`. Copy PhotoCraft's, renaming
   `Java_app_photocraft_android_PhotoCraftActivity_` to your activity's
   package path (dots → underscores).

4. **`src/lib.rs`**:
   - `mod pen;`
   - Load settings: `PenSettings::load(&config)`, `pen::sync_settings(&s)`.
   - Add the `pen` field to `AndroidHooks` (see PhotoCraft's
     `wire_android_hooks`): a `pen_section` closure drawing
     `pen::pen_settings_ui`.

5. **Upstream `ui-egui`** (port-modified, like the Storage section):
   - `lib.rs`: add `AndroidPenHooks` to `AndroidHooks` (copy PhotoCraft's).
   - `prefs_ui.rs`: call `(hooks.pen.pen_section)(ui)` in the stylus section.

6. **Action dispatch**: once per frame (e.g. in the shell's `ui()`), drain
   `pen::drain_actions()` and map `PenAction` → your commands. Copy
   PhotoCraft's `dispatch_pen_actions` and remap the ids.

7. **Pressure pipeline** (optional): `PenSettings::adjust_pressure(raw)` applies
   curve + sensitivity; `PressureSmoother` does the moving average. Wire into
   your stroke handling where you read touch force.

## JNI protocol (Java → Rust)

`nativeOnPenEvent(action, toolType, buttonState, actionButton)`:
- `action`: `MotionEvent.getActionMasked()`
- `toolType`: `getToolType()` (1=finger, 2=stylus, 4=eraser, 5=palm)
- `buttonState`: `getButtonState()` bits
- `actionButton`: `getActionButton()` (for press/release)

Java throttles to state changes; Rust debounces and remaps. All shared state
is thread-safe (UI thread writes, app thread reads).

## Testing without hardware

- `cargo test -p craftapps-shared` runs the pressure/smoothing/settings unit tests.
- The button state machine (`on_pen_event`) is pure logic — feed it synthetic
  (action, tool, buttons) tuples in a test to verify transitions.
- On-device: Preferences › Stylus › Pen test shows live pressure; the Assign
  flow needs a real S Pen (barrel button).

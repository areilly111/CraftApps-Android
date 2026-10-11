//! Pen/stylus support for Android: settings, pressure processing, and pen
//! button remapping.
//!
//! ## What winit 0.30 actually delivers on Android (researched 2026-10-11)
//!
//! winit's Android backend (`platform_impl/android/mod.rs`, `handle_input_event`)
//! turns every `MotionEvent` into a generic [`winit::event::WindowEvent::Touch`]
//! with `force = Some(Force::Normalized(pressure))`. That is ALL it forwards:
//!
//! | Android data | winit 0.30 | reaches the app? |
//! |---|---|---|
//! | `pressure` | `Touch.force` | yes (via egui `Event::Touch { force }`) |
//! | `device_id` | `Touch.device_id` | yes (egui `TouchDeviceId`, hashed) |
//! | `tool_type` (Stylus/Finger/Eraser/Palm) | — | **NO, dropped** |
//! | `button_state` (barrel button) | — | **NO, dropped** |
//! | `ACTION_BUTTON_PRESS/RELEASE` | mapped to `None` ("TODO mouse events") | **NO, dropped** |
//! | hover (`ACTION_HOVER_MOVE`) | mapped to `None` | **NO, dropped** |
//! | tilt / orientation | — | **NO** |
//!
//! The NDK *does* expose all of it (`android-activity 0.6.1`:
//! `Pointer::tool_type()`, `MotionEvent::button_state()`,
//! `ButtonState::stylus_primary()`), but winit consumes the input queue
//! exclusively, so the app cannot read it through winit.
//!
//! ## How this module gets the missing data
//!
//! Each port's `<App>Activity.java` overrides `dispatchTouchEvent` — a Java
//! entry point that still fires for `NativeActivity` subclasses — and forwards
//! only pen-relevant state changes (tool type, barrel-button transitions) to
//! Rust via the JNI entry [`native_on_pen_event`] (same established pattern as
//! `nativeOnActivityResult`). Java throttles to state changes so JNI traffic is
//! a handful of calls per stroke, not per motion event.
//!
//! ## What works without the Java hook
//!
//! Pressure (`force`), pressure curve/sensitivity/smoothing, and the pen-test
//! area work with winit alone. Pen-vs-touch separation uses a hybrid: the Java
//! side-channel bootstraps stylus detection, then the learned
//! `device_id` (the S Pen is a separate input device from the touchscreen on
//! Samsung tablets) becomes the primary signal.
//!
//! ## Threading
//!
//! The JNI entry runs on Android's UI thread; the app reads state on the
//! winit/egui thread. All shared state is behind `Mutex`/`Atomic*`. Remapped
//! actions are pushed to a queue drained once per frame by the app.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Types

/// Which physical tool is touching, from the Java side-channel
/// (`MotionEvent.getToolType`). winit never reports this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PenTool {
    Unknown,
    Finger,
    Stylus,
    Eraser,
    Palm,
}

/// The remappable pen inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PenButton {
    /// S Pen barrel button (`BUTTON_STYLUS_PRIMARY`).
    BarrelPrimary,
    /// Second barrel button, if the pen has one (`BUTTON_STYLUS_SECONDARY`).
    BarrelSecondary,
    /// Inverted pen / eraser end (`TOOL_TYPE_ERASER`).
    Eraser,
}

impl PenButton {
    pub fn label(self) -> &'static str {
        match self {
            PenButton::BarrelPrimary => "Pen button",
            PenButton::BarrelSecondary => "Pen button 2",
            PenButton::Eraser => "Eraser end",
        }
    }

    pub fn all() -> [PenButton; 3] {
        [PenButton::BarrelPrimary, PenButton::BarrelSecondary, PenButton::Eraser]
    }
}

/// App-agnostic actions a pen button can trigger. Each port maps these to its
/// own commands (see PhotoCraft's `shell.rs` for the reference mapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PenAction {
    /// Do nothing.
    None,
    Undo,
    Redo,
    /// Toggle between brush and eraser.
    ToggleEraser,
    /// Eyedropper / color picker.
    Eyedropper,
    /// Pan / hand tool while held (fires press + release).
    PanTool,
    /// Right-click at the pen tip.
    RightClick,
}

impl PenAction {
    pub fn label(self) -> &'static str {
        match self {
            PenAction::None => "Do nothing",
            PenAction::Undo => "Undo",
            PenAction::Redo => "Redo",
            PenAction::ToggleEraser => "Toggle eraser",
            PenAction::Eyedropper => "Eyedropper",
            PenAction::PanTool => "Pan tool (while held)",
            PenAction::RightClick => "Right-click",
        }
    }

    pub fn all() -> [PenAction; 7] {
        [
            PenAction::None,
            PenAction::Undo,
            PenAction::Redo,
            PenAction::ToggleEraser,
            PenAction::Eyedropper,
            PenAction::PanTool,
            PenAction::RightClick,
        ]
    }

    /// Momentary actions (like [`PenAction::PanTool`]) need a release event.
    pub fn is_momentary(self) -> bool {
        matches!(self, PenAction::PanTool)
    }
}

/// How pen and finger input are treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputMode {
    /// Default: pen draws, finger does gestures (pan/zoom).
    PenDrawsTouchGestures,
    /// Only the pen draws; finger input never paints.
    PenOnly,
    /// Finger paints too (for finger-painting).
    TouchDrawsToo,
}

impl InputMode {
    pub fn label(self) -> &'static str {
        match self {
            InputMode::PenDrawsTouchGestures => "Pen draws, touch does gestures",
            InputMode::PenOnly => "Pen only",
            InputMode::TouchDrawsToo => "Touch draws too",
        }
    }

    pub fn all() -> [InputMode; 3] {
        [InputMode::PenDrawsTouchGestures, InputMode::PenOnly, InputMode::TouchDrawsToo]
    }
}

/// Pressure response curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressureCurve {
    Linear,
    /// Reaches full strength with a light touch.
    Soft,
    /// Needs a hard press for full strength.
    Firm,
}

impl PressureCurve {
    pub fn label(self) -> &'static str {
        match self {
            PressureCurve::Linear => "Linear",
            PressureCurve::Soft => "Soft",
            PressureCurve::Firm => "Firm",
        }
    }

    pub fn all() -> [PressureCurve; 3] {
        [PressureCurve::Linear, PressureCurve::Soft, PressureCurve::Firm]
    }

    /// Map raw 0..1 pressure through the curve.
    pub fn apply(self, p: f32) -> f32 {
        let p = p.clamp(0.0, 1.0);
        match self {
            PressureCurve::Linear => p,
            // gamma < 1 boosts low end
            PressureCurve::Soft => p.powf(0.65),
            // gamma > 1 compresses low end
            PressureCurve::Firm => p.powf(1.6),
        }
    }
}

// ---------------------------------------------------------------------------
// Settings

/// Pen settings, persisted per-app as `pen.json` under the app's settings dir
/// (`/sdcard/CraftApps/Settings/<App>/`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PenSettings {
    pub pressure_enabled: bool,
    pub pressure_curve: PressureCurve,
    /// 0.25..=3.0 multiplier on top of the curve.
    pub sensitivity: f32,
    /// Moving-average window over recent pressure samples, 0 = off.
    pub smoothing: u8,
    pub input_mode: InputMode,
    /// Best-effort: ignore touches that start while a palm is down. Needs the
    /// Java side-channel (winit drops `TOOL_TYPE_PALM`).
    pub palm_rejection: bool,
    pub button_primary: PenAction,
    pub button_secondary: PenAction,
    pub eraser_action: PenAction,
}

impl Default for PenSettings {
    fn default() -> Self {
        PenSettings {
            pressure_enabled: true,
            pressure_curve: PressureCurve::Linear,
            sensitivity: 1.0,
            smoothing: 0,
            input_mode: InputMode::PenDrawsTouchGestures,
            palm_rejection: true,
            button_primary: PenAction::RightClick,
            button_secondary: PenAction::None,
            eraser_action: PenAction::ToggleEraser,
        }
    }
}

impl PenSettings {
    fn path(settings_dir: &Path) -> std::path::PathBuf {
        settings_dir.join("pen.json")
    }

    /// Load, falling back to defaults on any error (never fails the app).
    pub fn load(settings_dir: &Path) -> Self {
        let path = Self::path(settings_dir);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return PenSettings::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Save; errors are returned for the caller to log.
    pub fn save(&self, settings_dir: &Path) -> Result<(), String> {
        let path = Self::path(settings_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("pen settings dir: {e}"))?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| format!("pen settings encode: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("pen settings write: {e}"))?;
        Ok(())
    }

    pub fn action_for(&self, button: PenButton) -> PenAction {
        match button {
            PenButton::BarrelPrimary => self.button_primary,
            PenButton::BarrelSecondary => self.button_secondary,
            PenButton::Eraser => self.eraser_action,
        }
    }

    /// Full pressure pipeline: on/off → curve → sensitivity clamp.
    /// Smoothing is stateful; see [`PressureSmoother`].
    pub fn adjust_pressure(&self, raw: f32) -> f32 {
        if !self.pressure_enabled {
            return 1.0;
        }
        (self.pressure_curve.apply(raw) * self.sensitivity.clamp(0.25, 3.0)).clamp(0.0, 1.0)
    }
}

/// Stateful moving-average smoother for pressure. Feed raw 0..1 samples per
/// motion event; read the smoothed value for the stroke.
#[derive(Clone, Debug, Default)]
pub struct PressureSmoother {
    window: VecDeque<f32>,
}

impl PressureSmoother {
    pub fn new() -> Self {
        PressureSmoother { window: VecDeque::new() }
    }

    /// Push a sample; returns the smoothed value. `smoothing` is the window
    /// size from [`PenSettings::smoothing`]; 0 disables smoothing.
    pub fn push(&mut self, sample: f32, smoothing: u8) -> f32 {
        if smoothing == 0 {
            self.window.clear();
            return sample;
        }
        let n = smoothing.clamp(1, 16) as usize;
        self.window.push_back(sample);
        while self.window.len() > n {
            self.window.pop_front();
        }
        let sum: f32 = self.window.iter().sum();
        sum / self.window.len() as f32
    }

    pub fn reset(&mut self) {
        self.window.clear();
    }
}

// ---------------------------------------------------------------------------
// Runtime state (UI thread JNI writes, egui thread reads)

/// A pen-button press/release delivered from Java, queued for the app thread.
#[derive(Clone, Copy, Debug)]
pub struct PenButtonEvent {
    pub button: PenButton,
    pub pressed: bool,
}

struct PenRuntime {
    /// Last tool reported by the Java side-channel.
    tool: PenTool,
    /// Currently pressed buttons (from Java transitions).
    pressed: [bool; 3],
    /// Palm currently down (Java side-channel).
    palm_down: bool,
    /// Stylus currently touching (Java side-channel).
    stylus_down: bool,
    /// Learned winit device id hash for the pen (S Pen is a separate input
    /// device on Samsung tablets). `None` until observed.
    pen_device: Option<u64>,
    /// Remap capture in progress: which button's row is being assigned.
    capture: Option<PenButton>,
    /// Completed capture: (row being assigned, physical button pressed).
    captured: Option<(PenButton, PenButton)>,
    /// Queued button events for the app thread to drain.
    queue: VecDeque<PenButtonEvent>,
    /// Queued actions for the app thread to drain (already remapped).
    actions: VecDeque<QueuedAction>,
}

/// A remapped action waiting for the app thread, with press/release for
/// momentary actions.
#[derive(Clone, Copy, Debug)]
pub struct QueuedAction {
    pub action: PenAction,
    pub pressed: bool,
}

static RUNTIME: OnceLock<Mutex<PenRuntime>> = OnceLock::new();

fn runtime() -> &'static Mutex<PenRuntime> {
    RUNTIME.get_or_init(|| {
        Mutex::new(PenRuntime {
            tool: PenTool::Unknown,
            pressed: [false; 3],
            palm_down: false,
            stylus_down: false,
            pen_device: None,
            capture: None,
            captured: None,
            queue: VecDeque::new(),
            actions: VecDeque::new(),
        })
    })
}

/// Settings snapshot shared with the JNI thread for remap lookup. The app
/// calls [`sync_settings`] whenever settings change (and once at startup).
static SETTINGS_SNAP: OnceLock<Mutex<PenSettings>> = OnceLock::new();

fn settings_snap() -> &'static Mutex<PenSettings> {
    SETTINGS_SNAP.get_or_init(|| Mutex::new(PenSettings::default()))
}

/// Publish current settings to the runtime (call after load and on change).
pub fn sync_settings(settings: &PenSettings) {
    if let Ok(mut snap) = settings_snap().lock() {
        *snap = settings.clone();
    }
}

fn button_index(b: PenButton) -> usize {
    match b {
        PenButton::BarrelPrimary => 0,
        PenButton::BarrelSecondary => 1,
        PenButton::Eraser => 2,
    }
}

/// Called by the JNI entry when Java reports a pen state change.
/// `action`: MotionEvent action masked; `tool`: `getToolType`;
/// `buttons`: `getButtonState` bits; `action_button`: `getActionButton`.
pub fn on_pen_event(action: i32, tool: i32, buttons: i32, action_button: i32) {
    // Android constants (stable API literals, mirroring MotionEvent).
    const ACTION_BUTTON_PRESS: i32 = 11;
    const ACTION_BUTTON_RELEASE: i32 = 12;
    const ACTION_DOWN: i32 = 0;
    const ACTION_UP: i32 = 1;
    const ACTION_CANCEL: i32 = 3;
    const TOOL_STYLUS: i32 = 2;
    const TOOL_ERASER: i32 = 4;
    const TOOL_PALM: i32 = 5;
    const BUTTON_STYLUS_PRIMARY: i32 = 1 << 5;
    const BUTTON_STYLUS_SECONDARY: i32 = 1 << 6;

    let Ok(mut rt) = runtime().lock() else { return };

    // Tool tracking.
    rt.tool = match tool {
        TOOL_STYLUS => PenTool::Stylus,
        TOOL_ERASER => PenTool::Eraser,
        TOOL_PALM => PenTool::Palm,
        1 => PenTool::Finger,
        _ => PenTool::Unknown,
    };
    rt.palm_down = tool == TOOL_PALM && (action == ACTION_DOWN || action == 2 /* MOVE */);
    if action == ACTION_UP || action == ACTION_CANCEL {
        rt.palm_down = false;
        if tool == TOOL_STYLUS || tool == TOOL_ERASER {
            rt.stylus_down = false;
        }
    }
    if action == ACTION_DOWN && (tool == TOOL_STYLUS || tool == TOOL_ERASER) {
        rt.stylus_down = true;
    }

    // Eraser end: tool-type transition acts like a button.
    let eraser_now = tool == TOOL_ERASER && (action == ACTION_DOWN || action == 2);
    set_button(&mut rt, PenButton::Eraser, eraser_now);

    // Barrel buttons: transitions via ACTION_BUTTON_PRESS/RELEASE, or the
    // held state on DOWN/MOVE.
    let primary_held = buttons & BUTTON_STYLUS_PRIMARY != 0;
    let secondary_held = buttons & BUTTON_STYLUS_SECONDARY != 0;
    if action == ACTION_BUTTON_PRESS {
        if action_button == BUTTON_STYLUS_PRIMARY {
            set_button(&mut rt, PenButton::BarrelPrimary, true);
        } else if action_button == BUTTON_STYLUS_SECONDARY {
            set_button(&mut rt, PenButton::BarrelSecondary, true);
        }
    } else if action == ACTION_BUTTON_RELEASE {
        if action_button == BUTTON_STYLUS_PRIMARY {
            set_button(&mut rt, PenButton::BarrelPrimary, false);
        } else if action_button == BUTTON_STYLUS_SECONDARY {
            set_button(&mut rt, PenButton::BarrelSecondary, false);
        }
    } else {
        set_button(&mut rt, PenButton::BarrelPrimary, primary_held);
        set_button(&mut rt, PenButton::BarrelSecondary, secondary_held);
    }

    // Lift/cancel clears everything: a pen that leaves the screen cannot be
    // holding a button. Prevents stuck "pressed" states.
    if action == ACTION_UP || action == ACTION_CANCEL {
        set_button(&mut rt, PenButton::BarrelPrimary, false);
        set_button(&mut rt, PenButton::BarrelSecondary, false);
        set_button(&mut rt, PenButton::Eraser, false);
    }
}

/// Record a button transition; on change, queue the event and the remapped
/// action (or complete a capture).
fn set_button(rt: &mut PenRuntime, button: PenButton, pressed: bool) {
    let i = button_index(button);
    if rt.pressed[i] == pressed {
        return;
    }
    rt.pressed[i] = pressed;

    // Capture mode: the user pressed a pen button to assign it. Record the
    // (row, physical button) pair; the UI reassigns the row to the pressed
    // button. The press itself is swallowed, not dispatched.
    if pressed {
        if let Some(target) = rt.capture.take() {
            rt.captured = Some((target, button));
            return;
        }
    }

    rt.queue.push_back(PenButtonEvent { button, pressed });

    // Remap to the configured action.
    let action = settings_snap()
        .lock()
        .map(|s| s.action_for(button))
        .unwrap_or(PenAction::None);
    if action == PenAction::None {
        return;
    }
    if !action.is_momentary() && !pressed {
        return; // fire one-shot actions on press only
    }
    rt.actions.push_back(QueuedAction { action, pressed });
}

// ---------------------------------------------------------------------------
// App-thread API

/// Drain queued pen-button events (raw). Usually you want [`drain_actions`].
pub fn drain_button_events() -> Vec<PenButtonEvent> {
    runtime()
        .lock()
        .map(|mut rt| rt.queue.drain(..).collect())
        .unwrap_or_default()
}

/// Drain remapped actions. Call once per frame; dispatch each to the app's
/// command system. Returns `(action, pressed)`; one-shot actions only appear
/// with `pressed == true`.
pub fn drain_actions() -> Vec<QueuedAction> {
    runtime()
        .lock()
        .map(|mut rt| rt.actions.drain(..).collect())
        .unwrap_or_default()
}

/// Is the stylus currently touching? (Java side-channel.)
pub fn stylus_down() -> bool {
    runtime().lock().map(|rt| rt.stylus_down).unwrap_or(false)
}

/// Is a palm currently down? (Java side-channel; winit drops this.)
pub fn palm_down() -> bool {
    runtime().lock().map(|rt| rt.palm_down).unwrap_or(false)
}

/// Last tool type reported by the Java side-channel.
pub fn current_tool() -> PenTool {
    runtime().lock().map(|rt| rt.tool).unwrap_or(PenTool::Unknown)
}

/// Learn the pen's winit device id. Call from the app's touch handling: when
/// [`stylus_down`] is true and a `Touch{Started}` arrives, its `device_id`
/// hash is the pen. Returns true if this device is (now) known as the pen.
pub fn note_touch_device(device_hash: u64, is_start: bool) -> bool {
    let Ok(mut rt) = runtime().lock() else { return false };
    if is_start && rt.stylus_down {
        rt.pen_device = Some(device_hash);
    }
    rt.pen_device == Some(device_hash)
}

/// Forget the learned pen device (e.g. on settings reset).
pub fn forget_pen_device() {
    if let Ok(mut rt) = runtime().lock() {
        rt.pen_device = None;
    }
}

/// Begin "press pen button to assign" capture for `button`'s settings row.
/// The next physical pen-button press completes it; see [`take_capture`].
pub fn begin_capture(button: PenButton) {
    if let Ok(mut rt) = runtime().lock() {
        rt.capture = Some(button);
    }
}

/// Cancel an in-progress capture.
pub fn cancel_capture() {
    if let Ok(mut rt) = runtime().lock() {
        rt.capture = None;
    }
}

/// Which button's row is capturing, if any.
pub fn capturing() -> Option<PenButton> {
    runtime().lock().ok().and_then(|rt| rt.capture)
}

/// Take a completed capture: (row being assigned, physical button pressed).
/// The UI should move the row's action to the pressed button.
pub fn take_captured() -> Option<(PenButton, PenButton)> {
    runtime().lock().ok().and_then(|mut rt| rt.captured.take())
}

// ---------------------------------------------------------------------------
// JNI entry: <App>Activity.nativeOnPenEvent (mangled name per app; the shared
// crate exposes this generic handler and each port re-exports it under its
// own mangled symbol — see PhotoCraft's saf.rs pattern).

/// Handle a pen state change from Java. See module docs for the protocol.
pub fn handle_pen_event(action: i32, tool: i32, buttons: i32, action_button: i32) {
    // Never unwind into the JVM.
    let _ = std::panic::catch_unwind(|| on_pen_event(action, tool, buttons, action_button));
}

// ---------------------------------------------------------------------------
// egui settings UI

/// Draw the pen settings section. `save` persists (called on change);
/// `action_rows` lists the remappable buttons. App-agnostic: the port passes
/// its settings dir and a save closure.
pub fn pen_settings_ui(
    ui: &mut egui::Ui,
    settings: &mut PenSettings,
    on_change: &mut dyn FnMut(),
) {
    let mut changed = false;

    ui.label("Pressure");
    let mut enabled = settings.pressure_enabled;
    if ui.checkbox(&mut enabled, "Use pen pressure").changed() {
        settings.pressure_enabled = enabled;
        changed = true;
    }
    if !settings.pressure_enabled {
        ui.label("Off: the pen paints like a mouse, at full pressure.");
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Response curve");
        egui::ComboBox::from_id_salt("pen-curve")
            .selected_text(settings.pressure_curve.label())
            .show_ui(ui, |ui| {
                for c in PressureCurve::all() {
                    if ui.selectable_value(&mut settings.pressure_curve, c, c.label()).changed() {
                        changed = true;
                    }
                }
            });
    });
    ui.label("Soft reaches full strength with a light touch; Firm needs a hard press.");

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Sensitivity");
        let mut s = settings.sensitivity;
        if ui.add(egui::Slider::new(&mut s, 0.25..=3.0).step_by(0.05)).changed() {
            settings.sensitivity = s;
            changed = true;
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Smoothing");
        let mut s = settings.smoothing;
        if ui.add(egui::Slider::new(&mut s, 0..=10)).changed() {
            settings.smoothing = s;
            changed = true;
        }
    });
    ui.label("Averages recent pressure samples; higher is steadier but laggier. 0 = off.");

    ui.add_space(10.0);
    ui.label("Input");
    ui.horizontal(|ui| {
        ui.label("Pen / touch mode");
        egui::ComboBox::from_id_salt("pen-input-mode")
            .selected_text(settings.input_mode.label())
            .show_ui(ui, |ui| {
                for m in InputMode::all() {
                    if ui.selectable_value(&mut settings.input_mode, m, m.label()).changed() {
                        changed = true;
                    }
                }
            });
    });

    ui.add_space(4.0);
    let mut palm = settings.palm_rejection;
    if ui.checkbox(&mut palm, "Palm rejection").changed() {
        settings.palm_rejection = palm;
        changed = true;
    }
    ui.label("Ignores touches that start while your palm is on the screen. Needs the pen data channel (works on this build).");

    ui.add_space(10.0);
    ui.label("Pen buttons");
    ui.label("Tap “Assign…”, then press the pen button (or flip to the eraser end).");
    for button in PenButton::all() {
        ui.horizontal(|ui| {
            ui.label(button.label());
            let current = settings.action_for(button);
            egui::ComboBox::from_id_salt(format!("pen-btn-{:?}", button))
                .selected_text(current.label())
                .show_ui(ui, |ui| {
                    for a in PenAction::all() {
                        let mut sel = current;
                        if ui.selectable_value(&mut sel, a, a.label()).changed() {
                            match button {
                                PenButton::BarrelPrimary => settings.button_primary = sel,
                                PenButton::BarrelSecondary => settings.button_secondary = sel,
                                PenButton::Eraser => settings.eraser_action = sel,
                            }
                            changed = true;
                        }
                    }
                });
            let cap = capturing() == Some(button);
            let btn_label = if cap { "Press pen button… (tap to cancel)" } else { "Assign…" };
            if ui.button(btn_label).clicked() {
                if cap {
                    cancel_capture();
                } else {
                    begin_capture(button);
                }
                ui.ctx().request_repaint();
            }
        });
    }
    // Capture completion: the pressed button takes over the capturing row —
    // swap the two rows' actions so nothing is lost. Polled every frame while
    // the section is open (capture may complete on the UI thread at any time).
    if let Some((target, pressed)) = take_captured() {
        swap_button_actions(settings, target, pressed);
        changed = true;
    }
    if capturing().is_some() {
        ui.ctx().request_repaint();
    }

    if changed {
        sync_settings(settings);
        on_change();
    }
}

fn swap_button_actions(settings: &mut PenSettings, a: PenButton, b: PenButton) {
    if a == b {
        return;
    }
    let get = |s: &PenSettings, btn: PenButton| s.action_for(btn);
    let (aa, ab) = (get(settings, a), get(settings, b));
    let set = |s: &mut PenSettings, btn: PenButton, act: PenAction| match btn {
        PenButton::BarrelPrimary => s.button_primary = act,
        PenButton::BarrelSecondary => s.button_secondary = act,
        PenButton::Eraser => s.eraser_action = act,
    };
    set(settings, a, ab);
    set(settings, b, aa);
}

/// Which [`PenAction`]s the app actually supports, for greying out rows.
/// (Kept simple: all actions are offered; the port's dispatch maps them.)
pub fn supported_actions() -> [PenAction; 7] {
    PenAction::all()
}

// ---------------------------------------------------------------------------
// Tests (host)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_curve_endpoints() {
        assert_eq!(PressureCurve::Linear.apply(0.0), 0.0);
        assert_eq!(PressureCurve::Linear.apply(1.0), 1.0);
        assert!(PressureCurve::Soft.apply(0.25) > 0.25);
        assert!(PressureCurve::Firm.apply(0.25) < 0.25);
    }

    #[test]
    fn sensitivity_clamped() {
        let mut s = PenSettings::default();
        s.sensitivity = 99.0;
        assert_eq!(s.adjust_pressure(0.5), 1.0);
        s.sensitivity = -1.0;
        assert!(s.adjust_pressure(0.5) >= 0.0);
    }

    #[test]
    fn pressure_off_is_full() {
        let mut s = PenSettings::default();
        s.pressure_enabled = false;
        assert_eq!(s.adjust_pressure(0.1), 1.0);
    }

    #[test]
    fn smoother_averages() {
        let mut sm = PressureSmoother::new();
        assert_eq!(sm.push(1.0, 0), 1.0); // off
        sm.push(0.0, 2);
        let v = sm.push(1.0, 2);
        assert!((v - 0.5).abs() < 1e-6);
    }

    #[test]
    fn settings_round_trip() {
        let s = PenSettings::default();
        let json = serde_json::to_string(&s).expect("encode");
        let back: PenSettings = serde_json::from_str(&json).expect("decode");
        assert_eq!(back.sensitivity, s.sensitivity);
        assert_eq!(back.button_primary, s.button_primary);
    }
}

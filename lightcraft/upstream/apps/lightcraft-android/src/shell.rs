//! eframe bootstrap for LightCraft on Android.
//!
//! The android-activity 0.6 `native-activity` entry point, GPU cascade setup,
//! suite storage init, and the two JNI callbacks the Activity calls into:
//! `nativeOnActivityResult` (SAF picker answers) and `nativeOnPenEvent`
//! (stylus side-channel).

use crate::{VERSION_NAME, services};
use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen, saf, storage};
use lightcraft_ui_egui::LightcraftApp;
use std::path::PathBuf;
use std::sync::Arc;

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("LightCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("LightCraft Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    jni_bridge::init()?;

    let cache_dir = PathBuf::from(jni_bridge::cache_dir().map_err(|e| format!("cache dir: {e}"))?);

    // The file picker bridge lives in the app's own external files dir: always
    // writable, and no "All files access" permission needed. The Java side
    // reads the same path (`getExternalFilesDir(null)/.picker`).
    let bridge_base = app.external_data_path().unwrap_or_else(|| cache_dir.clone());
    let bridge_dir = bridge_base.join(".picker");
    let _ = std::fs::create_dir_all(&bridge_dir);
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("CRAFTAPPS_BRIDGE_DIR", &bridge_dir) };

    // Suite storage: /sdcard/CraftApps/Media, /sdcard/CraftApps/Settings/LightCraft.
    let storage = Arc::new(
        storage::StorageManager::new(
            &cache_dir,
            &storage::StorageAppInfo { app_name: "LightCraft", settings_folder: "LightCraft" },
        )
        .map_err(|e| format!("storage: {e}"))?,
    );
    storage.ensure_dirs().map_err(|e| format!("storage dirs: {e}"))?;

    // Create the shared media/export folders up front so exports never fail
    // on a missing directory.
    if let Ok(root) = jni_bridge::external_storage_root() {
        let exports = PathBuf::from(root).join("CraftApps/Media/LightCraft/Exports");
        if let Err(e) = std::fs::create_dir_all(&exports) {
            log::warn!("couldn't create {}: {e}", exports.display());
        }
    }

    if !jni_bridge::is_external_storage_manager().unwrap_or(false) {
        // Library folders are used as raw filesystem paths (the engine
        // opens/creates the catalog in place), which needs All-files
        // access; a SAF tree grant alone doesn't make the path writable.
        // Ask for it, like DesignCraft does.
        log::warn!("all-files access not granted; opening settings to request it");
        let _ = jni_bridge::open_all_files_settings();
    }

    // Stylus settings live in the suite settings dir.
    let pen_settings = pen::PenSettings::load(storage.settings_dir());

    // Shared GPU cascade (Vulkan → GLES → CPU).
    let gpu_plan = gpu::plan(
        &storage.settings_dir().join("gpu.json"),
        &gpu::GpuAppInfo { app_name: "LightCraft", cpu_canvas_env: "LIGHTCRAFT_CPU_CANVAS" },
    );
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    let services = services::android_services(storage.clone());
    let session = lightcraft_engine::Session::new()
        .with_fs()
        .with_default_denoise_models()
        .with_system_clock();

    let _ = pen_settings;
    eframe::run_native(
        "LightCraft",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(App {
                inner: LightcraftApp::new(session, services),
                gestures: craftapps_shared::gestures::GestureDetector::new(),
                modifiers: craftapps_shared::modifiers::ModifierKeys::default(),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

/// Thin `eframe::App` wrapper around [`LightcraftApp`]: the UI crate owns the
/// app logic (`logic` / `ui` / `raw_input_hook`); this shell only adapts it to
/// eframe's trait.
struct App {
    inner: LightcraftApp,
    /// Multi-finger tap recognition (the "stylus mode" gestures).
    gestures: craftapps_shared::gestures::GestureDetector,
    /// Which modifier keys are held (winit's Android backend sends no
    /// `ModifiersChanged`, so they are tracked from the key events).
    modifiers: craftapps_shared::modifiers::ModifierKeys,
}

impl App {
    /// Multi-finger taps (the "stylus mode" gestures) and the keyboard
    /// modifiers. Both run before the app's frame, so the app sees the
    /// modifiers and the gesture's side effects in the same frame.
    fn process_input(&mut self, ctx: &egui::Context) {
        // Pen input pump: wakes the app on pen-button transitions so a
        // hover press dispatches immediately instead of waiting for the
        // next touch to run a frame (the PhotoCraft 0.4.5 fix).
        craftapps_shared::pen::install_pen_pump(ctx.clone());
        // Keyboard modifiers: winit's Android backend never sends
        // `ModifiersChanged`, so egui's modifier state is empty and every
        // Ctrl/Shift/Alt shortcut is dead without this.
        self.modifiers.update(ctx);
        self.modifiers.publish(ctx);

        // The file bridge's answer to a system picker, if one came back.
        saf::poll_bridge();

        // The stylus data of the last motion event: the tool type (stylus,
        // eraser end, palm) and the side-button state. winit drops both, so
        // the vendored android-activity records them on the way past.
        self.feed_stylus_state();

        if let Some(tap) = self.gestures.update(ctx) {
            // The shared runtime maps the finger count to the configured
            // action (and ignores the gesture when stylus mode is off).
            let ppp = ctx.pixels_per_point();
            pen::on_gesture(tap.fingers as i32, tap.pos.x * ppp, tap.pos.y * ppp);
        }
    }

    /// Hand the stylus data of the motion events to the shared pen runtime:
    /// the tool type (stylus / eraser end / palm) and the side button (the
    /// S Pen's barrel button).
    ///
    /// The runtime keeps the pressed state and only acts on the transitions,
    /// so the same state twice is harmless.
    fn feed_stylus_state(&mut self) {
        // Android tool types.
        const TOOL_FINGER: i32 = 1;
        const TOOL_MOUSE: i32 = 3;
        // The stylus' side buttons.
        const STYLUS_BUTTONS: u32 = (1 << 5) | (1 << 6);

        // Stylus states via the shared pen pump when it is running (it
        // notes them for the diagnostics itself); otherwise drain the
        // Android queue directly, as before.
        let states = if craftapps_shared::pen::pen_pump_active() {
            craftapps_shared::pen::take_queued_states()
        } else {
            let raw = android_activity::input::drain_stylus_states();
            for state in &raw {
                craftapps_shared::pen::note_queue_state(
                    state.action,
                    state.buttons,
                    state.action_button as i32,
                    state.tool_type,
                );
            }
            raw
        };
        for state in states {
            // A finger's touch is not the pen's business: it must not clear
            // the pen's state (the pen may well still be down).
            if (state.tool_type == TOOL_FINGER || state.tool_type == TOOL_MOUSE)
                && state.buttons & STYLUS_BUTTONS == 0
            {
                continue;
            }
            // The pen runtime speaks the Java protocol: the Android action
            // codes (0 down, 1 up, 2 move, 3 cancel) and the `BUTTON_*` bits.
            pen::on_pen_event(
                state.action,
                state.tool_type,
                state.buttons as i32,
                state.action_button as i32,
                state.x,
                state.y,
            );
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.inner.logic(ctx);
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.perf.host_frame_ms = frame.info().cpu_usage.map(|s| f64::from(s) * 1000.0);
        // Modifiers, stylus side-channel and gestures first: the app must see
        // the modifier state and a cleaned-up tap in the frame they happen.
        let ctx = ui.ctx().clone();
        self.process_input(&ctx);
        self.inner.ui(ui);
        // Pen side-channel button remap dispatch. Runs after the app frame so
        // dialogs/menus are settled.
        self.dispatch_pen_actions(&ctx);
    }
    fn on_exit(&mut self) {
        if let Err(e) = self.inner.session.close_library() {
            log::error!("saving the library failed: {e}");
        }
    }
}

impl App {
    /// Shared [`pen`](craftapps_shared::pen) button actions → LightCraft
    /// commands. Runs after the app frame so dialogs/menus are settled.
    fn dispatch_pen_actions(&mut self, ctx: &egui::Context) {
        for qa in pen::drain_actions() {
            use craftapps_shared::pen::PenAction;
            match (qa.action, qa.pressed) {
                (PenAction::None, _) => {}
                (PenAction::Undo, true) => {
                    if let Err(e) = self.inner.session.execute("history.previous", &serde_json::Value::Null) {
                        log::warn!("pen undo: {e}");
                    }
                }
                (PenAction::Redo, true) => {
                    if let Err(e) = self.inner.session.execute("history.next", &serde_json::Value::Null) {
                        log::warn!("pen redo: {e}");
                    }
                }
                (PenAction::RightClick, true) => {
                    // Synthesize a right-click at the pen event position.
                    // The Java side passes MotionEvent coordinates, which work
                    // for hover button presses where egui has no pointer pos.
                    let ppp = ctx.pixels_per_point();
                    let pos = egui::pos2(qa.x / ppp, qa.y / ppp);
                    ctx.input_mut(|i| {
                        i.events.push(egui::Event::PointerMoved(pos));
                        i.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        });
                        i.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    });
                    ctx.request_repaint();
                }
                // LightCraft is a photo library + raw developer: it has no
                // brush/eraser/eyedropper/hand paint tools, so those remap
                // targets are no-ops here. Release events for one-shot
                // actions are ignored.
                _ => {}
            }
        }
    }
}

/// `LightCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call — no explicit registration, so no class-loader pitfalls
/// on the attach thread. Forwards to the shared SAF result handler, which
/// answers the pending dialog's reply channel.
///
/// # Safety
/// The raw pointers must be the valid JNI args the JVM passed to the entry.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_lightcraft_android_LightCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the JVM calls this with its own valid args; the shared handler
    // never unwinds into the JVM (it catches panics internally).
    unsafe {
        saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

/// `LightCraftActivity.nativeOnPenEvent`: the stylus side-channel. Java calls
/// this on pen state changes only (tool type / button transitions), not per
/// motion event.
///
/// # Safety
/// The JVM passes plain integers; no pointers cross the boundary.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_lightcraft_android_LightCraftActivity_nativeOnPenEvent(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    action: jni::sys::jint,
    tool_type: jni::sys::jint,
    buttons: jni::sys::jint,
    action_button: jni::sys::jint,
    x: jni::sys::jfloat,
    y: jni::sys::jfloat,
) {
    pen::handle_pen_event(action, tool_type, buttons, action_button, x, y);
}

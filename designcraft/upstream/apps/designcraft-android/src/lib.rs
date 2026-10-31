//! DesignCraft for Android (tablet).
//!
//! NativeActivity shell around `designcraft-ui-egui`: the shared suite
//! helpers provide the JNI bridge, SAF file pickers, `/sdcard/CraftApps`
//! storage, the Vulkan → GLES → CPU cascade and the stylus pipeline.
//! File open/import uses DesignCraft's async path (`open_async` + `inbox`)
//! because Android's pickers are asynchronous; save targets a deterministic
//! folder under the suite's shared media tree.
//!
//! This crate does not forbid `unsafe` (the JNI entry points an Android
//! shell fundamentally needs), but denies unwrap/expect/panic/todo like
//! every other crate; every `unsafe` block carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod pen;
mod services;

use android_activity::AndroidApp;
use craftapps_shared::{
    gpu,
    jni_bridge,
    pen as shared_pen,
    storage::{StorageAppInfo, StorageManager},
};
use designcraft_engine::Session;
use designcraft_ui_egui::{DesignApp, Services};
use std::path::PathBuf;
use std::sync::Arc;

/// Port version, shown in the UI and used by the self-update row.
/// `scripts/build-apk.sh` greps this line for the Gradle `versionName`.
pub const VERSION_NAME: &str = "0.9.3";

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("DesignCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("DesignCraft Android failed to start: {e}");
    }
}

/// `DesignCraftActivity.nativeOnActivityResult`: forwards the SAF picker
/// result to the shared result handler. The JVM resolves the mangled name.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_designcraft_android_DesignCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the JVM calls this with valid args on the UI thread; the
    // shared handler catches panics internally and never unwinds into Java.
    unsafe {
        craftapps_shared::saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    jni_bridge::init()?;
    log::info!("DesignCraft Android {VERSION_NAME} ({})", services::bridge_version_note());

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

    // Suite storage: /sdcard/CraftApps/{Media,Settings/DesignCraft,…}.
    let storage = Arc::new(
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "DesignCraft", settings_folder: "DesignCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );
    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    if granted {
        if let Err(e) = storage.ensure_dirs() {
            log::warn!("storage dirs: {e}");
        }
        log::info!("storage: {}", services::storage_state_line(&storage));
    } else {
        // First run: send the user to Settings to grant All-files access.
        log::warn!("All-files access not granted; opening settings");
        let _ = jni_bridge::open_all_files_settings();
    }

    // Deterministic save folder under the suite's shared media tree.
    let media_root: PathBuf = if granted {
        jni_bridge::external_storage_root()
            .map(|r| PathBuf::from(r).join("CraftApps/Media"))
            .unwrap_or_else(|e| {
                log::warn!("no external storage root ({e}); saving under the cache dir");
                cache_dir.clone()
            })
    } else {
        cache_dir.clone()
    };
    let documents_dir = media_root.join("DesignCraft/Documents");
    if let Err(e) = std::fs::create_dir_all(&documents_dir) {
        log::warn!("couldn't create {}: {e}", documents_dir.display());
    }

    // Stylus: load persisted settings and publish them to the pen runtime.
    let pen_settings = shared_pen::PenSettings::load(storage.settings_dir());
    shared_pen::sync_settings(&pen_settings);

    // Platform services (async open/import, deterministic save, SAF-aware
    // read/write, clipboard).
    let ui_services: Services = services::AndroidServices::new(storage.clone(), documents_dir).build();

    // Engine session with Android-appropriate recovery dir.
    let mut session = Session::new();
    session.recovery_dir = Some(storage.settings_dir().join("recovery"));

    // Shared GPU cascade (Vulkan → GLES → CPU) with explicit backend
    // ordering and device-error watching.
    let gpu_plan = gpu::plan(
        &storage.settings_dir().join("gpu.json"),
        &gpu::GpuAppInfo { app_name: "DesignCraft", cpu_canvas_env: "DESIGNCRAFT_CPU_CANVAS" },
    );
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    let settings_dir = storage.settings_dir().to_path_buf();
    eframe::run_native(
        "DesignCraft",
        options,
        Box::new(move |_cc| {
            // Crash recovery: reopen what a previous run left unsaved.
            if session.execute("file.recovery.open", &serde_json::json!({})).is_ok() {
                log::info!("recovered unsaved documents from a previous run");
            }
            let design_app = DesignApp::new(session, ui_services);
            Ok(Box::new(App {
                app: design_app,
                pen_settings,
                settings_dir,
                show_stylus: false,
                gestures: craftapps_shared::gestures::GestureDetector::new(),
                modifiers: craftapps_shared::modifiers::ModifierKeys::default(),
                history_len: 0,
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

/// eframe shell: the DesignCraft UI plus Android-only chrome (stylus settings
/// window) and the per-frame pen-button action dispatch.
struct App {
    app: DesignApp,
    pen_settings: shared_pen::PenSettings,
    settings_dir: PathBuf,
    show_stylus: bool,
    /// Multi-finger tap recognition (the "stylus mode" gestures).
    gestures: craftapps_shared::gestures::GestureDetector,
    /// Which modifier keys are held (winit's Android backend sends no
    /// `ModifiersChanged`, so they are tracked from the key events).
    modifiers: craftapps_shared::modifiers::ModifierKeys,
    /// The undo history length at the end of the last frame, used to tell
    /// whether a multi-finger tap painted something of its own.
    history_len: usize,
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
        craftapps_shared::saf::poll_bridge();

        // The stylus data of the last motion event: the tool type (stylus,
        // eraser end, palm) and the side-button state. winit drops both, so
        // the vendored android-activity records them on the way past.
        self.feed_stylus_state();

        // The history length before this frame's work: a multi-finger tap
        // arrives at the canvas as a click, which can paint a dab of its own.
        let history_now = self.app.session.active().map(|d| d.history.undo.len()).unwrap_or(0);
        let history_before = self.history_len;
        self.history_len = history_now;

        if let Some(tap) = self.gestures.update(ctx) {
            // Undo whatever the tap itself painted, so the gesture is the only
            // thing the tap does.
            if history_now > history_before && self.tap_painted_a_stroke() {
                let _ = self.app.run("edit.undo", serde_json::json!({}));
            }
            // The shared runtime maps the finger count to the configured
            // action (and ignores the gesture when stylus mode is off).
            let ppp = ctx.pixels_per_point();
            shared_pen::on_gesture(tap.fingers as i32, tap.pos.x * ppp, tap.pos.y * ppp);
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
            shared_pen::on_pen_event(
                state.action,
                state.tool_type,
                state.buttons as i32,
                state.action_button as i32,
                state.x,
                state.y,
            );
        }
    }

    /// Did the tap paint a stroke of its own? The history's last step names
    /// the tool that made it.
    fn tap_painted_a_stroke(&self) -> bool {
        let Some(doc) = self.app.session.active() else { return false };
        doc.history
            .undo
            .last()
            .map(|e| e.label.to_ascii_lowercase())
            .map(|label| {
                label.contains("brush")
                    || label.contains("pencil")
                    || label.contains("eraser")
                    || label.contains("pen")
            })
            .unwrap_or(false)
    }

    /// Map queued pen-button actions onto DesignCraft commands.
    fn dispatch_pen_actions(&mut self, ctx: &egui::Context) {
        for queued in shared_pen::drain_actions() {
            // Momentary actions (pan-while-held) act on press; release is a
            // no-op for now — the tool stays selected until the user changes it.
            if !queued.pressed && queued.action.is_momentary() {
                continue;
            }
            let _ = match queued.action {
                shared_pen::PenAction::None => continue,
                shared_pen::PenAction::Undo => self.app.run("edit.undo", serde_json::json!({})),
                shared_pen::PenAction::Redo => self.app.run("edit.redo", serde_json::json!({})),
                // A layout studio has no eraser or eyedropper tool; those
                // mappings stay inert rather than triggering something wrong.
                // (ToolWheel has no DesignCraft equivalent yet.)
                shared_pen::PenAction::ToggleEraser
                | shared_pen::PenAction::Eyedropper
                | shared_pen::PenAction::ToolWheel => continue,
                shared_pen::PenAction::PanTool => {
                    self.app.select_tool("hand");
                    Ok(serde_json::Value::Null)
                }
                shared_pen::PenAction::RightClick => {
                    // Synthesize a right-click at the pen event position.
                    // The Java side passes MotionEvent coordinates, which work
                    // for hover button presses where egui has no pointer pos.
                    let ppp = ctx.pixels_per_point();
                    let pos = egui::pos2(queued.x / ppp, queued.y / ppp);
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
                    // Ensure the next frame processes the injected events
                    // promptly.
                    ctx.request_repaint();
                    Ok(serde_json::Value::Null)
                }
            };
        }
    }

    /// Android-only stylus settings, drawn as a floating toggle + window so
    /// the upstream preferences UI stays untouched.
    fn stylus_chrome(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("android-stylus-toggle"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
            .show(ctx, |ui| {
                if ui.small_button("✒ Stylus").clicked() {
                    self.show_stylus = !self.show_stylus;
                }
            });
        if !self.show_stylus {
            return;
        }
        let mut open = true;
        egui::Window::new("Stylus").open(&mut open).resizable(true).show(ctx, |ui| {
            // `pen_settings_ui` syncs the runtime snapshot itself; persist to
            // disk when it reports a change.
            let changed = std::cell::Cell::new(false);
            {
                let mut mark_changed = || changed.set(true);
                shared_pen::pen_settings_ui(ui, &mut self.pen_settings, &mut mark_changed);
            }
            if changed.get() {
                if let Err(e) = self.pen_settings.save(&self.settings_dir) {
                    log::warn!("couldn't save pen settings: {e}");
                }
            }
        });
        self.show_stylus = open;
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Modifiers, stylus side-channel and gestures first: the app must see
        // the modifier state and a cleaned-up tap in the frame they happen.
        self.process_input(ctx);
        self.dispatch_pen_actions(ctx);
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        self.stylus_chrome(ui.ctx());
    }
}

//! SoundCraft for Android (tablet).
//!
//! A NativeActivity shell around [`soundcraft_ui_egui::SoundApp`]:
//! shared-storage dirs, SAF pickers through the file bridge (the app's
//! synchronous `Services` hooks, see `services.rs`), the stylus
//! side-channel, preference persistence in the suite settings folder, and
//! the Vulkan → GLES → CPU GPU cascade. Audio — playback and recording —
//! is the app's own `soundcraft_playback` over cpal's AAudio backend; the
//! shell only constructs the `Player` as the desktop host does.
//!
//! This crate does not opt into the workspace `unsafe_code = "forbid"`
//! lint: the JNI entries an Android shell needs are unsafe. It denies
//! unwrap/expect/panic/todo like every other port crate, and every unsafe
//! block carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod services;
mod shell;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen, storage::StorageAppInfo, storage::StorageManager};
use soundcraft_ui_egui::{SoundApp, UiState};

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it (`0.1.0` → 100100).
pub const VERSION_NAME: &str = "0.1.1";

const APP_NAME: &str = "SoundCraft";

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    log::info!("{APP_NAME} Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("{APP_NAME} Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    jni_bridge::init()?;

    let cache_dir = PathBuf::from(jni_bridge::cache_dir().map_err(|e| format!("cache dir: {e}"))?);

    // Shared storage needs "All files access". Nudge the user to the settings
    // page when it isn't granted yet; the app still starts (pickers work via
    // the app-private bridge dir regardless).
    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    if !granted {
        log::info!("all-files access not granted; opening settings");
        let _ = jni_bridge::open_all_files_settings();
    }
    let storage = StorageManager::new(
        &cache_dir,
        &StorageAppInfo { app_name: APP_NAME, settings_folder: "SoundCraft" },
    )
    .map(Arc::new)
    .map_err(|e| {
        log::warn!("shared storage unavailable ({e}); using app-local fallback");
        e
    })
    .ok();
    if let Some(s) = &storage {
        let _ = s.ensure_dirs();
    }

    // Suite media root for the save fallback in services.rs.
    let media_dir = storage
        .as_ref()
        .map(|s| s.media_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.join("media"));
    let _ = std::fs::create_dir_all(&media_dir);

    // SoundCraft's config folder: `ui.json`, `Autosave/` and `Presets/`
    // live here (the desktop layout, rooted in the suite settings folder).
    let settings_dir = storage
        .as_ref()
        .map(|s| s.settings_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.clone());
    let _ = std::fs::create_dir_all(&settings_dir);

    // Stylus: load the pressure curve / button map when the user saved one.
    let pen_settings = pen::PenSettings::load(&settings_dir);
    pen::sync_settings(&pen_settings);

    // The desktop resolves its settings folder from this variable (its
    // documented override); point it at the suite settings folder so any
    // upstream reader lands in the same place.
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("SOUNDCRAFT_CONFIG_DIR", &settings_dir) };

    // The file picker bridge lives in the app's own external files dir:
    // always writable, and no "All files access" permission needed. The Java
    // side reads the same path (`getExternalFilesDir(null)/.picker`).
    let bridge_base = app.external_data_path().unwrap_or_else(|| cache_dir.clone());
    let bridge_dir = bridge_base.join(".picker");
    let _ = std::fs::create_dir_all(&bridge_dir);
    // SAFETY: same as above — no other Rust thread exists yet.
    unsafe { std::env::set_var("CRAFTAPPS_BRIDGE_DIR", &bridge_dir) };

    // Shared GPU cascade (Vulkan → GLES → CPU).
    let gpu_plan = gpu::plan(
        &settings_dir.join("prefs.json"),
        &gpu::GpuAppInfo { app_name: APP_NAME, cpu_canvas_env: "SOUNDCRAFT_CPU_CANVAS" },
    );
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    let prefs_path = settings_dir.join("ui.json");
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |_cc| {
            let prefs = load_ui(&prefs_path).unwrap_or_default();
            let engine = soundcraft_engine::Engine::default();
            // The audio player, opened on the configured device as the
            // desktop opens it; without a device it runs a silent clock
            // rather than failing. Recording opens on first use, in the
            // app itself (`Recorder::open_with(&ui.audio)`).
            let player = soundcraft_playback::Player::with_config(
                Arc::new(engine.session().clone()),
                &prefs.audio,
            );
            let services = services::build_services(storage.clone(), media_dir.clone());
            let mut app = SoundApp::new(engine, Some(player), services);
            app.autosave_dir = Some(settings_dir.join("Autosave"));
            app.leftovers = app
                .autosave_dir
                .as_deref()
                .map(soundcraft_engine::autosave::leftovers)
                .unwrap_or_default();
            app.preset_dir = Some(settings_dir.join("Presets"));
            app.extra.ui_scale = prefs.ui_scale;
            app.ui = prefs;
            Ok(Box::new(shell::ShellApp::new(app, prefs_path.clone())) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

/// Read the saved UI state (`ui.json`), as the desktop host does. An
/// unreadable or malformed file yields `None` (defaults) with a warning —
/// the desktop additionally recovers the docking fields from a partially
/// damaged file; that refinement is not replicated here.
fn load_ui(path: &Path) -> Option<UiState> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::warn!("settings could not be read: {e}");
            return None;
        }
    };
    match serde_json::from_slice::<UiState>(&bytes) {
        Ok(state) => Some(state),
        Err(e) => {
            log::warn!("settings could not be restored: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// JNI entries. The JVM resolves these mangled names on first call; each port
// defines its own set forwarding to the shared handlers.

/// `<Activity>.nativeOnActivityResult`: forwards SAF picker results.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_soundcraft_android_SoundCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the JVM calls this with the valid JNI args it was given; the
    // shared handler never unwinds into the JVM.
    unsafe {
        craftapps_shared::saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

/// `<Activity>.nativeOnPenEvent`: the stylus side-channel (tool type, barrel
/// buttons) that winit's Android backend drops.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_soundcraft_android_SoundCraftActivity_nativeOnPenEvent(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    action: jni::sys::jint,
    tool: jni::sys::jint,
    buttons: jni::sys::jint,
    action_button: jni::sys::jint,
    x: jni::sys::jfloat,
    y: jni::sys::jfloat,
) {
    // handle_pen_event catches unwinds itself; plain integers/floats cross the ABI.
    pen::handle_pen_event(action, tool, buttons, action_button, x, y);
}

/// `<Activity>.nativeOnModifierState`: the Bluetooth-keyboard modifier
/// bitmask the Activity's `dispatchKeyEvent` tracks (bit 0 = Ctrl,
/// 1 = Shift, 2 = Alt). On a `NativeActivity` that Java hook never runs —
/// the live path is `craftapps_shared::modifiers`, which tracks the same
/// keys from egui's key events — so this only stores the value, for parity
/// with the other ports.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_soundcraft_android_SoundCraftActivity_nativeOnModifierState(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    modifiers: jni::sys::jint,
) {
    use std::sync::atomic::{AtomicI32, Ordering};
    static MODIFIER_STATE: AtomicI32 = AtomicI32::new(0);
    MODIFIER_STATE.store(modifiers, Ordering::SeqCst);
}

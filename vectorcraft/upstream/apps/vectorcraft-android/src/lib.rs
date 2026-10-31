//! VectorCraft for Android (tablet).
//!
//! A NativeActivity shell around [`vectorcraft_ui_egui::VectorcraftApp`]:
//! shared-storage media library, SAF pickers with an in-app browser first,
//! the stylus side-channel, and the Vulkan → GLES → CPU GPU cascade.
//!
//! This crate does not opt into the workspace `unsafe_code = "deny"` lint:
//! the JNI entries an Android shell needs are unsafe. It denies
//! unwrap/expect/panic/todo like every other crate, and every unsafe block
//! carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod services;
mod shell;

use std::path::PathBuf;
use std::sync::Arc;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen, storage::StorageManager, storage::StorageAppInfo};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it (`0.1.0` → 100100).
pub const VERSION_NAME: &str = "0.2.2";

const APP_NAME: &str = "VectorCraft";

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("{APP_NAME} Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("{APP_NAME} Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    jni_bridge::init()?;

    let cache_dir = PathBuf::from(jni_bridge::cache_dir().map_err(|e| format!("cache dir: {e}"))?);

    // Shared storage needs "All files access". Nudge the user to the settings
    // page when it isn't granted yet; the app still starts (browser falls back
    // to the app cache dir, SAF stays disabled until granted).
    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    if !granted {
        log::info!("all-files access not granted; opening settings");
        let _ = jni_bridge::open_all_files_settings();
    }
    let storage = StorageManager::new(
        &cache_dir,
        &StorageAppInfo { app_name: APP_NAME, settings_folder: "VectorCraft" },
    )
    .map(Arc::new)
    .map_err(|e| {
        log::warn!("shared storage unavailable ({e}); using app-local fallback");
        e
    })
    .ok();

    // Media library root for the in-app browser.
    let browser_root = storage
        .as_ref()
        .map(|s| s.media_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.join("media"));
    let _ = std::fs::create_dir_all(&browser_root);
    let inbox = cache_dir.join("saf-inbox");
    let _ = std::fs::create_dir_all(&inbox);

    // Stylus: load the pressure curve / button map when the user saved one.
    let settings_dir = storage
        .as_ref()
        .map(|s| s.settings_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.clone());
    let pen_settings = pen::PenSettings::load(&settings_dir);
    pen::sync_settings(&pen_settings);

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

    // Shared GPU cascade (Vulkan → GLES → CPU).
    let gpu_plan = gpu::plan(
        &settings_dir.join("prefs.json"),
        &gpu::GpuAppInfo { app_name: APP_NAME, cpu_canvas_env: "VECTORCRAFT_CPU_CANVAS" },
    );
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    let android_services = services::AndroidServices::new(storage, browser_root, inbox);
    let services = android_services.services();

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |_cc| {
            let session = Session::new();
            let app = VectorcraftApp::new(session, services);
            Ok(Box::new(shell::ShellApp::new(app, android_services)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

// ---------------------------------------------------------------------------
// JNI entries. The JVM resolves these mangled names on first call; each port
// defines its own pair forwarding to the shared handlers.

/// `<Activity>.nativeOnActivityResult`: forwards SAF picker results.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_vectorcraft_android_VectorCraftActivity_nativeOnActivityResult(
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
pub unsafe extern "system" fn Java_app_vectorcraft_android_VectorCraftActivity_nativeOnPenEvent(
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

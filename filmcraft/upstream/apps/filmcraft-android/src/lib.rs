//! FilmCraft for Android (tablet).
//!
//! A NativeActivity shell around [`filmcraft_ui_egui::FilmcraftApp`]:
//! shared-storage dirs, SAF pickers through the file bridge (the app's
//! `run_dialog` hook), the stylus side-channel, and the Vulkan → GLES → CPU
//! GPU cascade.
//!
//! This crate does not opt into the workspace `unsafe_code = "forbid"` lint:
//! the JNI entries an Android shell needs are unsafe. It denies
//! unwrap/expect/panic/todo like every other port crate, and every unsafe
//! block carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod audio;
mod picks;
mod services;
mod shell;

use std::path::PathBuf;
use std::sync::Arc;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen, storage::StorageAppInfo, storage::StorageManager};
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it (`0.1.0` → 100100).
pub const VERSION_NAME: &str = "0.1.2";

const APP_NAME: &str = "FilmCraft";

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
    // page when it isn't granted yet; the app still starts (pickers work via
    // the app-private bridge dir regardless).
    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    if !granted {
        log::info!("all-files access not granted; opening settings");
        let _ = jni_bridge::open_all_files_settings();
    }
    let storage = StorageManager::new(
        &cache_dir,
        &StorageAppInfo { app_name: APP_NAME, settings_folder: "FilmCraft" },
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

    // Suite media root for the save fallback in picks.rs.
    let media_dir = storage
        .as_ref()
        .map(|s| s.media_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.join("media"));
    let _ = std::fs::create_dir_all(&media_dir);

    // Stylus: load the pressure curve / button map when the user saved one.
    let settings_dir = storage
        .as_ref()
        .map(|s| s.settings_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.clone());
    let pen_settings = pen::PenSettings::load(&settings_dir);
    pen::sync_settings(&pen_settings);

    // The file picker bridge lives in the app's own external files dir:
    // always writable, and no "All files access" permission needed. The Java
    // side reads the same path (`getExternalFilesDir(null)/.picker`).
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
        &gpu::GpuAppInfo { app_name: APP_NAME, cpu_canvas_env: "FILMCRAFT_CPU_CANVAS" },
    );
    let allow_gpu = gpu_plan.allow_gpu_canvas;
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let mut session = Session::new(std::sync::Arc::new(services::AndroidServices::new(media_dir.clone())));
            // Auto-save / crash recovery in the suite settings dir. The
            // config's default local offset is UTC; recovery stamps just
            // read a few hours off on the tablet.
            let save_cfg =
                filmcraft_engine::autosave::AutosaveConfig::new(settings_dir.join("Autosave"));
            if let Err(e) = session.start_autosave(save_cfg) {
                log::warn!("auto-save unavailable: {e}");
            }
            // A previous run may have persisted the desktop start
            // directory ("/" on Android) as the Media Browser's last
            // directory; reset anything outside shared storage to the
            // suite media folder.
            let last_dir = &session.prefs.media_browser.last_dir;
            if !last_dir.is_empty() && !last_dir.starts_with("/sdcard") {
                session.prefs.media_browser.last_dir = media_dir.to_string_lossy().into_owned();
            }
            // First run (no project yet): the app opens its Home screen; do
            // not force a project open here.

            let mut app = FilmcraftApp::new(session);
            // GPU compositor on the eframe wgpu device (the desktop main
            // does the same through `set_wgpu`); skipped when the plan says
            // CPU (prefs/env), in which case the CPU compositor runs.
            if allow_gpu {
                if let Some(rs) = cc.wgpu_render_state.clone() {
                    app.set_wgpu(rs);
                }
            }
            app.audio = Some(Box::new(audio::AndroidAudioOut::new()));
            // File dialogs via the shared file bridge (see picks.rs). The
            // synchronous pick hooks stay unset: with `run_dialog` present
            // the app never calls them.
            let media = media_dir.clone();
            app.hooks.run_dialog = Some(Box::new(move |dialog| picks::run_dialog(dialog, media.clone())));
            Ok(Box::new(shell::ShellApp::new(app)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

// ---------------------------------------------------------------------------
// JNI entries. The JVM resolves these mangled names on first call; each port
// defines its own pair forwarding to the shared handlers.

/// `<Activity>.nativeOnActivityResult`: forwards SAF picker results.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_filmcraft_android_FilmCraftActivity_nativeOnActivityResult(
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
pub unsafe extern "system" fn Java_app_filmcraft_android_FilmCraftActivity_nativeOnPenEvent(
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

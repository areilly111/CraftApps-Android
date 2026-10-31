//! PdfCraft for Android (tablet).
//!
//! A NativeActivity shell around `pdfcraft-ui-egui`'s [`PdfCraftApp`]:
//! - shared suite storage (`/sdcard/CraftApps`), SAF file pickers and the
//!   `rfd` shim's pick handler (see [`file_browser`]);
//! - stylus support through the shared [`pen`](craftapps_shared::pen) module
//!   and the activity's `dispatchTouchEvent` side-channel;
//! - the shared GPU cascade (Vulkan → GLES → CPU).
//!
//! This crate does not forbid `unsafe` (the JNI the shell needs), but denies
//! unwrap/expect/panic/todo like every other crate; every `unsafe` block
//! carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod services;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen};
use std::path::PathBuf;

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it.
pub const VERSION_NAME: &str = "0.2.3";

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("PdfCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("PdfCraft Android failed to start: {e}");
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
    // Edition 2021: `set_var` is still a safe fn here (it became unsafe only
    // in edition 2024).
    std::env::set_var("CRAFTAPPS_BRIDGE_DIR", &bridge_dir);

    let storage = services::init_storage(&cache_dir)?;

    // Route the rfd shim's picks through the SAF file browser.
    file_browser::install_pick_handler(storage.clone());

    // Shared GPU cascade (Vulkan → GLES → CPU).
    let gpu_plan = gpu::plan(
        &storage.settings_dir().join("prefs.json"),
        &gpu::GpuAppInfo { app_name: "PdfCraft", cpu_canvas_env: "PDFCRAFT_CPU_CANVAS" },
    );
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "PdfCraft",
        options,
        Box::new(move |_cc| Ok(Box::new(shell::PdfCraftShell::new()) as Box<dyn eframe::App>)),
    )
    .map_err(|e| format!("eframe: {e}"))
}

// ---------------------------------------------------------------------------
// JNI entries called by PdfCraftActivity.
// ---------------------------------------------------------------------------

/// `PdfCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call; no explicit registration.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_pdfcraft_android_PdfCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the JVM passes valid args; the shared handler never unwinds.
    unsafe {
        craftapps_shared::saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

/// `PdfCraftActivity.nativeOnPenEvent`: the stylus side-channel. The
/// activity's `dispatchTouchEvent` forwards tool-type and barrel-button
/// transitions here; winit never sees them.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_pdfcraft_android_PdfCraftActivity_nativeOnPenEvent(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    action: jni::sys::jint,
    tool: jni::sys::jint,
    buttons: jni::sys::jint,
    action_button: jni::sys::jint,
    x: jni::sys::jfloat,
    y: jni::sys::jfloat,
) {
    pen::on_pen_event(action, tool, buttons, action_button, x, y);
}

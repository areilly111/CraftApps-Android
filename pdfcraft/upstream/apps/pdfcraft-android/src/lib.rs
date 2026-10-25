//! PdfCraft for Android (tablet).
//!
//! A NativeActivity shell around `pdfcraft-ui-egui`'s [`PdfCraftApp`]:
//! - app-private persistent storage, SAF file pickers and the
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
mod incoming;
mod services;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen};
use std::path::PathBuf;

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it.
pub const VERSION_NAME: &str = "0.2.8";

static DOCUMENT_CONTEXT: std::sync::Mutex<Option<egui::Context>> = std::sync::Mutex::new(None);

pub(crate) fn install_document_waker(ctx: &egui::Context) {
    if let Ok(mut slot) = DOCUMENT_CONTEXT.lock() {
        *slot = Some(ctx.clone());
    }
}

/// Java publishes a complete import event before waking the render thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_pdfcraft_android_PdfCraftActivity_nativeOnDocumentReady(_env: *mut jni::sys::JNIEnv, _this: jni::sys::jobject) {
    if let Ok(slot) = DOCUMENT_CONTEXT.lock() {
        if let Some(ctx) = slot.as_ref() {
            ctx.request_repaint();
        }
    }
}

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

    // Java uses getFilesDir() as well. Keep the bridge independent of shared
    // storage permission and of external-storage availability.
    let bridge_base = PathBuf::from(jni_bridge::files_dir()?);
    let bridge_dir = bridge_base.join(".picker");
    std::fs::create_dir_all(&bridge_dir).map_err(|e| format!("picker directory: {e}"))?;
    // Edition 2021: `set_var` is still a safe fn here (it became unsafe only
    // in edition 2024).
    std::env::set_var("CRAFTAPPS_BRIDGE_DIR", &bridge_dir);

    let storage = services::init_storage(&cache_dir)?;
    log::info!("Private storage ready: {}", storage.settings_dir().display());

    // Route the rfd shim's picks through the SAF file browser.
    file_browser::install_pick_handler(storage.clone());

    // Shared GPU cascade (Vulkan → GLES → CPU).
    let gpu_plan =
        gpu::plan(&storage.settings_dir().join("prefs.json"), &gpu::GpuAppInfo { app_name: "PdfCraft", cpu_canvas_env: "PDFCRAFT_CPU_CANVAS" });
    let mut options =
        eframe::NativeOptions { android_app: Some(app), persistence_path: Some(storage.settings_dir().join("ui-state.ron")), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native("PdfCraft", options, Box::new(move |cc| Ok(Box::new(shell::PdfCraftShell::new(cc.storage)) as Box<dyn eframe::App>)))
        .map_err(|e| format!("eframe: {e}"))
}

// ---------------------------------------------------------------------------
// JNI entries called by PdfCraftActivity.
// ---------------------------------------------------------------------------

/// `PdfCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call; no explicit registration.
///
/// # Safety
/// The JVM must supply the valid JNI environment, activity and result objects
/// for this callback; they are forwarded to the shared JNI boundary.
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
///
/// # Safety
/// This callback must be invoked with the JVM's native-method calling convention.
/// Object/environment pointers are unused and the numeric event values are copied.
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

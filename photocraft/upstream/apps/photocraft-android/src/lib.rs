//! PhotoCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`PhotocraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! Pen pressure is free: winit's Android backend forwards `MotionEvent`
//! pressure as `egui::Event::Touch { force }`, which
//! `photocraft_ui_egui::stylus::Stylus::update` already reads. The Java
//! side-channel (`PhotoCraftActivity.dispatchTouchEvent`) reports the tool
//! type and barrel-button state winit drops; those feed the shared pen
//! runtime and the app's [`stylus::StylusFeed`].
//!
//! Workspace rules (see `upstream/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "forbid"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod keyboard;
mod pen;
mod services;
mod shell;

use android_activity::AndroidApp;
use photocraft_engine::{Session, preset_store};
use photocraft_ui_egui::PhotocraftApp;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::file_browser::{FileBrowser, GalleryFlag};
use crate::services::AndroidDirs;
use crate::shell::{AndroidShell, StorageGate};

/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it.
pub const VERSION_NAME: &str = "0.4.6";

/// Per-app glue for the shared GPU cascade: attach the wgpu render state to
/// the app for GPU canvas compositing, or record the CPU fallback.
/// (The shared [`craftapps_shared::gpu::GpuCanvasHost`] trait can't be
/// implemented here — orphan rule — so the small attach step is inline.)
fn attach_gpu_canvas(
    app: &mut PhotocraftApp,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    plan: &craftapps_shared::gpu::GpuPlan,
) {
    let Some(rs) = render_state else { return };
    let info = rs.adapter.get_info();
    app.perf.gpu_info.set_adapter(&info);
    let software_window = info.device_type == eframe::wgpu::DeviceType::Cpu;
    if plan.allow_gpu_canvas && !software_window {
        app.set_wgpu(rs);
    } else {
        if software_window {
            app.perf.gpu_info.fallback =
                Some("No hardware graphics adapter; the canvas composites on the CPU.".to_string());
        }
        // The window still draws with wgpu: watch the device for errors instead of
        // letting them panic (the desktop `main.rs` does the same).
        let _ = photocraft_ui_egui::gpu_canvas::DeviceHealth::watch(&rs.device);
    }
}

/// The android-activity 0.6 `native-activity` entry point: exported, unmangled,
/// Rust ABI. Runs on a dedicated thread android-activity spawns per Activity.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("PhotoCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("PhotoCraft Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    // Internal storage root; the permission gate's choice file lives here too.
    let internal = app.internal_data_path().ok_or("no internal data path")?;
    std::fs::create_dir_all(&internal).map_err(|e| format!("couldn't create {}: {e}", internal.display()))?;

    craftapps_shared::jni_bridge::init()?;

    // Files live in the app's external files dir: no storage permission needed.
    let files_dir = app.external_data_path().unwrap_or_else(|| {
        log::warn!("no external data path; using internal storage for files");
        internal.clone()
    });
    std::fs::create_dir_all(&files_dir).map_err(|e| format!("couldn't create {}: {e}", files_dir.display()))?;
    let cache_dir = PathBuf::from(craftapps_shared::jni_bridge::cache_dir().unwrap_or_else(|e| {
        log::warn!("cache dir lookup failed ({e}); using the files dir");
        files_dir.to_string_lossy().into_owned()
    }));

    // The storage manager: shared-folder paths plus SAF plumbing. It answers
    // SAF URIs even without "All files access"; the `/sdcard/CraftApps` paths
    // need the permission, which the gate below handles.
    let storage = Arc::new(
        craftapps_shared::storage::StorageManager::new(
            &cache_dir,
            &craftapps_shared::storage::StorageAppInfo { app_name: "PhotoCraft", settings_folder: "PhotoCraft" },
        )
        .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = craftapps_shared::jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir. Every persisted file (prefs, recovery, presets, updater
    // setting) lives under it.
    let (config, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("PhotoCraft");
        for sub in [&dir, &dir.join("Recovery"), &dir.join("Presets")] {
            std::fs::create_dir_all(sub).map_err(|e| format!("couldn't create {}: {e}", sub.display()))?;
        }
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("PHOTOCRAFT_CONFIG_DIR", &config) };
    // The file picker bridge lives in the app's own external files dir: always
    // writable, and no "All files access" permission needed. The Java side
    // reads the same path (`getExternalFilesDir(null)/.picker`).
    unsafe { std::env::set_var("CRAFTAPPS_BRIDGE_DIR", files_dir.join(".picker")) };

    let gallery: GalleryFlag = Arc::new(Mutex::new(None));
    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir.
    let browser_root: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let browser = Arc::new(Mutex::new(FileBrowser::new(storage.clone(), browser_root, gallery.clone())));
    let dirs = AndroidDirs { config: config.clone() };
    let mut services = crate::services::android_services(&dirs, browser.clone(), gallery.clone(), storage.clone());
    // Brush presets load in the background; the app attaches them on arrival.
    services.preset_store = Some(preset_store::open_dir_async(config.join("Presets")));

    let updater = Arc::new(Mutex::new(craftapps_shared::updater::Updater::new(
        "PhotoCraft",
        "PhotoCraft-",
        VERSION_NAME,
        cache_dir,
        &config,
        "app.photocraft.android.apkprovider",
    )));
    // Pen/stylus: load persisted settings, publish to the JNI runtime.
    let pen_settings = Arc::new(Mutex::new(craftapps_shared::pen::PenSettings::load(&config)));
    {
        let guard = pen_settings.lock().unwrap_or_else(|poison| poison.into_inner());
        craftapps_shared::pen::sync_settings(&guard);
    }

    // GPU backend cascade (Vulkan → GLES → CPU canvas), honoring the persisted
    // Performance prefs.
    let gpu_plan = craftapps_shared::gpu::plan(
        &config.join("preferences.json"),
        &craftapps_shared::gpu::GpuAppInfo { app_name: "PhotoCraft", cpu_canvas_env: "PHOTOCRAFT_CPU_CANVAS" },
    );

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        persistence_path: Some(config.join("ui.ron")),
        ..Default::default()
    };
    craftapps_shared::gpu::configure(&mut options, &gpu_plan);

    let pen_settings_shell = pen_settings.clone();
    let pen_config_dir = config.clone();
    eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            let mut inner = PhotocraftApp::new(Session::new(), services);
            inner.background_jobs = true;
            // Attach the GPU canvas (or record the CPU fallback) from the
            // actual wgpu render state — this is what populates the
            // Performance panel's graphics/backend/driver fields.
            attach_gpu_canvas(&mut inner, cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(AndroidShell::new(
                inner,
                browser,
                updater,
                gate,
                pen_settings_shell,
                pen_config_dir,
            )) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// `PhotoCraftActivity.nativeOnActivityResult(requestCode, resultCode, data)`.
/// The JVM resolves the mangled name on first call. Forwards to the shared
/// SAF result handler, which answers the pending file dialog.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_photocraft_android_PhotoCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the JVM passes valid JNI args; the shared handler confines the
    // raw pointers to this call and never unwinds into the JVM.
    unsafe {
        craftapps_shared::saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

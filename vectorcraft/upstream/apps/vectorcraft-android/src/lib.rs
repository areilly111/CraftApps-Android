//! VectorCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`VectorcraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! Pen pressure is free: winit's Android backend forwards `MotionEvent`
//! pressure as `egui::Event::Touch { force }`, which the drawing tools read
//! with no cfg gates. Tilt, twist and the eraser end are dropped by winit
//! 0.30.
//!
//! Workspace rules (see `upstream/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "deny"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod services;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::gpu;
use craftapps_shared::storage::{StorageAppInfo, StorageManager};
use craftapps_shared::updater::Updater;
use craftapps_shared::{jni_bridge, saf};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

use crate::file_browser::FileBrowser;
use crate::shell::{AndroidShell, StorageGate};
/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.8.0-port1` → 80001).
pub const VERSION_NAME: &str = "0.2.0";

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]).
/// VectorCraft's canvas is CPU-rendered into an egui texture (there is no GPU
/// canvas hook like PhotoCraft's `set_wgpu`): the cascade still orders adapter
/// selection (Vulkan first) for the window, the adapter is recorded for
/// Help › About, and the device is watched for errors instead of panicking.
///
/// A local newtype: the orphan rule forbids implementing the shared trait for
/// the upstream app type directly.
struct VcHost<'a>(&'a mut VectorcraftApp);

impl gpu::GpuCanvasHost for VcHost<'_> {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        // Shown in Help › About and `ui.inspect` for GPU bug reports (the
        // desktop `main.rs` formats it the same way).
        self.0.graphics_adapter =
            Some(format!("{} ({:?}, {:?})", info.name.trim(), info.backend, info.device_type));
    }
    fn record_software_fallback(&mut self, note: &str) {
        let base = self.0.graphics_adapter.take().unwrap_or_else(|| "unknown adapter".to_string());
        self.0.graphics_adapter = Some(format!("{base} — {note}"));
    }
    fn set_gpu_canvas(&mut self, rs: eframe::egui_wgpu::RenderState) {
        self.watch_device(&rs.device);
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        // The window still draws with wgpu: watch the device for errors instead
        // of letting them panic (the desktop `main.rs` does the same).
        device.set_device_lost_callback(|reason, msg| {
            log::error!("graphics device lost: {reason:?}: {msg}");
        });
    }
}

/// `VectorCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call; `this` is the activity (used for the ContentResolver).
/// Forwards to the shared SAF result handler.
///
/// # Safety
///
/// Must only be called by the JVM with valid JNI arguments: `raw_env` is the
/// current thread's JNI environment, `this_raw` is the calling activity, and
/// `data_raw` is null or the intent's data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_vectorcraft_android_VectorCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the raw pointers are the valid JNI args the JVM passed to this
    // entry; `activity_result_ffi` documents the same contract.
    unsafe {
        saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

/// The android-activity 0.6 `native-activity` entry point: exported, unmangled,
/// Rust ABI. Runs on a dedicated thread android-activity spawns per Activity.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("VectorCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("VectorCraft Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    // Internal storage root; the permission gate's choice file lives here too.
    let internal = app.internal_data_path().ok_or("no internal data path")?;
    std::fs::create_dir_all(&internal).map_err(|e| format!("couldn't create {}: {e}", internal.display()))?;

    jni_bridge::init()?;

    // Files live in the app's external files dir: no storage permission needed.
    let files_dir = app.external_data_path().unwrap_or_else(|| {
        log::warn!("no external data path; using internal storage for files");
        internal.clone()
    });
    std::fs::create_dir_all(&files_dir).map_err(|e| format!("couldn't create {}: {e}", files_dir.display()))?;
    let cache_dir = PathBuf::from(jni_bridge::cache_dir().unwrap_or_else(|e| {
        log::warn!("cache dir lookup failed ({e}); using the files dir");
        files_dir.to_string_lossy().into_owned()
    }));

    // The storage manager: shared-folder paths plus SAF plumbing. It answers
    // SAF URIs even without "All files access"; the `/sdcard/ArtCraft` paths
    // need the permission, which the gate below handles.
    let storage = Arc::new(
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "VectorCraft", settings_folder: "VectorCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir (the v1 layout). Preferences (`ui.json`), Data Recovery,
    // Fonts, Swatches, Graphic Styles and Libraries all live under it.
    let (config, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("VectorCraft");
        for sub in [&dir, &dir.join("Data Recovery")] {
            std::fs::create_dir_all(sub).map_err(|e| format!("couldn't create {}: {e}", sub.display()))?;
        }
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };
    // Point the XDG config lookup at the Android config dir, so `ui.json`,
    // Data Recovery, Fonts, Swatches, Graphic Styles and Libraries land under
    // it (`<config>/vectorcraft/…`) instead of an Android home directory.
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &config) };
    let prefs_file = config.join("vectorcraft").join("ui.json");
    // The app's own Fonts folder, next to the settings (the desktop's
    // `prefs_dir::install_fonts`).
    vectorcraft_text::set_app_font_dir(config.join("vectorcraft").join("Fonts"));

    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir (the v1 layout).
    let browser_root: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let browser = Arc::new(Mutex::new(
        FileBrowser::new(storage.clone(), browser_root).map_err(|e| format!("file browser: {e}"))?,
    ));
    
    let services = crate::services::android_services(browser.clone(), cache_dir.clone());

    let mut session = Session::new();
    // Data Recovery copies live next to the preferences (the desktop's main.rs).
    session.recovery.set_default_folder(Some(config.join("vectorcraft").join("Data Recovery").to_string_lossy().into_owned()));
    let mut inner = VectorcraftApp::new(session, services);
    load_prefs(&mut inner, &prefs_file);

    let updater = Arc::new(Mutex::new(Updater::new(
        "VectorCraft",
        "VectorCraft-",
        VERSION_NAME,
        cache_dir,
        &config,
        "app.vectorcraft.android.apkprovider",
    )));
    // TODO: Port wire_android_hooks to new APIs (AndroidHooks removed).
    // wire_android_hooks(&mut inner.services, updater.clone(), storage, granted);
    let _ = (&mut inner.services, updater.clone(), storage, granted);

    // GPU backend cascade (Vulkan → GLES → CPU canvas). VectorCraft's engine
    // prefs name the choice `gpuPreference` (automatic/lowPower/highPower);
    // the shared cascade reads the PhotoCraft-style `performance.gpuBackend`,
    // so until that mapping exists the port runs the default cascade.
    let gpu_plan = gpu::plan(
        &config.join("vectorcraft").join("ui.json"),
        &gpu::GpuAppInfo { app_name: "VectorCraft", cpu_canvas_env: "VECTORCRAFT_CPU_CANVAS" },
    );

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "VectorCraft",
        options,
        Box::new(move |cc| {
            // Attach the wgpu render state: records the adapter for Help ›
            // About (or the CPU fallback) from the actual device.
            gpu::attach_canvas(&mut VcHost(&mut inner), cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(AndroidShell::new(inner, browser, updater, gate, prefs_file)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Read the saved UI state and engine prefs (`ui.json`), like the desktop's
/// `load_prefs`.
fn load_prefs(app: &mut VectorcraftApp, prefs_file: &PathBuf) {
    if let Ok(bytes) = std::fs::read(prefs_file)
        && let Ok(ui) = serde_json::from_slice::<vectorcraft_ui_egui::UiState>(&bytes)
    {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(app);
}

/// Connect the Android-only shell hooks: Help → Check for Updates…, and
/// Preferences › Storage (media location tablet/SD toggle + SD tree picker).
/// The menu item and the section are registered Android-only, so these
/// closures only ever run on Android. Sibling ports reuse this verbatim.
fn wire_android_hooks(
    services: &mut vectorcraft_ui_egui::Services,
    storage: Arc<StorageManager>,
    granted: bool,
) {
    let label_storage = storage.clone();
    let tablet_storage = storage.clone();
    let sd_storage = storage.clone();
    services.android_hooks = Some(vectorcraft_ui_egui::AndroidHooks {
        storage: vectorcraft_ui_egui::AndroidStorageHooks {
            location_label: Box::new(move || {
                if granted {
                    label_storage.location_label()
                } else {
                    "Internal app storage (shared folders not enabled)".to_string()
                }
            }),
            use_tablet_storage: Box::new(move || {
                if granted {
                    tablet_storage.use_tablet_storage();
                } else {
                    // No permission yet: take them to the grant screen. The
                    // startup gate finishes the job on the next launch.
                    let _ = jni_bridge::open_all_files_settings();
                }
            }),
            pick_folder: Box::new(move || {
                // The folder picker needs no permission: the chosen tree is
                // read and written through SAF directly.
                saf::set_pending_tree(sd_storage.clone());
                if let Err(e) = saf::launch_open_document_tree() {
                    log::error!("couldn't open the folder picker: {e}");
                    saf::cancel_pending();
                }
            }),
        },
        pen: None,    });
}

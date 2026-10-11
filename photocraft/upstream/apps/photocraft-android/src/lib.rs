//! PhotoCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`PhotocraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! Pen pressure is free: winit's Android backend forwards `MotionEvent`
//! pressure as `egui::Event::Touch { force }`, which
//! `photocraft_ui_egui::Stylus::update` already reads with no cfg gates.
//! Tilt, twist and the eraser end are dropped by winit 0.30: v1.1 work.
//!
//! Workspace rules (see `upstream/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "forbid"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod gpu;
mod jni_bridge;
mod pen;
mod saf;
mod services;
mod shell;
mod storage;
mod updater;

use android_activity::AndroidApp;
use photocraft_engine::{Session, preset_store};
use photocraft_ui_egui::PhotocraftApp;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::file_browser::{FileBrowser, GalleryFlag};
use crate::services::AndroidDirs;
use crate::shell::{AndroidShell, StorageGate};
use crate::storage::{StorageAppInfo, StorageManager};
use crate::updater::Updater;

/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.6.0-port1` → 60001).
pub const VERSION_NAME: &str = "0.1.0";

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]): how the render
/// state reaches this port's app type. Sibling ports implement the same trait for theirs;
/// the cascade logic in `gpu.rs` stays untouched.
impl gpu::GpuCanvasHost for PhotocraftApp {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        self.perf.gpu_info.set_adapter(info);
    }
    fn record_software_fallback(&mut self, note: &str) {
        self.perf.gpu_info.fallback = Some(note.to_string());
    }
    fn set_gpu_canvas(&mut self, rs: eframe::egui_wgpu::RenderState) {
        self.set_wgpu(rs);
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        // The window still draws with wgpu: watch the device for errors instead of
        // letting them panic (the desktop `main.rs` does the same).
        let _ = photocraft_ui_egui::gpu_canvas::DeviceHealth::watch(device);
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

    jni_bridge::init()?;
    // `startActivityForResult` must run on the Activity; the SAF pickers route
    // through the bridge's activity ref (`<App>Activity.launchForResult`).

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
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "PhotoCraft", settings_folder: "PhotoCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir (the v1 layout). Every persisted file (prefs, recovery,
    // presets, updater setting) lives under it via this override.
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

    let gallery: GalleryFlag = Arc::new(Mutex::new(None));
    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir (the v1 layout).
    let browser_root: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let browser = Arc::new(Mutex::new(FileBrowser::new(storage.clone(), browser_root, gallery.clone())));
    let dirs = AndroidDirs { config: config.clone() };
    let mut services = crate::services::android_services(&dirs, browser.clone(), gallery.clone(), storage.clone());
    // Brush presets load in the background; the app attaches them on arrival.
    services.preset_store = Some(preset_store::open_dir_async(config.join("Presets")));

    let updater = Arc::new(Mutex::new(Updater::new(VERSION_NAME, cache_dir, &config)));
    // Pen/stylus: load persisted settings, publish to the JNI runtime.
    let pen_settings =
        Arc::new(Mutex::new(craftapps_shared::pen::PenSettings::load(&config)));
    {
        let guard = pen_settings.lock().unwrap_or_else(|poison| poison.into_inner());
        craftapps_shared::pen::sync_settings(&guard);
    }
    let pen_config_dir = config.clone();
    wire_android_hooks(&mut services, updater.clone(), storage, granted, pen_settings.clone(), pen_config_dir);

    // GPU backend cascade (Vulkan → GLES → CPU canvas), honoring the persisted
    // Performance prefs. Replaces the v1 hardware-adapter picker below.
    let gpu_plan = gpu::plan(
        &config.join("preferences.json"),
        &gpu::GpuAppInfo { app_name: "PhotoCraft", cpu_canvas_env: "PHOTOCRAFT_CPU_CANVAS" },
    );

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        persistence_path: Some(config.join("ui.ron")),
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            let mut inner = PhotocraftApp::new(Session::new(), services);
            inner.background_jobs = true;
            // Attach the GPU canvas (or record the CPU fallback) from the
            // actual wgpu render state — this is what populates the
            // Performance panel's graphics/backend/driver fields.
            gpu::attach_canvas(&mut inner, cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(AndroidShell::new(inner, browser, updater, gate)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Connect the Android-only shell hooks: Help → Check for Updates…,
/// Preferences › Storage (media location tablet/SD toggle + SD tree picker),
/// and Preferences › Stylus (shared pen settings UI).
/// The menu item and the sections are registered Android-only, so these
/// closures only ever run on Android. Sibling ports reuse this verbatim.
fn wire_android_hooks(
    services: &mut photocraft_ui_egui::Services,
    updater: Arc<Mutex<Updater>>,
    storage: Arc<StorageManager>,
    granted: bool,
    pen_settings: Arc<Mutex<craftapps_shared::pen::PenSettings>>,
    pen_config_dir: PathBuf,
) {
    let label_storage = storage.clone();
    let tablet_storage = storage.clone();
    let sd_storage = storage.clone();
    services.android_hooks = Some(photocraft_ui_egui::AndroidHooks {
        check_updates: Box::new(move || {
            updater.lock().unwrap_or_else(|poison| poison.into_inner()).open();
        }),
        storage: photocraft_ui_egui::AndroidStorageHooks {
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
                    let _ = crate::jni_bridge::open_all_files_settings();
                }
            }),
            pick_sd_card: Box::new(move || {
                // The folder picker needs no permission: the chosen tree is
                // read and written through SAF directly.
                crate::saf::set_pending_tree(sd_storage.clone());
                if let Err(e) = crate::saf::launch_open_document_tree() {
                    log::error!("couldn't open the folder picker: {e}");
                    crate::saf::cancel_pending();
                }
            }),
        },
        pen: photocraft_ui_egui::AndroidPenHooks {
            pen_section: Box::new(move |ui| {
                let mut settings = pen_settings
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .clone();
                let mut changed = false;
                craftapps_shared::pen::pen_settings_ui(ui, &mut settings, &mut || changed = true);
                if changed {
                    craftapps_shared::pen::sync_settings(&settings);
                    if let Ok(mut guard) = pen_settings.lock() {
                        *guard = settings.clone();
                    }
                    if let Err(e) = settings.save(&pen_config_dir) {
                        log::error!("pen settings save: {e}");
                    }
                }
            }),
        },
    });
}


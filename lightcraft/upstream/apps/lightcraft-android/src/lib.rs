//! LightCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`LightcraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! Pen pressure is free: winit's Android backend forwards `MotionEvent`
//! pressure as `egui::Event::Touch { force }`, which the brush tools read with
//! no cfg gates. Tilt, twist and the eraser end are dropped by winit 0.30:
//! v1.1 work.
//!
//! Workspace rules (see `upstreams/lightcraft/AGENTS.md`): this crate cannot
//! inherit the workspace lints because `unsafe_code = "deny"` would rule out
//! the JNI this shell fundamentally needs. It denies
//! unwrap/expect/panic/todo like every other crate, and every `unsafe` block
//! carries a SAFETY comment (the photocraft-tablet pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod services;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, saf, storage, updater};
use lightcraft_engine::Session;
use lightcraft_ui_egui::LightcraftApp;
use lightcraft_ui_egui::panels::library_problem::LibraryProblem;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::file_browser::FileBrowser;
use crate::shell::{AndroidShell, PrefsWriter, StorageGate};

/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.5.0-port1` → 50001).
pub const VERSION_NAME: &str = "0.2.0";

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]).
///
/// LightCraft composites its photos as egui textures through its own engine
/// GPU stack (`lightcraft_engine::gpu`) — there is no egui-wgpu paint-callback
/// canvas to hand the [`RenderState`][eframe::egui_wgpu::RenderState] to, so
/// `set_gpu_canvas` is a deliberate no-op. The cascade still matters: it
/// orders the window's adapter pick Vulkan-first (and the Performance prefs
/// keep working through the engine's own GPU switch).
impl gpu::GpuCanvasHost for AndroidShell {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        log::info!("LightCraft Android: window adapter: {} ({:?}, {})", info.name, info.backend, info.device_type as u8);
    }
    fn record_software_fallback(&mut self, note: &str) {
        log::info!("LightCraft Android: {note}");
    }
    fn set_gpu_canvas(&mut self, _rs: eframe::egui_wgpu::RenderState) {
        // Deliberate no-op: LightCraft has no GPU canvas to attach (see above).
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        // The window still draws with wgpu: log device errors instead of
        // letting the default handler's panic policy apply.
        device.on_uncaptured_error(std::sync::Arc::new(|e: eframe::wgpu::Error| {
            log::error!("LightCraft Android: wgpu device error: {e}");
        }));
    }
}

/// The android-activity 0.6 `native-activity` entry point: exported, unmangled,
/// Rust ABI. Runs on a dedicated thread android-activity spawns per Activity.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("LightCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("LightCraft Android failed to start: {e}");
    }
}

/// Forward the Activity's `onActivityResult` to the shared SAF result
/// handler (the mangled name is resolved by the JVM on first call — no
/// explicit registration, so no class-loader pitfalls on the attach thread).
///
/// # Safety
/// The raw pointers must be the valid JNI args the JVM passed to this entry.
/// The shared handler catches panics itself, so none unwind into the JVM.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_lightcraft_android_LightCraftActivity_nativeOnActivityResult(
    env: *mut jni::sys::JNIEnv,
    this: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data: jni::sys::jobject,
) {
    // SAFETY: per the contract above; the handler confines the raw pointers
    // to this call.
    unsafe {
        saf::activity_result_ffi(env, this, request_code, result_code, data);
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
        storage::StorageManager::new(&cache_dir, &storage::StorageAppInfo { app_name: "LightCraft", settings_folder: "LightCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir. Every persisted file (ui.json, library, updater setting)
    // lives under it via this override.
    let (config, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("LightCraft");
        std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("LIGHTCRAFT_CONFIG_DIR", &config) };

    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir.
    let browser_root: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let browser = Arc::new(Mutex::new(FileBrowser::new(storage.clone(), browser_root)));
    let mut services = crate::services::android_services(browser.clone(), storage.clone());

    let updater = Arc::new(Mutex::new(updater::Updater::new(
        "LightCraft",
        "LightCraft-",
        VERSION_NAME,
        cache_dir,
        &config,
        "app.lightcraft.android.apkprovider",
    )));
    // TODO: Port wire_android_hooks to new APIs (AndroidHooks removed).
    // wire_android_hooks disabled
    let _ = (&mut services);

    // The library (catalog, originals, previews) lives in the config dir; a
    // library that can't be opened is never replaced silently (the window says
    // why, like the desktop's `library_problem`).
    let mut session = Session::new().with_fs().with_default_denoise_models().with_system_clock();
    session.face_models_dir = lightcraft_engine::config::default_face_models_dir();
    let library_dir = config.join("Library");
    let problem = match session.open_library(&library_dir, true) {
        Ok(_) => {
            log::info!("library {}: {} photos", library_dir.display(), session.catalog.len());
            None
        }
        Err(e) => {
            log::error!("can't open library {}: {e}", library_dir.display());
            Some(LibraryProblem::new(library_dir.to_string_lossy(), e.to_string()))
        }
    };

    // App settings (`ui.json`); a damaged file is set aside, never silently dropped.
    let prefs_path = config.join("ui.json");
    let (prefs, prefs_warning, keep_prefs_file) = crate::shell::load_prefs(&prefs_path);
    let mut inner = LightcraftApp::new(session, services);
    // Mobile memory: the default budget is a quarter of system RAM (up to
    // 1.5 GB) shared across the decode caches; a 50–100 MB RAW plus
    // thumbnails can push a tablet into lmkd territory. Clamp it for mobile.
    // (Also settable via LIGHTCRAFT_MEMORY_MB env, min 64 MB.)
    lightcraft_engine::memory::set_budget(384 << 20);
    if let Some(ui) = prefs {
        inner.ui = ui;
    }
    lightcraft_ui_egui::i18n::set_language(inner.ui.language);
    if let Some(problem) = problem {
        inner.library_problem = Some(problem);
    }
    inner.notices.extend(prefs_warning);
    let writer = PrefsWriter::new(
        Some(prefs_path),
        serde_json::to_vec_pretty(&inner.ui).unwrap_or_default(),
        inner.ui.settings.library_path.clone(),
        keep_prefs_file,
    );

    // GPU backend cascade (Vulkan → GLES → CPU canvas). The shared plan reads
    // the desktop prefs schema, which LightCraft doesn't use (its settings
    // live in `ui.json` as `settings.gpu`): the cascade default applies, and
    // LightCraft's own "Use the GPU for rendering" switch gates the canvas.
    let mut gpu_plan = gpu::plan(
        &config.join("ui.json"),
        &gpu::GpuAppInfo { app_name: "LightCraft", cpu_canvas_env: "LIGHTCRAFT_CPU_CANVAS" },
    );
    if !inner.ui.settings.gpu {
        gpu_plan.allow_gpu_canvas = false;
    }

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "LightCraft",
        options,
        Box::new(move |cc| {
            let mut shell = AndroidShell::new(inner, browser, updater, writer, gate);
            // Record the window's actual adapter from the real render state.
            gpu::attach_canvas(&mut shell, cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(shell) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Connect the Android-only shell hooks: Help → Check for Updates…, and
/// Settings › Storage (media location tablet/SD toggle + SD tree picker).
/// The menu item and the tab are registered Android-only, so these closures
/// only ever run on Android.
fn wire_android_hooks(
    services: &mut lightcraft_ui_egui::Services,
    updater: Arc<Mutex<updater::Updater>>,
    storage: Arc<storage::StorageManager>,
    granted: bool,
) {
    let label_storage = storage.clone();
    let tablet_storage = storage.clone();
    let sd_storage = storage.clone();
    services.android_hooks = Some(lightcraft_ui_egui::AndroidHooks {
        storage: lightcraft_ui_egui::AndroidStorageHooks {
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

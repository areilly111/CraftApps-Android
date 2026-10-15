//! DesignCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`DesignApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! Unlike PhotoCraft, DesignCraft renders its canvas on the CPU
//! (`designcraft-render`); the GPU only composites the eframe interface, so
//! the GPU cascade's canvas-attach step is a no-op here and `GpuCanvasHost`
//! just records the adapter for the log.
//!
//! Workspace rules (see `upstream/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "forbid"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod services;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, saf, storage, updater};
use designcraft_engine::Session;
use designcraft_ui_egui::DesignApp;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::file_browser::FileBrowser;
use crate::shell::{AndroidShell, StorageGate};
use storage::{StorageAppInfo, StorageManager};
use updater::Updater;

/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.1.0` → 100).
pub const VERSION_NAME: &str = "0.2.0";

/// Local newtype so the port can implement the shared [`gpu::GpuCanvasHost`]
/// trait: the orphan rule forbids implementing a foreign trait for a foreign
/// type (`DesignApp` lives in `designcraft-ui-egui`), so the impl goes on
/// this wrapper. Sibling ports need the same one-tuple newtype.
struct AppHost<'a> {
    app: &'a mut DesignApp,
}

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]): how the render
/// state reaches this port's app type. DesignCraft's canvas is CPU-rendered
/// (vello), so there is no GPU canvas to hand the render state to — the
/// attach step only records what the window runs on, mirroring the desktop
/// `main.rs` log line.
impl gpu::GpuCanvasHost for AppHost<'_> {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        log::info!(
            "graphics: {} on {:?} ({:?}; driver {} {})",
            info.name,
            info.backend,
            info.device_type,
            info.driver,
            info.driver_info
        );
    }
    fn record_software_fallback(&mut self, note: &str) {
        log::warn!("graphics: {note}");
        self.app.status(note);
    }
    fn set_gpu_canvas(&mut self, _rs: eframe::egui_wgpu::RenderState) {
        // No GPU canvas in DesignCraft: the canvas composites on the CPU and
        // the GPU only draws the eframe interface (same as the desktop).
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        // The window still draws with wgpu: log device errors instead of
        // letting them panic (the desktop has no such watcher; the port adds
        // one because a dead GPU on a tablet shouldn't take the session down).
        let device = device.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |e: eframe::wgpu::Error| {
            log::error!("wgpu device error: {e}");
        }));
    }
}

/// The android-activity 0.6 `native-activity` entry point: exported, unmangled,
/// Rust ABI. Runs on a dedicated thread android-activity spawns per Activity.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("DesignCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("DesignCraft Android failed to start: {e}");
    }
}

/// Read the persisted UI state and engine prefs, like the desktop's `load_prefs`.
fn load_prefs(app: &mut DesignApp, config: &std::path::Path) {
    if let Ok(bytes) = std::fs::read(config.join("ui.json"))
        && let Ok(ui) = serde_json::from_slice::<designcraft_ui_egui::UiState>(&bytes)
    {
        app.ui = ui;
    }
    if let Ok(bytes) = std::fs::read(config.join("prefs.json"))
        && let Ok(prefs) = serde_json::from_slice::<designcraft_engine::Prefs>(&bytes)
    {
        app.session.prefs = prefs;
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
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "DesignCraft", settings_folder: "DesignCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir (the v1 layout). Every persisted file (ui.json, prefs.json,
    // recovery, updater setting) lives under it.
    let (config, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("DesignCraft");
        for sub in [&dir, &dir.join("Recovery")] {
            std::fs::create_dir_all(sub).map_err(|e| format!("couldn't create {}: {e}", sub.display()))?;
        }
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };

    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir (the v1 layout).
    let fallback: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let browser = Arc::new(Mutex::new(FileBrowser::new(storage.clone(), fallback.clone())));
    let inbox = designcraft_ui_egui::Inbox::default();
    let mut services = crate::services::android_services(browser.clone(), inbox, storage.clone(), fallback);

    let updater = Arc::new(Mutex::new(Updater::new(
        "DesignCraft",
        "DesignCraft-",
        VERSION_NAME,
        cache_dir,
        &config,
        "app.designcraft.android.apkprovider",
    )));
    // TODO: Port wire_android_hooks to new APIs (AndroidHooks removed).
    // wire_android_hooks disabled
    let _ = (&mut services);

    // GPU backend cascade (Vulkan → GLES → CPU), mirroring the desktop's
    // backend choice. DesignCraft has no performance GPU prefs; the persisted
    // prefs.json simply carries no gpu keys, so the default cascade applies.
    let gpu_plan = gpu::plan(
        &config.join("prefs.json"),
        &gpu::GpuAppInfo { app_name: "DesignCraft", cpu_canvas_env: "DESIGNCRAFT_CPU_CANVAS" },
    );

    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "DesignCraft",
        options,
        Box::new(move |cc| {
            let mut session = Session::new();
            // Crash recovery: reopen what a previous run left unsaved, rooted
            // at the port's config dir (the desktop uses the XDG data dir).
            let recovery_dir = config.join("Recovery");
            session.recovery_dir = Some(recovery_dir);
            let recovered = session.execute("file.recovery.open", &serde_json::json!({})).ok();
            let mut inner = DesignApp::new(session, services);
            load_prefs(&mut inner, &config);
            // Attach the GPU info (or record the CPU fallback) from the
            // actual wgpu render state — the canvas itself stays on the CPU.
            gpu::attach_canvas(&mut AppHost { app: &mut inner }, cc.wgpu_render_state.clone(), &gpu_plan);
            if let Some(n) = recovered.as_ref().and_then(|r| r["opened"].as_array()).map(Vec::len).filter(|n| *n > 0) {
                inner.status(format!("Recovered {n} unsaved document{} from the last session.", if n == 1 { "" } else { "s" }));
            }
            Ok(Box::new(AndroidShell::new(inner, browser, updater, gate, config)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Connect the Android-only shell hooks: Help → Check for Updates…, and
/// Preferences › Storage (media location tablet/SD toggle + SD tree picker).
/// The menu item and the section are registered Android-only, so these
/// closures only ever run on Android. Sibling ports reuse this verbatim.
fn wire_android_hooks(
    services: &mut designcraft_ui_egui::Services,
    storage: Arc<StorageManager>,
    granted: bool,
) {
    let label_storage = storage.clone();
    let tablet_storage = storage.clone();
    let sd_storage = storage.clone();
    services.android_hooks = Some(designcraft_ui_egui::AndroidHooks {
        storage: designcraft_ui_egui::AndroidStorageHooks {
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

/// `DesignCraftActivity.nativeOnActivityResult`: forwards the SAF picker
/// result to the shared handler, which answers the pending file dialog or
/// folder choice. The JVM resolves this mangled name on first call; no
/// explicit registration, so no class-loader pitfalls on the attach thread.
///
/// # Safety
///
/// The raw pointers must be the valid JNI args the JVM passed to this entry;
/// the wrappers are confined to this call and never outlive the JNI frame.
/// Never unwinds into the JVM (the shared handler catches panics).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_designcraft_android_DesignCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: `activity_result_ffi`'s contract is exactly this entry's: the
    // JVM's valid JNI args, confined to this call.
    unsafe {
        saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

//! FilmCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`FilmcraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! FilmCraft has no desktop `Services` struct: file I/O goes through the
//! engine's [`filmcraft_engine::Services`] (implemented by
//! [`services::AndroidServices`]: plain paths on the filesystem, `content://`
//! URIs through SAF) and the UI's [`filmcraft_ui_egui::HostHooks`] pickers.
//! The pickers are asynchronous — the web app's pattern: a hook opens the
//! in-app browser and returns "no answer"; when the user confirms, the shell
//! runs the engine command directly. Nothing blocks the UI thread.
//!
//! Workspace rules (see `upstream/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "forbid"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod audio;
mod file_browser;
mod services;
mod shell;

use android_activity::AndroidApp;
use chrono::TimeZone;
use craftapps_shared::gpu;
use craftapps_shared::storage::{StorageAppInfo, StorageManager};
use craftapps_shared::updater::Updater;
use craftapps_shared::{jni_bridge, saf};
use filmcraft_engine::autosave::AutosaveConfig;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::file_browser::FileBrowser;
use crate::services::{install_hooks, AndroidServices, PortDirs};
use crate::shell::{attach_audio, AndroidShell, StorageGate};

/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.5.0-port1` → 50001).
pub const VERSION_NAME: &str = "0.1.0";

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]).
/// FilmCraft composites through [`FilmcraftApp::set_wgpu`] (which already
/// disables the GPU compositor on unsuitable adapters and falls back to the
/// CPU): the cascade still orders adapter selection (Vulkan first) for the
/// window, and the device is watched for errors instead of panicking.
///
/// A local newtype: the orphan rule forbids implementing the shared trait for
/// the upstream app type directly.
struct FcHost<'a>(&'a mut FilmcraftApp);

impl gpu::GpuCanvasHost for FcHost<'_> {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        // No adapter field on the app to record into (unlike the desktop's
        // Help › About, which is data-driven): keep it in the log for GPU bug
        // reports.
        log::info!("graphics adapter: {} ({:?}, {:?})", info.name.trim(), info.backend, info.device_type);
    }
    fn record_software_fallback(&mut self, note: &str) {
        self.0.ui.status = format!("Software rendering — {note}");
    }
    fn set_gpu_canvas(&mut self, rs: eframe::egui_wgpu::RenderState) {
        // `set_wgpu` checks the adapter itself (float render targets etc.),
        // logs GPU errors instead of panicking, and composites on the CPU
        // when the GPU can't.
        self.0.set_wgpu(rs);
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        // The window still draws with wgpu: watch the device for errors instead
        // of letting them panic (the desktop `main.rs` does the same).
        device.set_device_lost_callback(|reason, msg| {
            log::error!("graphics device lost: {reason:?}: {msg}");
        });
    }
}

/// `FilmCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call; `this` is the activity (used for the ContentResolver).
/// Forwards to the shared SAF result handler.
///
/// # Safety
///
/// Must only be called by the JVM with valid JNI arguments: `raw_env` is the
/// current thread's JNI environment, `this_raw` is the calling activity, and
/// `data_raw` is null or the intent's data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_filmcraft_android_FilmCraftActivity_nativeOnActivityResult(
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
    log::info!("FilmCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("FilmCraft Android failed to start: {e}");
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
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "FilmCraft", settings_folder: "FilmCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Data dir (preferences.json, auto-save journals, crash logs): the shared
    // settings folder when granted, else the app's private dir. The engine's
    // `default_data_dir` honours `FILMCRAFT_DATA_DIR` first.
    let (data_dir, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("FilmCraft");
        std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("FILMCRAFT_DATA_DIR", &data_dir) };

    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir.
    let media_root: PathBuf = if granted {
        jni_bridge::external_storage_root()
            .map(|r| PathBuf::from(r).join("ArtCraft/Media"))
            .unwrap_or_else(|e| {
                log::warn!("no external storage root ({e}); browsing the files dir");
                files_dir.clone()
            })
    } else {
        files_dir.clone()
    };
    let browser_root: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let browser = Arc::new(Mutex::new(
        FileBrowser::new(storage.clone(), browser_root).map_err(|e| format!("file browser: {e}"))?,
    ));
    // Deterministic save/export targets under the shared media folder (the
    // web app's `/exports/<name>` pattern).
    let port_dirs = PortDirs::new(&media_root);
    port_dirs.ensure()?;

    let engine_services = AndroidServices::new(media_root);
    let mut session = Session::new(engine_services);
    let mut autosave_cfg = AutosaveConfig::new(&data_dir);
    autosave_cfg.local_offset = local_offset;
    if let Err(e) = session.start_autosave(autosave_cfg) {
        log::warn!("auto-save and crash recovery unavailable: {e}");
    }

    let mut inner = FilmcraftApp::new(session);
    // Mobile memory: the desktop cache budgets (768 MB decoded frames + 1 GB
    // GPU plans) OOM-kill the app on Android during preview playback (logcat:
    // lmkd reclaimed the app at 1.9 GB RSS). Clamp both to mobile sizes before
    // the UI starts. 128 MB of frames covers the 14-frame prefetch window at
    // 1080p RGBA (~8 MB/frame); the plan cache holds render plans, not pixels.
    // Also clamp the saved preference so a desktop-sized value in an existing
    // preferences.json can't re-raise the budget on the first apply_prefs.
    const MOBILE_CACHE_MB: u32 = 128;
    inner.session.prefs.memory.frame_cache_mb =
        inner.session.prefs.memory.frame_cache_mb.min(MOBILE_CACHE_MB);
    inner.frames.set_cache_budget(MOBILE_CACHE_MB as usize * (1 << 20));
    inner.frames.set_plan_budget(MOBILE_CACHE_MB as usize * (1 << 20));
    // Hardware video decoders (MediaCodec) are not wired on Android yet:
    // `filmcraft_platform::register()` reports Unavailable there and the
    // pure-Rust decoders run. Voice-over input stays off (no mic capture).
    attach_audio(&mut inner);
    install_hooks(&mut inner, browser.clone(), cache_dir.clone(), port_dirs);

    let updater = Arc::new(Mutex::new(Updater::new(
        "FilmCraft",
        "FilmCraft-",
        VERSION_NAME,
        cache_dir,
        &data_dir,
        "app.filmcraft.android.apkprovider",
    )));
    wire_android_hooks(&mut inner.hooks, updater.clone(), storage, granted);

    // GPU backend cascade (Vulkan → GLES → CPU canvas). The shared cascade
    // reads the PhotoCraft-style `performance.gpuBackend` pref; until a
    // mapping exists the port runs the default cascade. `FILMCRAFT_CPU_COMPOSITE`
    // (the desktop's env) forces the CPU canvas.
    let gpu_plan = gpu::plan(
        &data_dir.join("preferences.json"),
        &gpu::GpuAppInfo { app_name: "FilmCraft", cpu_canvas_env: "FILMCRAFT_CPU_COMPOSITE" },
    );

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "FilmCraft",
        options,
        Box::new(move |cc| {
            // Attach the wgpu render state: enables the GPU compositor when
            // the adapter suits it (or the CPU fallback) from the actual device.
            gpu::attach_canvas(&mut FcHost(&mut inner), cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(AndroidShell::new(inner, browser, updater, gate)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Connect the Android-only shell hooks: Help → Check for Updates…, and
/// Preferences › Storage (media location tablet/SD toggle + SD tree picker).
/// The menu item and the section are registered Android-only, so these
/// closures only ever run on Android. Sibling ports reuse this verbatim.
fn wire_android_hooks(
    hooks: &mut filmcraft_ui_egui::HostHooks,
    updater: Arc<Mutex<Updater>>,
    storage: Arc<StorageManager>,
    granted: bool,
) {
    let label_storage = storage.clone();
    let tablet_storage = storage.clone();
    let sd_storage = storage.clone();
    hooks.android_hooks = Some(filmcraft_ui_egui::AndroidHooks {
        check_updates: Box::new(move || {
            updater.lock().unwrap_or_else(|poison| poison.into_inner()).open();
        }),
        storage: filmcraft_ui_egui::AndroidStorageHooks {
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
            pick_sd_card: Box::new(move || {
                // The folder picker needs no permission: the chosen tree is
                // read and written through SAF directly.
                saf::set_pending_tree(sd_storage.clone());
                if let Err(e) = saf::launch_open_document_tree() {
                    log::error!("couldn't open the folder picker: {e}");
                    saf::cancel_pending();
                }
            }),
        },
    });
}

/// Local UTC offset at a unix time (auto-save file names and recovery times use local time).
fn local_offset(unix: i64) -> i32 {
    chrono::Local.timestamp_opt(unix, 0).single().map(|d| d.offset().local_minus_utc()).unwrap_or(0)
}

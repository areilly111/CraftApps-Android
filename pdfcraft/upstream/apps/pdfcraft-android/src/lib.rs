//! PdfCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`PdfCraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! File I/O (see `shell.rs`): PdfCraft has no `Services` struct to map, so the
//! app's file dialogs queue an [`AndroidPickRequest`][pdfcraft_ui_egui::AndroidPickRequest]
//! on Android (rfd has no Android backend) and the shell's [`FileBrowser`]
//! answers them; the answer runs the pick's own follow-up closure. Saves to a
//! SAF "save elsewhere" choice arrive as `content://` URIs and are routed
//! through the shell's SAF writer.
//!
//! Workspace rules (see `upstreams/pdfcraft/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "forbid"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::gpu;
use craftapps_shared::storage::{StorageAppInfo, StorageManager};
use craftapps_shared::updater::Updater;
use craftapps_shared::{jni_bridge, saf};
use pdfcraft_ui_egui::PdfCraftApp;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::file_browser::{FileBrowser, SafAnswers};
use crate::shell::{AndroidShell, StorageGate};

/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.5.0-port1` → 50001).
pub const VERSION_NAME: &str = "0.1.0";

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]).
/// PdfCraft renders pages on the CPU into egui textures (there is no GPU
/// canvas hook): the cascade still orders adapter selection (Vulkan first) for
/// the window, the adapter is logged for bug reports (the desktop `main.rs`
/// logs it the same way), the user is told about a software renderer (the
/// desktop's `notify_software_renderer`), and the device is watched for errors
/// instead of panicking.
///
/// A local newtype: the orphan rule forbids implementing the shared trait for
/// the upstream app type directly.
struct PcHost<'a>(&'a mut PdfCraftApp);

impl gpu::GpuCanvasHost for PcHost<'_> {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        log::info!(
            "PdfCraft Android: renderer: wgpu; adapter: {} ({:?}, {:?})",
            info.name.trim(),
            info.device_type,
            info.backend
        );
    }
    fn record_software_fallback(&mut self, note: &str) {
        // The desktop tells the user when wgpu draws on the processor; do the same.
        self.0.notify_tr(&format!("PdfCraft is drawing without a graphics processor, so it may be slow. {note}"));
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

/// `PdfCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call; `this` is the activity (used for the ContentResolver).
/// Forwards to the shared SAF result handler.
///
/// # Safety
///
/// Must only be called by the JVM with valid JNI arguments: `raw_env` is the
/// current thread's JNI environment, `this_raw` is the calling activity, and
/// `data_raw` is null or the intent's data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_pdfcraft_android_PdfCraftActivity_nativeOnActivityResult(
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
    log::info!("PdfCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("PdfCraft Android failed to start: {e}");
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
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "PdfCraft", settings_folder: "PdfCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir. `app.ron` (UI state + preferences), the updater setting and
    // the recovery folder all live under it.
    let (config, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("PdfCraft");
        for sub in [&dir, &dir.join("recovery")] {
            std::fs::create_dir_all(sub).map_err(|e| format!("couldn't create {}: {e}", sub.display()))?;
        }
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };
    let prefs_file = config.join("app.ron");

    // The in-app browser is rooted at the shared media location when granted,
    // else at the app's files dir.
    let browser_root: Option<PathBuf> = if granted { None } else { Some(files_dir.clone()) };
    let saf_answers: SafAnswers = Arc::new(Mutex::new(Vec::new()));
    let browser = Arc::new(Mutex::new(FileBrowser::new(
        storage.clone(),
        browser_root,
        saf_answers.clone(),
        cache_dir.clone(),
    )));

    let mut inner = PdfCraftApp::new();
    crate::shell::load_prefs(&mut inner, &prefs_file);
    // With the preference on, last session's files come back (the desktop's `main.rs`).
    inner.reopen_last_files(&[]);
    // Autosave unsaved changes; offer to recover documents a crashed session left behind.
    inner.enable_recovery(pdfcraft_ui_egui::RecoveryStore::new(config.join("recovery")));

    let updater = Arc::new(Mutex::new(Updater::new(
        "PdfCraft",
        "PdfCraft-",
        VERSION_NAME,
        cache_dir.clone(),
        &config,
        "app.pdfcraft.android.apkprovider",
    )));
    wire_android_hooks(&mut inner, updater.clone());

    // GPU backend cascade (Vulkan → GLES → CPU canvas).
    let gpu_plan = gpu::plan(
        &prefs_file,
        &gpu::GpuAppInfo { app_name: "PdfCraft", cpu_canvas_env: "PDFCRAFT_CPU_CANVAS" },
    );

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    eframe::run_native(
        "PdfCraft",
        options,
        Box::new(move |cc| {
            // Attach the wgpu render state: logs the adapter and watches the
            // device (or notes the CPU fallback) from the actual device.
            gpu::attach_canvas(&mut PcHost(&mut inner), cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(AndroidShell::new(
                inner,
                browser,
                saf_answers,
                updater,
                storage,
                granted,
                gate,
                prefs_file,
            )) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Connect the Android-only shell hooks: Help ▸ Check for updates…, links in
/// the system browser, and the SAF read/write hooks for `content://` answers.
/// Sibling ports reuse this verbatim; only the app type and the hook fields differ.
fn wire_android_hooks(app: &mut PdfCraftApp, updater: Arc<Mutex<Updater>>) {
    app.android_update_hook = Some(std::sync::Arc::new(move || {
        updater.lock().unwrap_or_else(|poison| poison.into_inner()).open();
    }));
    app.android_open_url = Some(std::sync::Arc::new(|url: &str| {
        let _ = jni_bridge::open_url(url);
    }));
    app.android_read_uri = Some(std::sync::Arc::new(|uri: &str| {
        // A document saved through the system picker earlier: read it back through SAF.
        saf::saf_read_uri(uri).map(|(_, bytes)| bytes)
    }));
    app.android_write_uri = Some(std::sync::Arc::new(|uri: &str, bytes: &[u8]| saf::saf_write_uri(uri, bytes)));
}

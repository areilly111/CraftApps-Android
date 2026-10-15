//! EffectCraft for Android (tablet).
//!
//! A `NativeActivity` shell around the shared egui app: [`android_main`]
//! (the android-activity 0.6 `native-activity` contract) builds the Android
//! platform services, then runs the same [`EffectcraftApp`] as the desktop
//! through `eframe::run_native` with `NativeOptions::android_app`.
//!
//! EffectCraft has no `Services` struct: the engine owns all file access
//! through [`Session::execute`] commands (`file.open`, `file.import`,
//! `file.saveAs`, …) on plain filesystem paths. This crate wires the app's
//! [`Hooks`](effectcraft_ui_egui::Hooks) file pickers to the in-app browser
//! (`file_browser.rs`): a hook opens the browser and reports "cancelled" for
//! that invocation (the hooks are synchronous, so they cannot block), and the
//! shell (`shell.rs`) reruns the matching engine command with the picked
//! paths once the browser answers. Save dialogs and folder picks get
//! immediate default paths under the app's documents folder. `content://`
//! URIs (the system picker's choices, or the SD-card location) are staged
//! into the cache as plain files the engine can read.
//!
//! Scripting (boa) and WebAssembly effect plug-ins (wasmi) come from
//! [`effectcraft_host::session`], exactly like the desktop: both runtimes are
//! pure Rust and check clean for `aarch64-linux-android`.
//!
//! Workspace rules (see `upstream/AGENTS.md`): this crate cannot inherit the
//! workspace lints because `unsafe_code = "deny"` would rule out the JNI
//! this shell fundamentally needs. It denies unwrap/expect/panic/todo like
//! every other crate, and every `unsafe` block carries a SAFETY comment (the
//! `photocraft-tablet` pattern).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod audio;
mod file_browser;
mod shell;

use android_activity::AndroidApp;
use craftapps_shared::gpu;
use craftapps_shared::storage::{MediaLocation, StorageAppInfo, StorageManager};
use craftapps_shared::updater::Updater;
use craftapps_shared::{jni_bridge, saf};
use effectcraft_ui_egui::{AndroidHooks, AndroidStorageHooks, EffectcraftApp};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::file_browser::{FileBrowser, PickKind};
use crate::shell::{AndroidShell, PendingPick, StorageGate};
/// Port version, shown in the UI and used by the updater. `build-apk.sh`
/// derives the Gradle `versionCode` from it (`0.7.0-port1` → 70001).
pub const VERSION_NAME: &str = "0.2.0";

/// Per-app glue for the shared GPU cascade ([`gpu::GpuCanvasHost`]).
/// EffectCraft's viewer composites on the GPU through its own `init_gpu`
/// (run from the app's `ui()`); the cascade still orders adapter selection
/// (Vulkan first) for the window, the adapter is recorded for Help ▸ System
/// Compatibility Report, and the device is watched for errors instead of
/// panicking.
///
/// A local newtype: the orphan rule forbids implementing the shared trait for
/// the upstream app type directly.
struct EcHost<'a>(&'a mut EffectcraftApp);

impl gpu::GpuCanvasHost for EcHost<'_> {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        // Help ▸ System Compatibility Report names this (the app's own
        // `init_gpu` records it too, in the same format).
        self.0.session.window_adapter = Some(format!(
            "{} ({:?}, {:?}{})",
            info.name.trim(),
            info.backend,
            info.device_type,
            if info.driver.is_empty() { String::new() } else { format!(", {} {}", info.driver, info.driver_info) }
        ));
    }
    fn record_software_fallback(&mut self, note: &str) {
        self.0.session.accel_note = Some(note.to_string());
    }
    fn set_gpu_canvas(&mut self, rs: eframe::egui_wgpu::RenderState) {
        self.watch_device(&rs.device);
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        // The window still draws with wgpu: watch the device for errors instead
        // of letting them panic (the desktop `main.rs` installs a failure
        // bridge on top of this; see `run`).
        device.set_device_lost_callback(|reason, msg| {
            log::error!("graphics device lost: {reason:?}: {msg}");
        });
    }
}

/// `EffectCraftActivity.nativeOnActivityResult`. The JVM resolves the mangled
/// name on first call; `this` is the activity (used for the ContentResolver).
/// Forwards to the shared SAF result handler.
///
/// # Safety
///
/// Must only be called by the JVM with valid JNI arguments: `raw_env` is the
/// current thread's JNI environment, `this_raw` is the calling activity, and
/// `data_raw` is null or the intent's data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_effectcraft_android_EffectCraftActivity_nativeOnActivityResult(
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
    log::info!("EffectCraft Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("EffectCraft Android failed to start: {e}");
    }
}

/// The desktop's import extensions (`file.import`'s `Ask::Import` filter).
/// The `pick_files` hook opens the browser for these; anything else is a
/// specialised pick the port answers as "cancelled" for now.
fn is_import_list(exts: &[&str]) -> bool {
    exts.contains(&"mp4") && exts.contains(&"png") && exts.contains(&"wav")
}

/// Open the in-app browser for `kind` and record what to rerun with its
/// answer. Called from the hook closures on the UI thread.
fn open_browser(
    browser: &Arc<Mutex<FileBrowser>>,
    pending: &Arc<Mutex<Option<PendingPick>>>,
    kind: PickKind,
    pick: PendingPick,
) {
    if let Ok(mut b) = browser.lock() {
        b.push(kind);
    }
    if let Ok(mut p) = pending.lock() {
        *p = Some(pick);
    }
}

/// A save dialog's default name → the engine command to rerun with the picked
/// path (and under which key). These are the `Ask::Save` defaults in the
/// frontend's `file_dialog`; anything else gets an immediate default path.
fn save_command(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "Untitled Project.ecproj" => ("file.saveAs", "path"),
        "Untitled Project.ecprojx" => ("file.saveCopyAsXml", "path"),
        "Frame.png" => ("comp.saveFrameAs", "path"),
        "Frame.psd" => ("comp.saveFrameAsPsd", "path"),
        "Frame.exr" => ("comp.saveFrameAsExr", "path"),
        "Preview.mp4" => ("render.saveCurrentPreview", "path"),
        "Animation.json" => ("file.exportLottie", "path"),
        "Preset.ecpreset" => ("anim.savePreset", "path"),
        "Guides.json" => ("view.exportGuides", "path"),
        "Template.ectemplate" => ("essential.exportTemplate", "path"),
        "Watch Folder" => ("file.watchFolder", "folder"),
        "Collected Files" => ("file.collectFiles", "folder"),
        _ => return None,
    })
}

/// Immediate default save path under the documents folder (used for save
/// dialogs the port doesn't browser-ify, and for folder picks).
fn default_path(docs: &Path, sub: &str, name: &str) -> String {
    let dir = docs.join(sub);
    let _ = std::fs::create_dir_all(&dir);
    let safe: String =
        name.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect();
    let safe = if safe.is_empty() { "file".to_string() } else { safe };
    dir.join(safe).to_string_lossy().into_owned()
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
        StorageManager::new(&cache_dir, &StorageAppInfo { app_name: "EffectCraft", settings_folder: "EffectCraft" })
            .map_err(|e| format!("storage: {e}"))?,
    );

    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    let choice_file = internal.join("storage-choice.json");
    let chose_internal = std::fs::read_to_string(&choice_file)
        .map(|text| text.contains("\"internal\""))
        .unwrap_or(false);

    // Config dir: the shared settings folder when granted, else the app's
    // private dir. Settings, shortcut presets, crash recovery and the Plug-ins
    // folder all live under it.
    let (config_base, gate): (PathBuf, Option<StorageGate>) = if granted {
        storage.ensure_dirs()?;
        (storage.settings_dir().to_path_buf(), None)
    } else {
        let dir = internal.join("EffectCraft");
        for sub in [&dir, &dir.join("Data Recovery")] {
            std::fs::create_dir_all(sub).map_err(|e| format!("couldn't create {}: {e}", sub.display()))?;
        }
        let gate = if chose_internal { None } else { Some(StorageGate::new(choice_file)) };
        (dir, gate)
    };
    // Point the platform config lookup at the Android config dir, so
    // `effectcraft_host::config_dir()` (settings, Plug-ins, recovery) lands
    // under it (`<config>/effectcraft/…`) instead of an Android home
    // directory.
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &config_base) };
    let config_dir = effectcraft_host::config_dir().ok_or("no config dir")?;
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("couldn't create {}: {e}", config_dir.display()))?;

    // Documents root for immediate default save paths: the shared media folder
    // when it is plain tablet storage, else the app's files dir (the SD-card
    // location holds document URIs, not paths).
    let docs_root = if granted && storage.media_location() != MediaLocation::SdCard {
        jni_bridge::external_storage_root()
            .map(|r| PathBuf::from(r).join("ArtCraft/Media"))
            .unwrap_or_else(|_| files_dir.clone())
    } else {
        files_dir.clone()
    };
    let docs_root = docs_root.join("EffectCraft");

    let browser = Arc::new(Mutex::new(
        FileBrowser::new(storage.clone(), if granted { None } else { Some(files_dir.clone()) }, cache_dir.clone())
            .map_err(|e| format!("file browser: {e}"))?,
    ));
    let pending: Arc<Mutex<Option<PendingPick>>> = Arc::new(Mutex::new(None));

    // A session with media, import, expressions, scripting (boa) and the
    // WebAssembly effect plug-ins (wasmi) enabled — the desktop's `host`
    // wiring, reused wholesale.
    let mut session = effectcraft_host::session();
    // Lazy open and non-blocking auto-save: footage is checked and auto-saves are
    // written on background threads (the desktop's main.rs).
    session.check_footage_on_open = true;
    session.autosave.background = true;
    // Settings, shortcut presets and the crash-recovery sentinel live in the
    // platform config directory.
    session.config = Some(std::sync::Arc::new(effectcraft_engine::config::DirConfig::new(&config_dir)));
    session.load_settings();
    // WebAssembly effect plug-ins in <config>/Plug-ins load before the UI starts.
    let plugins = config_dir.join("Plug-ins");
    if plugins.is_dir()
        && let Err(e) = session.execute("effect.plugins.load", json!({"folder": plugins.to_string_lossy()}))
    {
        log::warn!("couldn't load plug-ins from {}: {e}", plugins.display());
    }
    let recovery = session.begin_recovery();

    let mut inner = EffectcraftApp::new(session);
    // Mobile memory: the desktop cache budgets (3 GB RAM preview cache + 1 GB
    // GPU frames + 1 GB media pool) OOM-kill the app on Android during preview
    // playback. Clamp all three before the UI starts. The media pool budget is
    // already 128 MB on Android via target-gated DEFAULT_BUDGET.
    const MOBILE_CACHE_MB: usize = 256;
    inner.frames.set_budget(MOBILE_CACHE_MB << 20);
    inner.frames.set_budget(MOBILE_CACHE_MB << 20);
    if let Some(r) = recovery {
        inner.offer_recovery(r);
    }
    let ui_state_path = config_dir.join("ui.json");
    load_ui_state(&mut inner, &ui_state_path);

    let updater = Arc::new(Mutex::new(Updater::new(
        "EffectCraft",
        "EffectCraft-",
        VERSION_NAME,
        cache_dir,
        &config_base,
        "app.effectcraft.android.apkprovider",
    )));

    wire_hooks(&mut inner, browser.clone(), pending.clone(), storage.clone(), &docs_root, granted);

    // GPU backend cascade (Vulkan → GLES → CPU canvas). EffectCraft's viewer
    // composites on the GPU through its own `init_gpu`; the cascade orders
    // adapter selection for the window and records the fallback.
    let gpu_plan = gpu::plan(
        &config_dir.join("prefs.json"),
        &gpu::GpuAppInfo { app_name: "EffectCraft", cpu_canvas_env: "EFFECTCRAFT_CPU_CANVAS" },
    );

    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    let ui_state_path = config_dir.join("ui.json");
    eframe::run_native(
        "EffectCraft",
        options,
        Box::new(move |cc| {
            // This executable owns the shared device's handlers. Install them
            // before EffectCraft's compositor pipelines are built (the desktop
            // `main.rs` does the same).
            let gpu_failures = effectcraft_ui_egui::gpu_failure::GpuFailureBridge::new(&cc.egui_ctx);
            if let Some(rs) = &cc.wgpu_render_state {
                let errors = gpu_failures.clone();
                rs.device.on_uncaptured_error(std::sync::Arc::new(move |error| {
                    let message = format!("uncaptured GPU error: {error}");
                    if errors.report(&message, false) {
                        log::error!("{message}");
                    }
                }));
                let lost = gpu_failures.clone();
                rs.device.set_device_lost_callback(move |reason, message| {
                    lost.report(&format!("GPU device lost ({reason:?}): {message}"), true);
                });
            }
            inner.set_gpu_failure_bridge(gpu_failures);
            // Attach the wgpu render state: records the adapter for Help ▸
            // System Compatibility Report (or the CPU fallback) from the
            // actual device, and watches it for errors.
            let mut host = EcHost(&mut inner);
            gpu::attach_canvas(&mut host, cc.wgpu_render_state.clone(), &gpu_plan);
            Ok(Box::new(AndroidShell::new(inner, browser, pending, updater, gate, ui_state_path)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe failed to run: {e}"))
}

/// Restore the saved UI state (dock layout, panel state), best effort.
fn load_ui_state(app: &mut EffectcraftApp, ui_file: &Path) {
    if let Ok(bytes) = std::fs::read(ui_file)
        && let Ok(ui) = serde_json::from_slice::<effectcraft_ui_egui::state::UiState>(&bytes)
    {
        app.ui = ui;
    }
}

/// Wire the app's host hooks to the Android shell: the file pickers open the
/// in-app browser (or answer immediate default paths), and the Android-only
/// hooks reach the updater and the storage settings.
fn wire_hooks(
    app: &mut EffectcraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    pending: Arc<Mutex<Option<PendingPick>>>,
    storage: Arc<StorageManager>,
    docs_root: &Path,
    granted: bool,
) {
    let open_b = browser.clone();
    let open_p = pending.clone();

    // File ▸ Open Project…: the browser answers, the shell reruns `file.open`.
    let (b, p) = (open_b.clone(), open_p.clone());
    app.hooks.pick_open_project = Some(Box::new(move || {
        open_browser(&b, &p, PickKind::OpenProject, PendingPick::OpenProject);
        None
    }));

    // File ▸ Import… (and Replace Footage): the browser answers with the
    // picked files, the shell reruns `file.import`. Specialised picks (model
    // installers, preset browsers, …) report "cancelled" for now.
    let (b, p) = (open_b.clone(), open_p.clone());
    app.hooks.pick_files = Some(Box::new(move |exts: &[&str]| {
        if is_import_list(exts) {
            open_browser(
                &b,
                &p,
                PickKind::ImportFiles { exts: exts.iter().map(|s| s.to_string()).collect() },
                PendingPick::ImportFiles,
            );
        }
        Vec::new()
    }));

    // File ▸ Save / Save As…: the browser asks where, the shell reruns
    // `file.saveAs`. Anything else gets an immediate default path.
    let (b, p) = (open_b.clone(), open_p.clone());
    let docs = docs_root.to_path_buf();
    app.hooks.pick_save = Some(Box::new(move |name: &str| {
        if name.eq_ignore_ascii_case("Untitled Project.ecproj") {
            open_browser(
                &b,
                &p,
                PickKind::SaveProject { default: name.to_string() },
                PendingPick::SaveAs { command: "file.saveAs", key: "path" },
            );
            None
        } else {
            Some(default_path(&docs, "Projects", name))
        }
    }));

    // The other save dialogs (frame, preview, Lottie, presets, templates,
    // collect-files, watch folder): known defaults open the browser and the
    // shell reruns the matching command; direct callers (Render Queue output,
    // shortcut presets) get an immediate default path and handle it
    // themselves.
    let (b, p) = (open_b, open_p);
    let docs = docs_root.to_path_buf();
    app.hooks.pick_save_file = Some(Box::new(move |name: &str, _ext: &str| {
        match save_command(name) {
            Some((command, key)) => {
                open_browser(&b, &p, PickKind::SaveFile { default: name.to_string() }, PendingPick::SaveAs { command, key });
                None
            }
            None => Some(default_path(&docs, "Exports", name)),
        }
    }));

    // Folder picks (Settings paths): an immediate default folder.
    let docs = docs_root.to_path_buf();
    app.hooks.pick_folder = Some(Box::new(move || Some(docs.to_string_lossy().into_owned())));

    // Preview audio: cpal on the AAudio backend. When no output device opens,
    // the factory returns None and preview runs silent instead of failing.
    app.hooks.audio_device = Some(crate::audio::android_audio_factory());

    // Android-only shell hooks: Help ▸ Check for Updates…, and Settings ▸
    // Storage (media location tablet/SD toggle + SD tree picker).
    let label_storage = storage.clone();
    let tablet_storage = storage.clone();
    let sd_storage = storage.clone();
    app.hooks.android_hooks = Some(AndroidHooks {
        storage: AndroidStorageHooks {
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
        pen: None,
    });
}

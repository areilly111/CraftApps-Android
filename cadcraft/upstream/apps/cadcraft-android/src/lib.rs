//! CADCraft for Android (tablet).
//!
//! A NativeActivity shell around [`cadcraft_ui_egui::CadApp`]:
//! shared-storage dirs, SAF pickers through the file bridge (the app's
//! synchronous `Services` hooks, see `services.rs`), the stylus
//! side-channel, preference persistence in the suite settings folder, and
//! the Vulkan → GLES → CPU GPU cascade. File I/O hooks are the desktop
//! host's (`cadcraft_io` over the engine's `cmd::file` hooks); the control
//! server, CLI arguments, SpaceMouse, native menu, desktop theme, system
//! animations and drag-and-drop are desktop-only and are not ported.
//!
//! This crate does not opt into the workspace `unsafe_code = "forbid"`
//! lint: the JNI entries an Android shell needs are unsafe. It denies
//! unwrap/expect/panic/todo like every other port crate, and every unsafe
//! block carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod services;
mod shell;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use android_activity::AndroidApp;
use cadcraft_engine::Session;
use cadcraft_ui_egui::{CadApp, UiState};
use craftapps_shared::{gpu, jni_bridge, pen, storage::StorageAppInfo, storage::StorageManager};

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it (`0.1.0` → 100100).
pub const VERSION_NAME: &str = "0.1.0";

const APP_NAME: &str = "CadCraft";

/// Stencil bits of the window's render pass (the desktop host's value):
/// a stencil buffer clips the contents of clipped layout viewports on
/// the GPU.
const STENCIL_BITS: u8 = 8;

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    log::info!("{APP_NAME} Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("{APP_NAME} Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    jni_bridge::init()?;

    let cache_dir = PathBuf::from(jni_bridge::cache_dir().map_err(|e| format!("cache dir: {e}"))?);

    // Shared storage needs "All files access". Nudge the user to the settings
    // page when it isn't granted yet; the app still starts (pickers work via
    // the app-private bridge dir regardless).
    let granted = jni_bridge::is_external_storage_manager().unwrap_or(false);
    if !granted {
        log::info!("all-files access not granted; opening settings");
        let _ = jni_bridge::open_all_files_settings();
    }
    let storage = StorageManager::new(
        &cache_dir,
        &StorageAppInfo { app_name: APP_NAME, settings_folder: "CadCraft" },
    )
    .map(Arc::new)
    .map_err(|e| {
        log::warn!("shared storage unavailable ({e}); using app-local fallback");
        e
    })
    .ok();
    if let Some(s) = &storage {
        let _ = s.ensure_dirs();
    }

    // Suite media root for the save fallback in services.rs.
    let media_dir = storage
        .as_ref()
        .map(|s| s.media_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.join("media"));
    let _ = std::fs::create_dir_all(&media_dir);

    // CADCraft's settings folder: `workspace.json`, `profile.json`,
    // `prefs.json`, `autosave/` and `support/` live here (the desktop
    // layout, rooted in the suite settings folder).
    let settings_dir = storage
        .as_ref()
        .map(|s| s.settings_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.clone());
    let _ = std::fs::create_dir_all(&settings_dir);

    // Stylus: load the pressure curve / button map when the user saved one.
    let pen_settings = pen::PenSettings::load(&settings_dir);
    pen::sync_settings(&pen_settings);

    // The desktop resolves its settings folder from this variable (its
    // documented override); point it at the suite settings folder so any
    // upstream reader lands in the same place.
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("CADCRAFT_CONFIG_DIR", &settings_dir) };

    // The file picker bridge lives in the app's own external files dir:
    // always writable, and no "All files access" permission needed. The Java
    // side reads the same path (`getExternalFilesDir(null)/.picker`).
    let bridge_base = app.external_data_path().unwrap_or_else(|| cache_dir.clone());
    let bridge_dir = bridge_base.join(".picker");
    let _ = std::fs::create_dir_all(&bridge_dir);
    // SAFETY: same as above — no other Rust thread exists yet.
    unsafe { std::env::set_var("CRAFTAPPS_BRIDGE_DIR", &bridge_dir) };

    // File format hooks for the engine (it stays I/O-agnostic), as the
    // desktop host installs them, and the engine's settings directory.
    install_io();
    cadcraft_engine::settings_dir::set(Some(settings_dir.clone()));

    // Shared GPU cascade (Vulkan → GLES → CPU). The plan reads the
    // `performance.*` keys of the prefs file when one exists; a missing
    // file means Auto/Auto — the default cascade.
    let prefs_path = settings_dir.join("prefs.json");
    let gpu_plan = gpu::plan(
        &prefs_path,
        &gpu::GpuAppInfo { app_name: APP_NAME, cpu_canvas_env: "CADCRAFT_CPU_CANVAS" },
    );
    let allow_gpu_canvas = gpu_plan.allow_gpu_canvas;
    let mut options = eframe::NativeOptions {
        android_app: Some(app),
        stencil_buffer: STENCIL_BITS,
        ..Default::default()
    };
    gpu::configure(&mut options, &gpu_plan);

    let workspace_path = settings_dir.join("workspace.json");
    let profile_path = settings_dir.join("profile.json");
    let autosave_prefs_path = settings_dir.join("autosave-prefs.json");
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            let mut app = CadApp::new(Session::empty(), services::build_services(storage.clone(), media_dir.clone()));
            // The desktop restores the workspace first, then the profile,
            // then the interface prefs (each later layer wins its fields).
            if let Some(ui) = load_workspace(&workspace_path) {
                app.ui = ui;
            }
            if let Some(profile) = read_file(&profile_path) {
                app.load_profile(&profile);
            }
            if let Some(prefs) = read_file(&prefs_path) {
                app.load_prefs(&prefs);
            }
            if let Some(prefs) = read_file(&autosave_prefs_path) {
                cadcraft_ui_egui::autosave::load_prefs(&prefs);
            }
            // Autosave files (SAVEFILEPATH) default to `<settings dir>/autosave`.
            app.enable_autosave(&settings_dir.join("autosave"));
            // The startup update check is NOT enabled: updates_ui is a
            // silent no-op on Android (see the ANDROID-PORT note there).
            // Startup LISP (acad.lsp, acaddoc.lsp) is read from
            // `<settings>/support`; the folder is made so it is there to
            // put them in.
            let support = settings_dir.join("support");
            if let Err(e) = std::fs::create_dir_all(&support) {
                log::warn!("no LISP support folder {}: {e}", support.display());
            }
            app.session.set_lisp_support_dir(Some(support));
            // GPU canvas on the eframe wgpu device (the desktop does the
            // same through `set_wgpu`); skipped for a software adapter or
            // when the plan says CPU, in which case the canvas composites
            // on the CPU.
            if allow_gpu_canvas {
                if let Some(rs) = &cc.wgpu_render_state {
                    if rs.adapter.get_info().device_type != eframe::wgpu::DeviceType::Cpu {
                        app.set_wgpu(rs, eframe::egui_wgpu::depth_format_from_bits(0, STENCIL_BITS));
                    }
                }
            }
            if app.session.docs.is_empty() {
                // In the units the profile asks for (MEASUREINIT, restored above).
                let metric = app.session.settings.measureinit;
                app.session.new_drawing(metric);
            }
            Ok(Box::new(shell::ShellApp::new(
                app,
                workspace_path.clone(),
                profile_path.clone(),
                prefs_path.clone(),
                autosave_prefs_path.clone(),
            )) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

/// File format hooks for the engine (it stays I/O-agnostic): the desktop
/// host's wiring, unchanged.
fn install_io() {
    cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    cadcraft_engine::cmd::file::set_write_as(|d, name, o| {
        cadcraft_io::write_with(d, name, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary }).map_err(|e| e.to_string())
    });
    // AutoLISP's entget, entmake and entmod see entities as DXF groups.
    cadcraft_engine::cmd::file::set_entity_codec(cadcraft_engine::cmd::file::EntityCodec {
        groups: cadcraft_io::entity_groups,
        entity: cadcraft_io::entity_from_groups,
    });
}

/// Read a small text file; a missing file yields `None` (defaults), any
/// other failure is logged and also yields `None`.
fn read_file(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            log::warn!("settings could not be read from {}: {e}", path.display());
            None
        }
    }
}

/// Read the saved workspace (`workspace.json` in the suite settings
/// folder). This is the port's own plain-`UiState` JSON, not the desktop
/// host's versioned `{version, workspace}` wrapper: the Android settings
/// folder is never shared with a desktop install, so the wrapper buys
/// nothing here. An unreadable or malformed file yields `None` (defaults)
/// with a warning.
fn load_workspace(path: &Path) -> Option<UiState> {
    let text = read_file(path)?;
    match serde_json::from_str::<UiState>(&text) {
        Ok(mut ui) => {
            // A restored file must never reopen a dialog or replace the
            // active drawing view (the desktop reader enforces the same).
            ui.dialog = None;
            ui.start_tab = false;
            Some(ui)
        }
        Err(e) => {
            log::warn!("workspace could not be restored: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// JNI entries. The JVM resolves these mangled names on first call; each port
// defines its own set forwarding to the shared handlers.

/// `<Activity>.nativeOnActivityResult`: forwards SAF picker results.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_cadcraft_android_CadCraftActivity_nativeOnActivityResult(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // SAFETY: the JVM calls this with the valid JNI args it was given; the
    // shared handler never unwinds into the JVM.
    unsafe {
        craftapps_shared::saf::activity_result_ffi(raw_env, this_raw, request_code, result_code, data_raw);
    }
}

/// `<Activity>.nativeOnPenEvent`: the stylus side-channel (tool type, barrel
/// buttons) that winit's Android backend drops.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_cadcraft_android_CadCraftActivity_nativeOnPenEvent(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    action: jni::sys::jint,
    tool: jni::sys::jint,
    buttons: jni::sys::jint,
    action_button: jni::sys::jint,
    x: jni::sys::jfloat,
    y: jni::sys::jfloat,
) {
    // handle_pen_event catches unwinds itself; plain integers/floats cross the ABI.
    pen::handle_pen_event(action, tool, buttons, action_button, x, y);
}

/// `<Activity>.nativeOnModifierState`: the Bluetooth-keyboard modifier
/// bitmask the Activity's `dispatchKeyEvent` tracks (bit 0 = Ctrl,
/// 1 = Shift, 2 = Alt). On a `NativeActivity` that Java hook never runs —
/// the live path is `craftapps_shared::modifiers`, which tracks the same
/// keys from egui's key events — so this only stores the value, for parity
/// with the other ports.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_cadcraft_android_CadCraftActivity_nativeOnModifierState(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    modifiers: jni::sys::jint,
) {
    use std::sync::atomic::{AtomicI32, Ordering};
    static MODIFIER_STATE: AtomicI32 = AtomicI32::new(0);
    MODIFIER_STATE.store(modifiers, Ordering::SeqCst);
}

//! EffectCraft for Android (tablet).
//!
//! A NativeActivity shell around [`effectcraft_ui_egui::EffectcraftApp`]:
//! shared-storage dirs, SAF pickers through the file bridge (the app's
//! `file_dialog_async` hook and the synchronous pick hooks), the stylus
//! side-channel, and the Vulkan → GLES → CPU GPU cascade.
//!
//! This crate does not opt into the workspace `unsafe_code = "forbid"` lint:
//! the JNI entries an Android shell needs are unsafe. It denies
//! unwrap/expect/panic/todo like every other port crate, and every unsafe
//! block carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod audio;
mod picks;
mod shell;

use std::path::PathBuf;
use std::sync::Arc;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge, pen, storage::StorageAppInfo, storage::StorageManager};
use effectcraft_ui_egui::EffectcraftApp;
use serde_json::json;

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from it (`0.1.0` → 100100).
pub const VERSION_NAME: &str = "0.1.1";

const APP_NAME: &str = "EffectCraft";

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    // The engine's own logger (stderr + the session log file once attached)
    // and its panic hook, as the desktop installs them. Records logged
    // before the session log is attached are written to it first.
    effectcraft_engine::logging::install();
    effectcraft_engine::logging::install_panic_hook();
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
        &StorageAppInfo { app_name: APP_NAME, settings_folder: "EffectCraft" },
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

    // Suite media root for the save fallback in picks.rs.
    let media_dir = storage
        .as_ref()
        .map(|s| s.media_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.join("media"));
    let _ = std::fs::create_dir_all(&media_dir);

    // Stylus: load the pressure curve / button map when the user saved one.
    let settings_dir = storage
        .as_ref()
        .map(|s| s.settings_dir().to_path_buf())
        .unwrap_or_else(|| cache_dir.clone());
    let pen_settings = pen::PenSettings::load(&settings_dir);
    pen::sync_settings(&pen_settings);

    // EffectCraft keeps its settings, models, plug-ins and crash recovery in
    // one config folder; the host crate reads it from this variable (its
    // documented override), so point it at the suite settings folder.
    // SAFETY: `android_main` runs before the eframe UI thread (or any other
    // Rust thread) exists, so no other thread can read the environment
    // concurrently. (Edition 2024 made `set_var` unsafe for data races.)
    unsafe { std::env::set_var("EFFECTCRAFT_CONFIG_DIR", &settings_dir) };
    // The session log (Logs/effectcraft.log in the settings folder), as the
    // desktop attaches it.
    match effectcraft_engine::logging::attach_session_log(
        &effectcraft_engine::logging::log_dir(Some(&settings_dir)),
    ) {
        Ok(path) => log::info!("{APP_NAME} {VERSION_NAME}, log file {}", path.display()),
        Err(e) => log::warn!("no log file: {e}"),
    }

    // The font menus list the installed fonts: read their names in the
    // background while the window opens (the desktop does the same).
    effectcraft_engine::text::fonts::scan_system_in_background();

    // The file picker bridge lives in the app's own external files dir:
    // always writable, and no "All files access" permission needed. The Java
    // side reads the same path (`getExternalFilesDir(null)/.picker`).
    let bridge_base = app.external_data_path().unwrap_or_else(|| cache_dir.clone());
    let bridge_dir = bridge_base.join(".picker");
    let _ = std::fs::create_dir_all(&bridge_dir);
    // SAFETY: same as above — no other Rust thread exists yet.
    unsafe { std::env::set_var("CRAFTAPPS_BRIDGE_DIR", &bridge_dir) };

    // Shared GPU cascade (Vulkan → GLES → CPU).
    let gpu_plan = gpu::plan(
        &settings_dir.join("prefs.json"),
        &gpu::GpuAppInfo { app_name: APP_NAME, cpu_canvas_env: "EFFECTCRAFT_CPU_CANVAS" },
    );
    let allow_gpu = gpu_plan.allow_gpu_canvas;
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);
    // The window's wgpu device asks for the adapter's own limits (textures up
    // to 16384 px) instead of eframe's fixed set, as the desktop configures
    // it: an older GPU that can't meet those still opens the app, and the
    // compositor then falls back to the CPU when the device can't run it.
    options.wgpu_options = wgpu_options();

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            // The GPU failure bridge: uncaptured wgpu errors and device loss
            // are reported to the app (the desktop wires the same handlers).
            let gpu_failures = effectcraft_ui_egui::gpu_failure::GpuFailureBridge::new(&cc.egui_ctx);
            if let Some(rs) = &cc.wgpu_render_state {
                let errors = gpu_failures.clone();
                rs.device.on_uncaptured_error(Arc::new(move |error| {
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

            let mut session = effectcraft_host::session();
            // Lazy open and non-blocking auto-save: footage is checked and
            // auto-saves are written on background threads (the desktop sets
            // the same).
            session.check_footage_on_open = true;
            session.autosave.background = true;
            // Settings, shortcut presets and the crash-recovery sentinel live
            // in the config directory (the suite settings folder, above).
            if let Some(dir) = effectcraft_host::config_dir() {
                // WebAssembly effect plug-ins in <config>/Plug-ins load before
                // the menus are built.
                let plugins = dir.join("Plug-ins");
                if plugins.is_dir() {
                    let _ = session.execute("effect.plugins.load", json!({"folder": plugins.to_string_lossy()}));
                }
                session.config = Some(Arc::new(effectcraft_engine::config::DirConfig::new(dir)));
            }
            session.load_settings();
            let recovery = session.begin_recovery();
            // First run (no project yet): the app opens an empty Untitled
            // Project, or its Home screen when the Startup preference says so.
            let show_home = session.prefs.startup.show_home_on_launch;

            let mut app = EffectcraftApp::new(session);
            app.set_gpu_failure_bridge(gpu_failures);
            app.ui.start_screen = show_home;
            if let Some(r) = recovery {
                app.offer_recovery(r);
            }
            if !allow_gpu {
                log::info!("GPU canvas disabled by the suite GPU plan; the CPU compositor runs");
            }

            // File dialogs via the shared file bridge (see picks.rs): the
            // non-blocking hook serves the commands' dialogs (File ▸ Import,
            // Open, Save As…), the synchronous hooks serve the panels that
            // call them directly (Render Queue output, Settings paths…).
            let media = Arc::new(media_dir.clone());
            let media_async = media.clone();
            app.hooks.file_dialog_async = Some(Box::new(move |dialog, done| {
                picks::run_dialog_async(dialog, done, media_async.clone());
            }));
            app.hooks.pick_files = Some(Box::new(|exts: &[&str]| picks::pick_files(exts)));
            let media_save = media.clone();
            app.hooks.pick_save = Some(Box::new(move |name: &str| {
                picks::pick_save_project(name, &media_save)
            }));
            app.hooks.pick_open_project = Some(Box::new(picks::pick_open_project));
            app.hooks.pick_folder = Some(Box::new(picks::pick_folder));
            let media_save_file = media.clone();
            app.hooks.pick_save_file = Some(Box::new(move |name: &str, ext: &str| {
                picks::pick_save_file(name, ext, &media_save_file)
            }));
            // Audio preview through cpal's AAudio backend (see audio.rs).
            app.hooks.audio_device = Some(Box::new(audio::open));
            app.hooks.audio_devices = Some(Box::new(audio::devices));
            // No native dialog on Android for a lost graphics device: log it
            // (the app closes itself; crash recovery covers the work).
            app.hooks.device_lost = Some(Box::new(|saved: Option<Result<String, String>>| {
                match saved {
                    Some(Ok(path)) => log::error!("graphics device lost; unsaved changes saved to {path}"),
                    Some(Err(e)) => log::error!("graphics device lost; unsaved changes could not be saved ({e})"),
                    None => log::error!("graphics device lost"),
                }
            }));
            app.hooks.clipboard_text = Some(Box::new(clipboard_text));
            Ok(Box::new(shell::ShellApp::new(app)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

/// The desktop's wgpu configuration: the adapter's own limits (textures
/// capped at 16384 px like the headless renderer's device).
fn wgpu_options() -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::WgpuSetup;
    let mut config = eframe::WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        setup.device_descriptor = Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
            label: Some("EffectCraft device"),
            required_limits: eframe::wgpu::Limits {
                max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(16384),
                ..adapter.limits()
            },
            ..Default::default()
        });
    }
    config
}

/// The system clipboard's text via Android's `ClipboardManager` (the
/// `clipboard_text` hook: Edit ▸ Paste into a text field).
///
/// Best effort: any failure (no clipboard, no primary clip, JNI error)
/// yields `None` rather than an error — the caller treats it as "no text".
fn clipboard_text() -> Option<String> {
    use jni::objects::JObject;
    // SAFETY: ndk-context hands us the JVM pointer the platform gave this
    // process; `from_raw` only wraps it.
    let vm = unsafe { jni::JavaVM::from_raw(ndk_context::android_context().vm().cast()) };
    // jni 0.22 attaches via a callback that receives the Env; the callback's
    // error type must convert from jni::errors::Error.
    let attached: Result<Option<String>, jni::errors::Error> =
        vm.attach_current_thread(|env: &mut jni::Env| {
            // SAFETY: the context pointer is the ANativeActivity* the platform
            // passed to android_main; `from_raw` only wraps it, and the env
            // outlives the object within this callback.
            let activity = unsafe { JObject::from_raw(env, ndk_context::android_context().context().cast()) };
            Ok(clipboard_text_with_env(env, &activity))
        });
    match attached {
        Ok(text) => text,
        Err(_) => None,
    }
}

/// Clipboard read given an attached JNI env. Every step is fallible;
/// any failure means "no text".
fn clipboard_text_with_env(env: &mut jni::Env, activity: &jni::objects::JObject) -> Option<String> {
    use jni::objects::JValue;
    let name = env.new_string("clipboard").ok()?;
    let cm = env
        .call_method(
            activity,
            jni::jni_str!("getSystemService"),
            jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::from(&name)],
        )
        .ok()?
        .l()
        .ok()?;
    if cm.is_null() {
        return None;
    }
    let clip = env
        .call_method(&cm, jni::jni_str!("getPrimaryClip"), jni::jni_sig!("()Landroid/content/ClipData;"), &[])
        .ok()?
        .l()
        .ok()?;
    if clip.is_null() {
        return None;
    }
    let count = env
        .call_method(&clip, jni::jni_str!("getItemCount"), jni::jni_sig!("()I"), &[])
        .ok()?
        .i()
        .ok()?;
    if count <= 0 {
        return None;
    }
    let item = env
        .call_method(
            &clip,
            jni::jni_str!("getItemAt"),
            jni::jni_sig!("(I)Landroid/content/ClipData$Item;"),
            &[JValue::from(0)],
        )
        .ok()?
        .l()
        .ok()?;
    if item.is_null() {
        return None;
    }
    let text = env
        .call_method(&item, jni::jni_str!("getText"), jni::jni_sig!("()Ljava/lang/CharSequence;"), &[])
        .ok()?
        .l()
        .ok()?;
    if text.is_null() {
        return None;
    }
    let as_string = env
        .call_method(&text, jni::jni_str!("toString"), jni::jni_sig!("()Ljava/lang/String;"), &[])
        .ok()?
        .l()
        .ok()?;
    if as_string.is_null() {
        return None;
    }
    // SAFETY: `toString()` on a CharSequence returns a java.lang.String, so
    // the raw jobject is a valid jstring; `from_raw` only wraps the pointer.
    let jstring = unsafe { jni::objects::JString::from_raw(env, as_string.as_raw().cast()) };
    let out: String = jstring.try_to_string(env).ok()?;
    if out.is_empty() { None } else { Some(out) }
}

// ---------------------------------------------------------------------------
// JNI entries. The JVM resolves these mangled names on first call; each port
// defines its own pair forwarding to the shared handlers.

/// `<Activity>.nativeOnActivityResult`: forwards SAF picker results.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_effectcraft_android_EffectCraftActivity_nativeOnActivityResult(
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
pub unsafe extern "system" fn Java_app_effectcraft_android_EffectCraftActivity_nativeOnPenEvent(
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

//! Craft Launcher for Android (tablet).
//!
//! An Android port of storytold/craft-launcher: a NativeActivity shell that
//! shows the suite's app cards (install status, installed/latest versions,
//! release dates) with Install / Update / Open buttons. Releases come from
//! the single `areilly111/CraftApps-Android` repo, filtered by per-app tag
//! prefix; APKs are SHA-256 verified against the release's `SHA256SUMS.txt`
//! and handed to the system installer.
//!
//! This crate does not forbid `unsafe` (the JNI the shell needs), but denies
//! unwrap/expect/panic/todo like every other crate; every `unsafe` block
//! carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod catalog;
mod releases;
mod ui;

use android_activity::AndroidApp;
use craftapps_shared::{gpu, jni_bridge};
use std::path::PathBuf;

/// Port version, shown in the UI and used by the self-update row.
/// `build-apk.sh` derives the Gradle `versionCode` from it
/// (`1.0.0-port1` → 10001).
pub const VERSION_NAME: &str = "0.1.0";

/// FileProvider authority for serving downloaded APKs to the installer
/// (must match the manifest's provider authorities).
pub const PROVIDER_AUTHORITY: &str = "app.craftlauncher.android.apkprovider";

/// The android-activity 0.6 `native-activity` entry point: exported,
/// unmangled, Rust ABI.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("Craft Launcher Android {VERSION_NAME} starting");
    if let Err(e) = run(app) {
        log::error!("Craft Launcher Android failed to start: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), String> {
    jni_bridge::init()?;

    let cache_dir = PathBuf::from(jni_bridge::cache_dir().map_err(|e| format!("cache dir: {e}"))?);

    // Shared GPU cascade (Vulkan → GLES → CPU): explicit backend ordering and
    // device-error watching. The launcher keeps no GPU prefs, so the default
    // cascade applies — still better than eframe's implicit adapter pick.
    let gpu_plan = gpu::plan(
        &cache_dir.join("prefs.json"),
        &gpu::GpuAppInfo { app_name: "Craft Launcher", cpu_canvas_env: "CRAFTLAUNCHER_CPU_CANVAS" },
    );
    let mut options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    gpu::configure(&mut options, &gpu_plan);
    eframe::run_native(
        "Craft Launcher",
        options,
        Box::new(move |cc| {
            Ok(Box::new(ui::LauncherApp::new(
                cc,
                cache_dir.clone(),
                PROVIDER_AUTHORITY,
                VERSION_NAME,
                &gpu_plan,
            )) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| format!("eframe: {e}"))
}

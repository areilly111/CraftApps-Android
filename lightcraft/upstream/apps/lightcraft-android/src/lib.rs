//! LightCraft for Android (tablet).
//!
//! An Android port of storytold/lightcraft: a NativeActivity shell that runs
//! the photo library + non-destructive raw developer through eframe/egui.
//!
//! Platform services are wired to Android equivalents:
//! - file dialogs → Storage Access Framework (async [`Services::picker`],
//!   see [`file_browser`])
//! - writes → plain filesystem paths, or SAF `content://` URIs
//! - storage → the suite's shared `/sdcard/CraftApps/` tree
//! - GPU → the shared Vulkan → GLES → CPU cascade
//! - stylus → the shared pen side-channel (see `pen.rs` and the Activity's
//!   `dispatchTouchEvent`)
//!
//! This crate does not forbid `unsafe` (the JNI the shell needs), but denies
//! unwrap/expect/panic/todo like every other crate; every `unsafe` block
//! carries a SAFETY comment.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod file_browser;
mod services;
mod shell;

/// Port version, shown in the UI. `build-apk.sh` derives the Gradle
/// `versionCode` from this constant — it must stay in this file.
pub const VERSION_NAME: &str = "0.9.5";

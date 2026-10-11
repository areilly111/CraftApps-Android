//! Shared Android shell code for the CraftApps suite.
//!
//! The updater, GPU cascade, shared storage (`/sdcard/CraftApps`), SAF file
//! dialogs and JNI bridge are written once here; each port's android crate is
//! a thin shell over this plus its app-specific services, file browser and
//! eframe shell. Per-app identity arrives via the `*AppInfo` structs —
//! nothing in here names an app.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod gpu;
pub mod jni_bridge;
pub mod pen;
pub mod saf;
pub mod services;
pub mod storage;
pub mod updater;

//! In-app self-updater for the Android port.
//!
//! The upstream project ships no Android builds, so the ports publish their
//! own release channel: the single `areilly111/CraftApps-Android` repo
//! carries unified releases tagged `v<major>.<minor>.<patch>` (e.g. `v0.1.0`)
//! with one `-universal.apk` per app plus `SHA256SUMS.txt`.
//!
//! Flow: "Check for updates" (or the auto-check shortly after launch) GETs
//! the repo's release list from the GitHub API, picks the newest unified
//! release, finds the APK asset matching this app (by filename prefix),
//! compares the release version against the compiled-in version, downloads
//! the APK with progress, verifies its SHA-256 against the release's
//! checksums, then hands it to the system installer via
//! [`crate::jni_bridge::install_apk`] — the system shows its own confirmation
//! dialog, as `REQUEST_INSTALL_PACKAGES` requires.
//!
//! Network and hashing run on worker threads; the UI polls an mpsc channel
//! every frame (the same pattern as the file-dialog service).
//!
//! Reuse: each port passes its own `apk_prefix`; everything else is identical.

use crate::jni_bridge;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

/// Release channel: the single `CraftApps-Android` monorepo carries unified
/// releases tagged `v<major>.<minor>.<patch>` (e.g. `v0.1.0`), each with one
/// `-universal.apk` per app plus `SHA256SUMS.txt`.
pub const UPDATE_CHANNEL: &str = "https://api.github.com/repos/areilly111/CraftApps-Android/releases";
/// Filename prefix identifying this app's APK in a unified release (e.g.
/// `"DesignCraft-"` matches `DesignCraft-0.1.0-universal.apk`); supplied per
/// app to [`Updater::new`].
const APK_SUFFIX: &str = "-universal.apk";
const SUMS_NAME: &str = "SHA256SUMS.txt";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    Checking,
    Downloading,
    Installing,
}

pub struct ReleaseInfo {
    pub tag: String,
    pub notes: String,
    pub apk_url: String,
    pub apk_name: String,
    pub apk_size: u64,
    pub sums_url: String,
}

enum Event {
    Checked(Result<Option<ReleaseInfo>, String>),
    Progress(u64, i64),
    Downloaded(Result<PathBuf, String>),
}

pub struct Updater {
    app_name: String,
    apk_prefix: String,
    current: String,
    cache_dir: PathBuf,
    settings_path: PathBuf,
    provider_authority: String,
    auto_check: bool,
    auto_checked: bool,
    frames: u64,
    phase: Phase,
    release: Option<ReleaseInfo>,
    apk_path: Option<PathBuf>,
    progress: (u64, i64),
    status: String,
    error: Option<String>,
    window_open: bool,
    rx: Option<Receiver<Event>>,
}

impl Updater {
    pub fn new(
        app_name: &str,
        apk_prefix: &str,
        current: &str,
        cache_dir: PathBuf,
        config_dir: &Path,
        provider_authority: &str,
    ) -> Self {
        let settings_path = config_dir.join("updater.json");
        let auto_check = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("auto_check").and_then(serde_json::Value::as_bool))
            .unwrap_or(true);
        Updater {
            app_name: app_name.to_string(),
            apk_prefix: apk_prefix.to_string(),
            current: current.to_string(),
            cache_dir,
            settings_path,
            provider_authority: provider_authority.to_string(),
            auto_check,
            auto_checked: false,
            frames: 0,
            phase: Phase::Idle,
            release: None,
            apk_path: None,
            progress: (0, -1),
            status: "Not checked yet.".to_string(),
            error: None,
            window_open: false,
            rx: None,
        }
    }

    /// Open the update dialog (and start a check): the Help menu's hook.
    /// Sibling ports reuse this verbatim; only `apk_prefix` differs per app.
    pub fn open(&mut self) {
        self.window_open = true;
        self.check();
    }

    pub fn set_auto_check(&mut self, on: bool) {
        self.auto_check = on;
        let text = format!("{{\"auto_check\":{on}}}");
        if let Err(e) = std::fs::write(&self.settings_path, text) {
            self.error = Some(format!("couldn't save updater setting: {e}"));
        }
    }

    /// "0.1.0" (or "v0.1.0") → (0, 1, 0).
    fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut it = s.split('.');
        let v = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
        if it.next().is_some() {
            return None;
        }
        Some(v)
    }

    /// Start a check (no-op while one is already running).
    pub fn check(&mut self) {
        if self.phase == Phase::Checking || self.phase == Phase::Downloading {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.phase = Phase::Checking;
        self.error = None;
        self.status = "Checking for updates…".to_string();
        let current = self.current.clone();
        let apk_prefix = self.apk_prefix.clone();
        let user_agent = jni_bridge::user_agent(&self.app_name, &self.current);
        if let Err(e) = std::thread::Builder::new().name("updater-check".into()).spawn(move || {
            let _ = tx.send(Event::Checked(check_latest(&apk_prefix, &current, &user_agent)));
        }) {
            self.phase = Phase::Idle;
            self.rx = None;
            self.error = Some(format!("couldn't start update check: {e}"));
            self.status = "Check failed.".to_string();
        }
    }

    /// Download the APK of the available release (no-op unless Available).
    pub fn download(&mut self) {
        let Some(rel) = self.release.as_ref() else { return };
        if self.phase == Phase::Downloading || self.phase == Phase::Checking {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.phase = Phase::Downloading;
        self.error = None;
        self.progress = (0, -1);
        self.status = format!("Downloading {}…", rel.apk_name);
        let url = rel.apk_url.clone();
        let name = rel.apk_name.clone();
        let sums_url = rel.sums_url.clone();
        let cache_dir = self.cache_dir.clone();
        let user_agent = jni_bridge::user_agent(&self.app_name, &self.current);
        if let Err(e) = std::thread::Builder::new().name("updater-download".into()).spawn(move || {
            let _ = tx.send(Event::Downloaded(download_and_verify(&tx, &user_agent, &url, &name, &sums_url, &cache_dir)));
        }) {
            self.phase = Phase::Idle;
            self.rx = None;
            self.error = Some(format!("couldn't start download: {e}"));
            self.status = "Download failed.".to_string();
        }
    }

    /// Hand the verified APK to the system installer.
    pub fn install(&mut self) {
        let Some(path) = self.apk_path.clone() else { return };
        match jni_bridge::install_apk(&path, &self.provider_authority) {
            Ok(()) => {
                self.phase = Phase::Installing;
                self.status = "Confirm the install in the system dialog.".to_string();
            }
            Err(e) => {
                self.phase = Phase::Idle;
                self.error = Some(format!("couldn't start the installer: {e}"));
            }
        }
    }

    /// Drain worker events; run the auto-check once, shortly after launch.
    pub fn poll(&mut self) {
        self.frames += 1;
        if self.auto_check && !self.auto_checked && self.frames > 90 {
            self.auto_checked = true;
            self.check();
        }
        // Take the receiver out while draining so event handling can borrow self.
        let mut rx = self.rx.take();
        if let Some(r) = rx.as_ref() {
            loop {
                match r.try_recv() {
                    Ok(event) => self.handle(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        rx = None;
                        break;
                    }
                }
            }
        }
        self.rx = rx;
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Checked(Ok(None)) => {
                self.phase = Phase::Idle;
                self.status = format!("You're on the latest version ({}).", self.current);
            }
            Event::Checked(Ok(Some(rel))) => {
                self.phase = Phase::Idle;
                self.release = Some(rel);
                self.status = "An update is available.".to_string();
            }
            Event::Checked(Err(e)) => {
                self.phase = Phase::Idle;
                self.error = Some(e);
                self.status = "Check failed.".to_string();
            }
            Event::Progress(received, total) => {
                self.progress = (received, total);
            }
            Event::Downloaded(Ok(path)) => {
                self.phase = Phase::Idle;
                self.apk_path = Some(path);
                self.status = "Download verified. Ready to install.".to_string();
            }
            Event::Downloaded(Err(e)) => {
                self.phase = Phase::Idle;
                self.error = Some(e);
                self.status = "Download failed.".to_string();
            }
        }
    }

    fn update_available(&self) -> bool {
        self.release.is_some() && self.apk_path.is_none()
    }

    /// Floating button + dialog, drawn by the shell after the main app.
    pub fn show(&mut self, ctx: &egui::Context) {
        // No floating button: updates live under Help → "Check for Updates…"
        // (see `open()`), which is where the user asked for them.
        if !self.window_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Software Update")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([420.0, 380.0])
            .show(ctx, |ui| {
                ui.label(format!("Current version: {}", self.current));
                ui.separator();
                match self.phase {
                    Phase::Checking => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Checking for updates…");
                        });
                    }
                    Phase::Downloading => {
                        let (received, total) = self.progress;
                        let frac = if total > 0 { received as f32 / total as f32 } else { 0.0 };
                        ui.add(egui::ProgressBar::new(frac).show_percentage());
                        ui.label(format!("{} / {}", fmt_bytes(received), fmt_bytes(total.max(0) as u64)));
                    }
                    Phase::Installing => {
                        ui.label("Confirm the install in the system dialog to finish.");
                    }
                    Phase::Idle => {}
                }
                ui.label(&self.status);
                if let Some(e) = self.error.as_ref() {
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 90), e);
                }
                if let Some(rel) = self.release.as_ref() {
                    ui.separator();
                    ui.heading(format!("{} {}", self.app_name, rel.tag));
                    ui.label(format!("Size: {}", fmt_bytes(rel.apk_size)));
                    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                        ui.label(&rel.notes);
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let busy = self.phase == Phase::Checking || self.phase == Phase::Downloading;
                    if ui.add_enabled(!busy, egui::Button::new("Check for updates")).clicked() {
                        self.check();
                    }
                    if self.update_available() {
                        let label = if self.phase == Phase::Downloading { "Downloading…" } else { "Download update" };
                        if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                            self.download();
                        }
                    }
                    if self.apk_path.is_some() && ui.button("Install now").clicked() {
                        self.install();
                    }
                });
                let mut auto = self.auto_check;
                if ui.checkbox(&mut auto, "Check for updates on launch").changed() {
                    self.set_auto_check(auto);
                }
            });
        self.window_open = open;
    }
}

fn fmt_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}

/// Query the shared `CraftApps-Android` release feed and pick the newest
/// unified release (tag `v<major>.<minor>.<patch>`, e.g. `v0.1.0`; GitHub
/// returns newest first), then find this app's APK in its assets by
/// `apk_prefix` (e.g. `"DesignCraft-"` matches
/// `DesignCraft-0.1.0-universal.apk`).
/// `Ok(None)` = up to date (or unparseable tag, in which case we stay quiet
/// rather than nag).
fn check_latest(apk_prefix: &str, current: &str, user_agent: &str) -> Result<Option<ReleaseInfo>, String> {
    let url = format!("{UPDATE_CHANNEL}?per_page=100");
    let body = jni_bridge::http_get(&url, user_agent, None, None).map_err(|e| {
        if e.contains("404") {
            "The update channel isn't published yet (no releases on areilly111/CraftApps-Android).".to_string()
        } else {
            format!("update check failed: {e}")
        }
    })?;
    let feed: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("couldn't parse the release feed: {e}"))?;
    let releases = feed.as_array().ok_or_else(|| "release feed isn't a release list".to_string())?;
    let v = releases
        .iter()
        .find(|r| {
            r.get("tag_name")
                .and_then(|t| t.as_str())
                .is_some_and(|t| Updater::parse_version(t).is_some())
        })
        .ok_or_else(|| "no unified releases on the update channel yet".to_string())?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).ok_or_else(|| "release feed has no tag_name".to_string())?;
    let notes = v.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string();
    let assets = v.get("assets").and_then(|a| a.as_array()).ok_or_else(|| "release feed has no assets".to_string())?;
    let apk = assets.iter().find(|a| {
        a.get("name").and_then(|n| n.as_str()).is_some_and(|n| {
            n.starts_with(apk_prefix) && n.ends_with(APK_SUFFIX)
        })
    });
    let sums = assets.iter().find(|a| a.get("name").and_then(|n| n.as_str()).is_some_and(|n| n == SUMS_NAME));
    let (Some(apk), Some(sums)) = (apk, sums) else {
        return Err("latest release has no APK for this app".to_string());
    };
    let apk_url = apk.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or("").to_string();
    let apk_name = apk.get("name").and_then(|n| n.as_str()).unwrap_or("update.apk").to_string();
    let apk_size = apk.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
    let sums_url = sums.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or("").to_string();
    if apk_url.is_empty() || sums_url.is_empty() {
        return Err("release assets have no download URLs".to_string());
    }
    let newer = {
        let cur = Updater::parse_version(current);
        let new = Updater::parse_version(tag);
        match (cur, new) {
            (Some(c), Some(n)) => n > c,
            _ => false,
        }
    };
    if !newer {
        return Ok(None);
    }
    Ok(Some(ReleaseInfo { tag: tag.to_string(), notes, apk_url, apk_name, apk_size, sums_url }))
}

/// Download the APK (progress → `tx`), verify its SHA-256 against the
/// release's `SHA256SUMS.txt`, return the file path.
fn download_and_verify(
    tx: &Sender<Event>,
    user_agent: &str,
    apk_url: &str,
    apk_name: &str,
    sums_url: &str,
    cache_dir: &Path,
) -> Result<PathBuf, String> {
    let dest = cache_dir.join(apk_name);
    let (ptx, prx) = channel::<(u64, i64)>();
    let fwd = tx.clone();
    std::thread::Builder::new()
        .name("updater-progress".into())
        .spawn(move || {
            for (received, total) in prx {
                let _ = fwd.send(Event::Progress(received, total));
            }
        })
        .map_err(|e| format!("couldn't start progress relay: {e}"))?;
    jni_bridge::http_get(apk_url, user_agent, Some(dest.as_path()), Some(&ptx)).map_err(|e| format!("download failed: {e}"))?;
    let sums = jni_bridge::http_get(sums_url, user_agent, None, None).map_err(|e| format!("couldn't fetch checksums: {e}"))?;
    let sums = String::from_utf8_lossy(&sums);
    let want = sums
        .lines()
        .find_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            if name.trim().trim_start_matches('*') == apk_name { Some(hash.to_string()) } else { None }
        })
        .ok_or_else(|| format!("{SUMS_NAME} has no entry for {apk_name}"))?;
    let bytes = std::fs::read(&dest).map_err(|e| format!("couldn't read the downloaded APK: {e}"))?;
    let got = jni_bridge::sha256_hex(&bytes)?;
    if got != want.to_lowercase() {
        let _ = std::fs::remove_file(&dest);
        return Err("checksum mismatch — the download is corrupt or tampered with; deleted.".to_string());
    }
    Ok(dest)
}

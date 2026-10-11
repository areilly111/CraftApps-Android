//! The launcher UI: a grid of app cards, modeled on craft-launcher's
//! `views.rs` (brand-color strip, app name, tagline, status, versions,
//! Install / Update / Open buttons).
//!
//! Dark theme, hard square corners, 1 px hairlines — the getartcraft.com
//! look the upstream launcher follows.

use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui::{self, Color32, RichText, Ui};

use crate::catalog::SUITE;
use crate::releases::{CheckOutcome, LatestRelease, ReleaseChecker, cmp_versions, download_and_verify};

/// Per-app install/download state.
#[derive(Default)]
struct AppState {
    /// From PackageManager: (version_code, version_name).
    installed: Option<(i64, String)>,
    /// Latest release, once the check lands.
    latest: Option<LatestRelease>,
    /// `Some` while the release check for this app hasn't answered.
    checking: bool,
    /// Why release info is unavailable (private repo, no network…).
    unavailable: Option<String>,
    /// No release with this app's tag prefix exists yet.
    no_release: bool,
    /// Download in flight: (received, total_or_-1).
    downloading: Option<(u64, i64)>,
    /// Verified APK waiting for the install tap.
    downloaded: Option<std::path::PathBuf>,
    error: Option<String>,
}

enum DlEvent {
    Progress(usize, u64, i64),
    Done(usize, Result<std::path::PathBuf, String>),
}

pub struct LauncherApp {
    states: Vec<AppState>,
    checker: ReleaseChecker,
    user_agent: String,
    cache_dir: std::path::PathBuf,
    provider_authority: String,
    dl_tx: Sender<DlEvent>,
    dl_rx: Receiver<DlEvent>,
    /// This launcher's own version (`1.0.0-port1`), for the self-update row.
    own_version: String,
}

use craftapps_shared::{gpu, jni_bridge};

/// Minimal [`gpu::GpuCanvasHost`] for the launcher: no GPU canvas compositing
/// (plain egui UI), but the shared Vulkan→GLES→CPU cascade still picks the
/// adapter and device errors are logged instead of panicking.
struct GpuHost;

impl gpu::GpuCanvasHost for GpuHost {
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo) {
        log::info!(
            "Craft Launcher graphics: {} ({:?}, {:?})",
            info.name.trim(),
            info.backend,
            info.device_type
        );
    }
    fn record_software_fallback(&mut self, note: &str) {
        log::info!("Craft Launcher: {note}");
    }
    fn set_gpu_canvas(&mut self, _rs: eframe::egui_wgpu::RenderState) {
        // No canvas compositing in the launcher; the egui UI draws directly.
    }
    fn watch_device(&mut self, device: &eframe::wgpu::Device) {
        let device = device.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |e: eframe::wgpu::Error| {
            log::error!("Craft Launcher wgpu device error: {e}");
        }));
    }
}

impl LauncherApp {
    pub fn new(
        cc: &eframe::CreationContext,
        cache_dir: std::path::PathBuf,
        provider_authority: &str,
        own_version: &str,
        gpu_plan: &gpu::GpuPlan,
    ) -> Self {
        let user_agent = jni_bridge::user_agent("craftlauncher", own_version);
        // Attach the shared GPU cascade's adapter pick: Vulkan-first ordering
        // plus device-error watching (the launcher keeps the default cascade —
        // it stores no GPU prefs — but still benefits from explicit backends).
        gpu::attach_canvas(&mut GpuHost, cc.wgpu_render_state.clone(), gpu_plan);
        let mut states = Vec::new();
        states.resize_with(SUITE.len(), AppState::default);
        let checker = ReleaseChecker::new(&user_agent);
        let (dl_tx, dl_rx) = channel();
        let mut app = LauncherApp {
            states,
            checker,
            user_agent,
            cache_dir,
            provider_authority: provider_authority.to_string(),
            dl_tx,
            dl_rx,
            own_version: own_version.to_string(),
        };
        app.refresh();
        app
    }

    /// Re-scan installed versions and re-check releases.
    fn refresh(&mut self) {
        for (i, app) in SUITE.iter().enumerate() {
            self.states[i].installed = craftapps_shared::jni_bridge::installed_package_info(app.package)
                .unwrap_or(None);
            self.states[i].checking = true;
            self.states[i].unavailable = None;
            self.states[i].no_release = false;
        }
        // The launcher's own installed version is this build.
        if let Some(last) = self.states.last_mut() {
            last.installed = Some((10001, self.own_version.clone()));
        }
        self.checker.check_all(&SUITE);
    }

    fn pump(&mut self, ctx: &egui::Context) {
        let mut repaint = false;
        for (i, outcome) in self.checker.drain() {
            if let Some(st) = self.states.get_mut(i) {
                st.checking = false;
                match outcome {
                    CheckOutcome::Found(rel) => {
                        st.no_release = rel.is_none();
                        st.latest = rel;
                    }
                    CheckOutcome::Unavailable(msg) => st.unavailable = Some(msg),
                }
                repaint = true;
            }
        }
        while let Ok(ev) = self.dl_rx.try_recv() {
            match ev {
                DlEvent::Progress(i, r, t) => {
                    if let Some(st) = self.states.get_mut(i) {
                        st.downloading = Some((r, t));
                        repaint = true;
                    }
                }
                DlEvent::Done(i, res) => {
                    if let Some(st) = self.states.get_mut(i) {
                        st.downloading = None;
                        match res {
                            Ok(p) => st.downloaded = Some(p),
                            Err(e) => st.error = Some(e),
                        }
                        repaint = true;
                    }
                }
            }
        }
        if repaint {
            ctx.request_repaint();
        }
    }

    /// What the card's primary button should do.
    fn primary_action(&self, i: usize) -> Primary {
        let st = &self.states[i];
        let app = &SUITE[i];
        if st.downloading.is_some() {
            return Primary::Busy;
        }
        if let Some(path) = st.downloaded.clone() {
            return Primary::InstallNow(path);
        }
        let installed_version = st.installed.as_ref().map(|(_, v)| v.as_str()).unwrap_or("");
        match (&st.latest, st.installed.is_some()) {
            (Some(rel), false) => Primary::Download(rel.clone()),
            (Some(rel), true) => {
                if cmp_versions(&rel.version, installed_version) == std::cmp::Ordering::Greater {
                    Primary::Download(rel.clone())
                } else {
                    Primary::Open(app.package)
                }
            }
            // Installed but no release info (private repo): can still open.
            (None, true) => Primary::Open(app.package),
            // Not installed and nothing to download.
            (None, false) => Primary::Unavailable,
        }
    }

    fn start_download(&mut self, i: usize, rel: &LatestRelease) {
        let st = match self.states.get_mut(i) {
            Some(s) => s,
            None => return,
        };
        st.error = None;
        st.downloaded = None;
        st.downloading = Some((0, -1));
        let tx = self.dl_tx.clone();
        let ua = self.user_agent.clone();
        let cache = self.cache_dir.clone();
        let rel = rel.clone();
        std::thread::Builder::new()
            .name(format!("dl-{}", SUITE[i].id))
            .spawn(move || {
                let (ptx, prx) = channel::<(u64, i64)>();
                let fwd = tx.clone();
                std::thread::Builder::new()
                    .name("dl-progress".into())
                    .spawn(move || {
                        for (r, t) in prx {
                            let _ = fwd.send(DlEvent::Progress(i, r, t));
                        }
                    })
                    .ok();
                let res = download_and_verify(
                    &ua,
                    &rel.apk_url,
                    &rel.apk_name,
                    &rel.sums_url,
                    &cache,
                    &ptx,
                );
                let _ = tx.send(DlEvent::Done(i, res));
            })
            .map_err(|e| {
                if let Some(st) = self.states.get_mut(i) {
                    st.downloading = None;
                    st.error = Some(format!("couldn't start download: {e}"));
                }
            })
            .ok();
    }

    fn install(&mut self, i: usize, path: std::path::PathBuf) {
        let ok = craftapps_shared::jni_bridge::install_apk(&path, &self.provider_authority);
        if let Some(st) = self.states.get_mut(i) {
            match ok {
                Ok(()) => {
                    st.downloaded = None;
                    st.error = None;
                }
                Err(e) => st.error = Some(format!("couldn't start the installer: {e}")),
            }
        }
    }

    fn open(&self, package: &str) {
        if let Err(e) = craftapps_shared::jni_bridge::launch_package(package) {
            log::warn!("open {package} failed: {e}");
        }
    }
}

enum Primary {
    Busy,
    InstallNow(std::path::PathBuf),
    Download(LatestRelease),
    Open(&'static str),
    Unavailable,
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        // Keep progress bars moving.
        if self.states.iter().any(|s| s.downloading.is_some() || s.checking) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        let bg = Color32::from_rgb(0x14, 0x14, 0x16);
        let panel = Color32::from_rgb(0x1e, 0x1e, 0x22);
        let hairline = Color32::from_rgb(0x3a, 0x3a, 0x40);
        let fg = Color32::from_rgb(0xf2, 0xf1, 0xee);
        let muted = Color32::from_rgb(0x9a, 0x98, 0x94);

        // eframe already wraps this Ui in the central panel; paint the
        // background and draw the header + card grid directly.
        let bg_rect = ui.max_rect();
        ui.painter().rect_filled(bg_rect, 0.0, bg);

        // Header.
        ui.horizontal(|ui| {
            ui.heading(RichText::new("CraftApps Launcher").color(fg).size(22.0));
            ui.label(RichText::new("Install, update and open the Craft apps").color(muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⟳ Check for updates").clicked() {
                    self.refresh();
                }
            });
        });
        ui.label(RichText::new("Unofficial Android ports — based on the craft apps by the ArtCraft team").color(muted).size(11.0));
        ui.add_space(8.0);

        // Card grid: 2 columns on a tablet.
        let n = SUITE.len();
        let half = n.div_ceil(2);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(2, |cols| {
                for i in 0..half {
                    draw_card(self, &mut cols[0], i, panel, hairline, fg, muted);
                }
                for i in half..n {
                    draw_card(self, &mut cols[1], i, panel, hairline, fg, muted);
                }
            });
        });
    }
}

/// One app card. A free function (not a method) so the draw closure can call
/// `&mut LauncherApp` actions; per-app state is snapshotted up front.
fn draw_card(
    launcher: &mut LauncherApp,
    ui: &mut Ui,
    i: usize,
    panel: Color32,
    hairline: Color32,
    fg: Color32,
    muted: Color32,
) {
    let app = &SUITE[i];
    let accent = Color32::from_rgb(app.color[0], app.color[1], app.color[2]);
    // Snapshot: the button handlers below need `&mut launcher`.
    let (installed, latest, unavailable, checking, no_release, downloading, error) = {
        let st = &launcher.states[i];
        (
            st.installed.clone(),
            st.latest.clone(),
            st.unavailable.clone(),
            st.checking,
            st.no_release,
            st.downloading,
            st.error.clone(),
        )
    };

    egui::Frame::new()
        .fill(panel)
        .stroke(egui::Stroke::new(1.0, hairline))
        .inner_margin(0.0)
        .show(ui, |ui| {
            // Brand strip.
            let (strip, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
            ui.painter().rect_filled(strip, 0.0, accent);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(app.name).color(fg).size(18.0).strong());
                    ui.label(RichText::new(app.tagline).color(muted).size(13.0));
                    ui.add_space(6.0);
                    // Status lines.
                    match installed.as_ref() {
                        Some((_, v)) => {
                            ui.label(RichText::new(format!("Installed: {v}")).color(fg).size(13.0));
                        }
                        None => {
                            ui.label(RichText::new("Not installed").color(muted).size(13.0));
                        }
                    }
                    match (&latest, &unavailable, checking, no_release) {
                        (_, _, true, _) => {
                            ui.label(RichText::new("Checking releases…").color(muted).size(13.0));
                        }
                        (Some(rel), _, _, _) => {
                            ui.label(
                                RichText::new(format!(
                                    "Latest: {}{}",
                                    rel.version,
                                    rel.published_at.map(|p| format!(" · {}", date_text(p))).unwrap_or_default()
                                ))
                                .color(muted)
                                .size(13.0),
                            );
                        }
                        (None, Some(msg), _, _) => {
                            ui.label(RichText::new(msg.clone()).color(muted).size(12.0));
                        }
                        (None, None, _, true) => {
                            ui.label(RichText::new("No releases published yet").color(muted).size(13.0));
                        }
                        (None, None, false, false) => {}
                    }
                    if let Some(e) = error {
                        ui.label(RichText::new(e).color(Color32::from_rgb(0xff, 0x7a, 0x7a)).size(12.0));
                    }
                    ui.add_space(8.0);
                    // Primary button.
                    match launcher.primary_action(i) {
                        Primary::Busy => {
                            if let Some((r, t)) = downloading {
                                let frac = if t > 0 { r as f32 / t as f32 } else { 0.0 };
                                ui.add(egui::ProgressBar::new(frac).show_percentage());
                            } else {
                                ui.spinner();
                            }
                        }
                        Primary::InstallNow(path) => {
                            if ui.button(RichText::new("INSTALL").size(14.0)).clicked() {
                                launcher.install(i, path);
                            }
                            ui.label(RichText::new("Download verified — tap INSTALL, then confirm in the system dialog.").color(muted).size(12.0));
                        }
                        Primary::Download(rel) => {
                            let label = if installed.is_some() { "UPDATE" } else { "INSTALL" };
                            let size = if rel.apk_size > 0 {
                                format!(" ({} MB)", rel.apk_size / 1_000_000)
                            } else {
                                String::new()
                            };
                            if ui.button(RichText::new(format!("{label}{size}")).size(14.0)).clicked() {
                                launcher.start_download(i, &rel);
                            }
                        }
                        Primary::Open(pkg) => {
                            if ui.button(RichText::new("OPEN").size(14.0)).clicked() {
                                launcher.open(pkg);
                            }
                        }
                        Primary::Unavailable => {
                            ui.add_enabled(false, egui::Button::new("UNAVAILABLE"));
                        }
                    }
                    ui.add_space(12.0);
                });
            });
        });
    ui.add_space(12.0);
}

/// "Oct 10, 2026".
fn date_text(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mi = (m - 1).clamp(0, 11) as usize;
    format!("{} {d}, {y}", MONTHS[mi])
}

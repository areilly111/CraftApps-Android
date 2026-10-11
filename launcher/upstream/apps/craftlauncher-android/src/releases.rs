//! Release tracking for the suite.
//!
//! The single `areilly111/CraftApps-Android` repo carries one GitHub release
//! per app build, tagged `<id>-v<upstream>-port<build>` (e.g.
//! `photocraft-v0.6.0-port5`) with a `-universal.apk` plus `SHA256SUMS.txt`.
//!
//! The repo is PRIVATE (the user's call), so the unauthenticated API poll
//! 404s. That is handled gracefully: the app is shown as "release info
//! unavailable" instead of erroring. No tokens are embedded anywhere.
//!
//! Network runs on worker threads; the UI polls an mpsc channel every frame.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::catalog::SuiteApp;

/// Release channel for all suite apps.
pub const RELEASES_API: &str = "https://api.github.com/repos/areilly111/CraftApps-Android/releases?per_page=100";
const APK_SUFFIX: &str = "-universal.apk";
const SUMS_NAME: &str = "SHA256SUMS.txt";

/// What we know about an app's latest release.
#[derive(Clone, Debug, Default)]
pub struct LatestRelease {
    /// `0.6.0-port5` (the tag minus the `<id>-v` prefix).
    pub version: String,
    /// Unix seconds, when the API told us.
    pub published_at: Option<i64>,
    pub apk_url: String,
    pub apk_name: String,
    pub apk_size: u64,
    pub sums_url: String,
}

/// Outcome of checking one app.
pub enum CheckOutcome {
    /// Found a release (or confirmed there is none newer to report).
    Found(Option<LatestRelease>),
    /// The repo is private / API unreachable: show "unavailable", not an error.
    Unavailable(String),
}

pub struct ReleaseChecker {
    user_agent: String,
    tx: Sender<(usize, CheckOutcome)>,
    rx: Receiver<(usize, CheckOutcome)>,
}

impl ReleaseChecker {
    pub fn new(user_agent: &str) -> Self {
        let (tx, rx) = channel();
        ReleaseChecker { user_agent: user_agent.to_string(), tx, rx }
    }

    /// Start a background check for every app in the suite.
    pub fn check_all(&self, apps: &[SuiteApp]) {
        for (i, app) in apps.iter().enumerate() {
            let tx = self.tx.clone();
            let ua = self.user_agent.clone();
            let prefix = app.full_tag_prefix();
            std::thread::Builder::new()
                .name(format!("release-check-{}", app.id))
                .spawn(move || {
                    let outcome = check_one(&prefix, &ua);
                    let _ = tx.send((i, outcome));
                })
                .map_err(|e| log::warn!("couldn't start release check thread: {e}"))
                .ok();
        }
    }

    /// Drain completed checks; returns `(app_index, outcome)` pairs.
    pub fn drain(&self) -> Vec<(usize, CheckOutcome)> {
        let mut out = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            out.push(item);
        }
        out
    }
}

fn check_one(tag_prefix: &str, user_agent: &str) -> CheckOutcome {
    let body = match craftapps_shared::jni_bridge::http_get(RELEASES_API, user_agent, None, None) {
        Ok(b) => b,
        Err(e) => {
            // Private repo: unauthenticated API access 404s. Anything else
            // (no network, timeout) is reported the same gentle way.
            let msg = if e.contains("404") {
                "release info unavailable — the CraftApps-Android repo is private".to_string()
            } else {
                format!("release info unavailable ({e})")
            };
            return CheckOutcome::Unavailable(msg);
        }
    };
    let feed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return CheckOutcome::Unavailable(format!("release info unavailable (bad feed: {e})")),
    };
    let releases = match feed.as_array() {
        Some(a) => a,
        None => return CheckOutcome::Unavailable("release info unavailable (unexpected feed)".to_string()),
    };
    // The API returns newest first; take the first release for this app.
    for r in releases {
        let tag = match r.get("tag_name").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };
        if !tag.starts_with(tag_prefix) {
            continue;
        }
        if r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false) {
            continue;
        }
        let version = tag.strip_prefix(tag_prefix).unwrap_or(tag).to_string();
        let published_at = r.get("published_at").and_then(|p| p.as_str()).and_then(parse_rfc3339);
        let assets = match r.get("assets").and_then(|a| a.as_array()) {
            Some(a) => a,
            None => continue,
        };
        let apk = assets.iter().find(|a| {
            a.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.ends_with(APK_SUFFIX))
        });
        let sums = assets.iter().find(|a| {
            a.get("name").and_then(|n| n.as_str()).is_some_and(|n| n == SUMS_NAME)
        });
        let (Some(apk), Some(sums)) = (apk, sums) else { continue };
        let apk_url = apk.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or("").to_string();
        let apk_name = apk.get("name").and_then(|n| n.as_str()).unwrap_or("app.apk").to_string();
        let apk_size = apk.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        let sums_url = sums.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or("").to_string();
        if apk_url.is_empty() || sums_url.is_empty() {
            continue;
        }
        return CheckOutcome::Found(Some(LatestRelease {
            version,
            published_at,
            apk_url,
            apk_name,
            apk_size,
            sums_url,
        }));
    }
    CheckOutcome::Found(None)
}

/// Minimal RFC 3339 (`2026-10-10T21:35:00Z`) → unix seconds. Only the `Z`
/// form the GitHub API emits.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) =
        (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let mut t = time.split(':');
    let (hh, mm, ss): (i64, i64, i64) =
        (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = (m + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

/// Compare `X.Y.Z-portN` versions. Returns `Ordering`; unparseable → Equal.
pub fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    match (parse_version(a), parse_version(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => std::cmp::Ordering::Equal,
    }
}

fn parse_version(v: &str) -> Option<(u64, u64, u64, u64)> {
    let (up, port) = v.split_once("-port")?;
    let mut it = up.split('.');
    let (maj, min, pat): (u64, u64, u64) =
        (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    Some((maj, min, pat, port.parse().ok()?))
}

/// Download the APK with progress, verify its SHA-256 against the release's
/// `SHA256SUMS.txt`, return the file path. Mirrors the shared updater's
/// `download_and_verify` (which is per-app UI bound; the launcher drives
/// several downloads from one UI).
pub fn download_and_verify(
    user_agent: &str,
    apk_url: &str,
    apk_name: &str,
    sums_url: &str,
    cache_dir: &std::path::Path,
    progress: &Sender<(u64, i64)>,
) -> Result<PathBuf, String> {
    let dest = cache_dir.join(apk_name);
    craftapps_shared::jni_bridge::http_get(apk_url, user_agent, Some(&dest), Some(progress))
        .map_err(|e| format!("download failed: {e}"))?;
    let sums = craftapps_shared::jni_bridge::http_get(sums_url, user_agent, None, None)
        .map_err(|e| format!("couldn't fetch checksums: {e}"))?;
    let sums = String::from_utf8_lossy(&sums);
    let want = sums
        .lines()
        .find_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            if name.trim().trim_start_matches('*') == apk_name {
                Some(hash.to_ascii_lowercase())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("{SUMS_NAME} has no entry for {apk_name}"))?;
    if want.len() != 64 || !want.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("checksum in SHA256SUMS.txt is malformed".to_string());
    }
    let bytes = std::fs::read(&dest).map_err(|e| format!("couldn't read the downloaded APK: {e}"))?;
    let got = craftapps_shared::jni_bridge::sha256_hex(&bytes)?;
    if got != want {
        let _ = std::fs::remove_file(&dest);
        return Err("checksum mismatch — the download is corrupt or tampered with; deleted.".to_string());
    }
    Ok(dest)
}

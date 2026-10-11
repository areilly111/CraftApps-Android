//! Headless audio playback bench: plays a project's sequences through a simulated 48 kHz stereo
//! device with a 512-frame buffer (the app's defaults) and counts dropouts, the way the device
//! would hear them. Nothing is played out loud.
//!
//! ```sh
//! FILMCRAFT_DATA_DIR=/tmp/fc-bench cargo run --release -p filmcraft-ui-egui --example bench_audio -- \
//!     "Project.fcproj" --seconds 20 [--sequence "One hero" …] [--mode direct|ahead|both] [--json out.json] [--wav dir]
//! ```
//!
//! `direct` mixes inside the device callback (how playback worked before sound was mixed ahead);
//! `ahead` plays through [`play_ahead`] as the app does now. A callback that returns after its
//! buffer was due is a dropout, and the device cycles it overran are lost. For `ahead` the ring's
//! own underruns (silence played because the mixer fell behind) count too. `--wav` writes what a
//! device would have played for each run, to listen to.
//!
//! The simulated device thread runs at normal priority (CoreAudio's own I/O thread runs higher),
//! so absolute counts are pessimistic for both modes; compare the modes on the same machine load.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use filmcraft_engine::Session;
use filmcraft_project::ItemId;
use filmcraft_ui_egui::play_ahead::{self, AudioStats};
use serde_json::{Value, json};

const SR: u32 = 48_000;
const PERIOD: usize = 512;
const CH: usize = 2;

#[derive(Default)]
struct Run {
    callbacks: u64,
    /// Callbacks that returned after their buffer was due.
    late: u64,
    /// Callbacks whose own work took longer than one buffer: a dropout on any device.
    overruns: u64,
    /// Device cycles lost to late callbacks.
    lost_cycles: u64,
    max_callback_ms: f64,
    /// `ahead` only: callbacks the ring could not fill.
    ring_underruns: u64,
    min_lead_ms: Option<f64>,
    /// What a device whose I/O thread always wakes on time plays (interleaved stereo): a callback
    /// that overran loses its buffer and the slots it overran to silence.
    heard: Vec<f32>,
}

impl Run {
    /// What a device whose I/O thread always wakes on time would hear: overrunning callbacks plus
    /// buffers the ring could not fill. (`late` also counts this bench's normal-priority device
    /// thread waking late under load, which CoreAudio's real-time thread does not.)
    fn dropouts(&self) -> u64 {
        self.overruns + self.ring_underruns
    }
}

/// Calls `fill` once per device period for `seconds` of wall time, as a device does.
fn device(fill: &mut dyn FnMut(&mut [f32], usize), seconds: f64) -> Run {
    let period = PERIOD as f64 / SR as f64;
    let total = (seconds / period) as u64;
    let mut buf = vec![0.0f32; PERIOD * CH];
    let mut r = Run::default();
    let t0 = Instant::now();
    let mut k = 0u64;
    while k < total {
        let due = Duration::from_secs_f64(period * k as f64);
        if let Some(wait) = due.checked_sub(t0.elapsed()) {
            std::thread::sleep(wait);
        }
        let c0 = Instant::now();
        fill(&mut buf, CH);
        r.callbacks += 1;
        let took = c0.elapsed().as_secs_f64();
        r.max_callback_ms = r.max_callback_ms.max(took * 1e3);
        if took > period {
            r.overruns += 1;
            let lost = (took / period).ceil() as usize;
            r.heard.resize(r.heard.len() + lost * PERIOD * CH, 0.0);
        } else {
            r.heard.extend_from_slice(&buf);
        }
        let done = t0.elapsed().as_secs_f64();
        let deadline = period * (k + 1) as f64;
        if done > deadline {
            r.late += 1;
            let next = (done / period).ceil() as u64;
            r.lost_cycles += next.saturating_sub(k + 1);
            k = next;
        } else {
            k += 1;
        }
    }
    r
}

/// 32-bit float stereo WAV at the device rate.
fn write_wav(path: &std::path::Path, samples: &[f32]) -> std::io::Result<()> {
    let data = (samples.len() * 4) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&(CH as u16).to_le_bytes());
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * CH as u32 * 4).to_le_bytes());
    b.extend_from_slice(&(CH as u16 * 4).to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for x in samples {
        b.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(path, b)
}

fn play(s: &Session, seq: ItemId, mode: &str, seconds: f64) -> Run {
    let mut mix = filmcraft_ui_egui::playback_mix(s, seq, SR);
    match mode {
        "direct" => {
            let mut cursor = 0i64;
            let mut fill = move |buf: &mut [f32], ch: usize| {
                mix(cursor, buf, ch);
                cursor += (buf.len() / ch) as i64;
            };
            device(&mut fill, seconds)
        }
        _ => {
            let stats = Arc::new(AudioStats::default());
            let mut fill = play_ahead::spawn(mix, 0, SR, CH, stats.clone());
            let mut r = device(&mut *fill, seconds);
            r.ring_underruns = stats.underruns.load(Ordering::Relaxed);
            let min = stats.min_lead_frames.load(Ordering::Relaxed);
            r.min_lead_ms = (min != u64::MAX).then(|| min as f64 * 1e3 / SR as f64);
            r
        }
    }
}

fn main() {
    filmcraft_platform::register();
    let mut args = std::env::args().skip(1);
    let (mut project, mut seconds, mut only, mut mode, mut json_out, mut wav_dir) = (None, 20.0, Vec::new(), "both".to_string(), None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().and_then(|v| v.parse().ok()).unwrap_or(seconds),
            "--sequence" => only.extend(args.next()),
            "--mode" => mode = args.next().unwrap_or(mode),
            "--json" => json_out = args.next(),
            "--wav" => wav_dir = args.next().map(std::path::PathBuf::from),
            p => project = Some(p.to_string()),
        }
    }
    let Some(project) = project else {
        eprintln!("usage: bench_audio <project.fcproj> [--seconds N] [--sequence NAME] [--mode direct|ahead|both] [--json PATH] [--wav DIR]");
        std::process::exit(2);
    };
    let mut s = Session::default();
    if let Err(e) = s.execute("file.open", json!({ "path": project })) {
        eprintln!("{project}: {e}");
        std::process::exit(1);
    }
    let mut seqs: Vec<(ItemId, String)> = s.project.sequences().map(|i| (i.id, i.name.clone())).filter(|(_, n)| only.is_empty() || only.contains(n)).collect();
    seqs.sort_by(|a, b| a.1.cmp(&b.1));
    let modes: Vec<&str> = if mode == "both" { vec!["direct", "ahead"] } else { vec![mode.as_str()] };
    println!(
        "{:<48} {:<7} {:>9} {:>8} {:>9} {:>9} {:>8} {:>11} {:>9}",
        "sequence", "mode", "callbacks", "dropouts", "overruns", "underruns", "late", "max cb (ms)", "min lead"
    );
    let mut out = Vec::<Value>::new();
    for (id, name) in &seqs {
        for m in &modes {
            let r = play(&s, *id, m, seconds);
            if let Some(d) = &wav_dir
                && let Err(e) = write_wav(&d.join(format!("{name} - {m}.wav")), &r.heard)
            {
                eprintln!("{}: {e}", d.display());
            }
            let lost_ms = r.lost_cycles as f64 * PERIOD as f64 * 1e3 / SR as f64;
            let lead = r.min_lead_ms.map_or("-".to_string(), |v| format!("{v:.0} ms"));
            println!(
                "{:<48} {:<7} {:>9} {:>8} {:>9} {:>9} {:>8} {:>11.1} {:>9}",
                name,
                m,
                r.callbacks,
                r.dropouts(),
                r.overruns,
                r.ring_underruns,
                r.late,
                r.max_callback_ms,
                lead
            );
            out.push(json!({"sequence": name, "mode": m, "seconds": seconds, "callbacks": r.callbacks, "dropouts": r.dropouts(), "overruns": r.overruns, "late": r.late,
                "lostMs": lost_ms, "ringUnderruns": r.ring_underruns, "maxCallbackMs": r.max_callback_ms, "minLeadMs": r.min_lead_ms}));
        }
    }
    if let Some(p) = json_out
        && let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap_or_default())
    {
        eprintln!("{p}: {e}");
    }
}

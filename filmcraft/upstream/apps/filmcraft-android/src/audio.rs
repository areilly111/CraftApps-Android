//! cpal audio output on Android: the playback master clock.
//!
//! A lean port of the desktop's `apps/filmcraft/src/audio.rs`: the default
//! host's default output device (AAudio on Android), f32, the device's own
//! channel count and sample rate. No device probing or fallback ranking —
//! when no output device opens, playback runs without sound (the UI says so)
//! instead of failing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filmcraft_ui_egui::AudioOut;

/// Append a line to /sdcard/FilmCraft-audio-log.txt for debugging (user requested file logging).
fn audio_log(msg: &str) {
    use std::io::Write;
    if let Ok(external) = std::env::var("EXTERNAL_STORAGE") {
        let path = format!("{external}/FilmCraft-audio-log.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{msg}");
        }
    }
    log::info!("{msg}");
}

pub struct AndroidAudioOut {
    stream: Option<cpal::Stream>,
    played: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    rate: u32,
    channels: u16,
}

impl AndroidAudioOut {
    pub fn new() -> Self {
        // Query the device for its default config so sample_rate()/channels()
        // return valid values even before start() is called. This fixes the
        // chicken-and-egg where start_source_audio() checks sample_rate()==0
        // and returns early, preventing start() from ever being called.
        let (rate, channels) = cpal::default_host()
            .default_output_device()
            .and_then(|dev| dev.default_output_config().ok())
            .map(|cfg| (cfg.sample_rate().0, cfg.channels()))
            .unwrap_or((0, 0));
        AndroidAudioOut { stream: None, played: Arc::new(AtomicU64::new(0)), failed: Arc::new(AtomicBool::new(false)), rate, channels }
    }

    fn stream_error(failed: &Arc<AtomicBool>, e: cpal::StreamError) {
        log::error!("audio output error: {e}");
        failed.store(true, Ordering::SeqCst);
    }
}

impl Default for AndroidAudioOut {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioOut for AndroidAudioOut {
    fn start(&mut self, mut fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        self.stop();
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or("no audio output device")?;
        let cfg = dev.default_output_config().map_err(|e| format!("no output config: {e}"))?;
        let channels = cfg.channels() as usize;
        let rate = cfg.sample_rate().0;
        audio_log(&format!("audio: device channels={channels} rate={rate} format={:?}", cfg.sample_format()));
        if channels == 0 || rate == 0 {
            return Err("the output device returned an invalid channel count or sample rate".into());
        }
        let config: cpal::StreamConfig = cfg.clone().into();
        self.rate = rate;
        self.channels = cfg.channels();
        self.played.store(0, Ordering::SeqCst);
        self.failed.store(false, Ordering::SeqCst);
        let played = self.played.clone();
        let failed = self.failed.clone();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => {
                audio_log("audio: using F32 format");
                dev.build_output_stream(
                &config,
                move |buf: &mut [f32], _| {
                    fill(buf, channels);
                    // Log first few samples to see if there's actual audio data
                    let sum: f32 = buf.iter().take(100).map(|s| s.abs()).sum();
                    if sum > 0.001 {
                        log::debug!("audio: F32 callback, buf len={}, sum={sum}", buf.len());
                    }
                    played.fetch_add((buf.len() / channels.max(1)) as u64, Ordering::SeqCst);
                },
                move |e| Self::stream_error(&failed, e),
                None,
            )},
            // The device doesn't take f32: convert through an f32 staging buffer.
            cpal::SampleFormat::I16 => dev.build_output_stream(
                &config,
                move |buf: &mut [i16], _| {
                    let n = buf.len();
                    // Use a temporary f32 buffer for the fill callback
                    let mut tmp = vec![0.0f32; n];
                    fill(&mut tmp, channels);
                    for (o, &s) in buf.iter_mut().zip(tmp.iter()) {
                        *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                    }
                    played.fetch_add((n / channels.max(1)) as u64, Ordering::SeqCst);
                },
                move |e| Self::stream_error(&failed, e),
                None,
            ),
            cpal::SampleFormat::U16 => dev.build_output_stream(
                &config,
                move |buf: &mut [u16], _| {
                    let n = buf.len();
                    let mut tmp = vec![0.0f32; n];
                    fill(&mut tmp, channels);
                    for (o, &s) in buf.iter_mut().zip(tmp.iter()) {
                        *o = ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16;
                    }
                    played.fetch_add((n / channels.max(1)) as u64, Ordering::SeqCst);
                },
                move |e| Self::stream_error(&failed, e),
                None,
            ),
            fmt => {
                let err = format!("unsupported sample format {fmt:?}");
                return Err(err);
            }
        }
        .map_err(|e| format!("couldn't open the audio output: {e}"))?;
        stream.play().map_err(|e| format!("couldn't start audio output: {e}"))?;
        self.stream = Some(stream);
        audio_log(&format!("audio output: {channels} channels at {rate} Hz"));
        Ok(rate)
    }

    fn stop(&mut self) {
        // Dropping the stream stops the device (its callback owns `fill`).
        self.stream = None;
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn channels(&self) -> usize {
        self.channels.max(1) as usize
    }

    fn played_frames(&self) -> Option<u64> {
        if self.failed.load(Ordering::SeqCst) {
            return None;
        }
        Some(self.played.load(Ordering::SeqCst))
    }
}

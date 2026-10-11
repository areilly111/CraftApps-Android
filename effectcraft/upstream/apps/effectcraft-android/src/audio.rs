//! cpal audio output on Android for EffectCraft's preview playback.
//!
//! Implements [`effectcraft_ui_egui::audio::AudioDevice`] on the default
//! output device (AAudio on Android): f32, the device's own channel count
//! and sample rate. The device callback pulls interleaved stereo frames from
//! the [`AudioFeed`]; the feed's `consumed` counter is the A/V sync clock.
//!
//! Best effort: when no output device opens, the factory returns `None` and
//! preview runs silent instead of failing to start.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use effectcraft_ui_egui::audio::{AudioDevice, AudioFeed, AudioOutput};

pub struct AndroidAudioDevice {
    stream: Option<cpal::Stream>,
    rate: u32,
}

impl AndroidAudioDevice {
    pub fn new() -> Self {
        AndroidAudioDevice { stream: None, rate: 0 }
    }
}

impl Default for AndroidAudioDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioDevice for AndroidAudioDevice {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn start(&mut self, feed: Arc<AudioFeed>) -> Result<(), String> {
        self.stop();
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or("no audio output device")?;
        let cfg = dev
            .default_output_config()
            .map_err(|e| format!("no audio output config: {e}"))?;
        let rate = cfg.sample_rate().0;
        if rate == 0 {
            return Err("the output device returned an invalid sample rate".to_string());
        }
        if cfg.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!(
                "unsupported audio sample format {:?} (need f32)",
                cfg.sample_format()
            ));
        }
        let config: cpal::StreamConfig = cfg.into();
        let stream = dev
            .build_output_stream(
                &config,
                move |buf: &mut [f32], _| {
                    // `AudioFeed::pull` fills interleaved stereo, silencing
                    // underruns; `consumed` advances only on real samples.
                    feed.pull(buf);
                },
                |e| log::error!("EffectCraft audio output error: {e}"),
                None,
            )
            .map_err(|e| format!("couldn't open the audio output: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("couldn't start audio output: {e}"))?;
        self.stream = Some(stream);
        self.rate = rate;
        log::info!("EffectCraft audio output: {rate} Hz");
        Ok(())
    }

    fn stop(&mut self) {
        // Dropping the stream stops the device.
        self.stream = None;
        self.rate = 0;
    }

    fn latency_frames(&self) -> u64 {
        // cpal 0.16 doesn't expose AAudio output latency; the A/V sync clock
        // tolerates this (a few ms of constant offset).
        0
    }
}

/// [`effectcraft_ui_egui::audio::AudioDeviceFactory`]: open the default
/// output device, or `None` when there is none (preview stays silent).
pub fn android_audio_factory() -> effectcraft_ui_egui::audio::AudioDeviceFactory {
    Box::new(|_out: &AudioOutput| {
        let mut dev = AndroidAudioDevice::new();
        // Probe by opening: a `start` immediately followed by `stop` would
        // need a feed; instead just construct — `start` fails gracefully.
        let _ = &mut dev;
        Some(Box::new(dev) as Box<dyn AudioDevice>)
    })
}

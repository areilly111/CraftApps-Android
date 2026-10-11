//! Audio preview through Web Audio (`js/host.js`): an AudioWorklet (ScriptProcessorNode where
//! worklets are unavailable) plays the blocks the UI mixes ahead
//! ([`effectcraft_ui_egui::audio::AudioPlayback::pump`]).
//!
//! Every UI frame [`WebAudioOut::pump`] moves samples from the feed to the output so it stays
//! ~150 ms ahead of what has played. A/V sync follows `AudioContext.currentTime`: frames played
//! = (currentTime − start) × rate, so the device latency the playback clock subtracts is
//! "handed over but not played yet" plus the context's output latency.
//!
//! Browsers keep the AudioContext suspended until the user interacts with the page; the host
//! resumes it on the first pointer or key event.

use std::sync::Arc;

use effectcraft_ui_egui::audio::{AudioDevice, AudioFeed};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/js/host.js")]
extern "C" {
    #[wasm_bindgen(js_name = audioInstallUnlock)]
    pub fn install_unlock();
    #[wasm_bindgen(js_name = audioState)]
    pub fn state() -> JsValue;
    #[wasm_bindgen(js_name = audioSampleRate)]
    fn sample_rate() -> f64;
    #[wasm_bindgen(js_name = audioStart)]
    fn audio_start(worklet_url: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = audioPush)]
    fn audio_push(samples: js_sys::Float32Array);
    #[wasm_bindgen(js_name = audioPlayed)]
    fn audio_played() -> f64;
    #[wasm_bindgen(js_name = audioOutputLatency)]
    fn audio_output_latency() -> f64;
    #[wasm_bindgen(js_name = audioStop)]
    fn audio_stop();
}

/// How far ahead of the playhead the output is kept fed.
const LEAD_SECONDS: f64 = 0.15;

pub struct WebAudioOut {
    rate: u32,
    feed: Option<Arc<AudioFeed>>,
    /// Sample frames handed to the output since start.
    posted: u64,
    buf: Vec<f32>,
}

/// The Settings ▸ Audio factory: Web Audio's default output (`None` without Web Audio).
pub fn open(_: &effectcraft_ui_egui::audio::AudioOutput) -> Option<Box<dyn AudioDevice>> {
    let rate = sample_rate();
    (rate > 0.0).then(|| Box::new(WebAudioOut { rate: rate as u32, feed: None, posted: 0, buf: Vec::new() }) as Box<dyn AudioDevice>)
}

fn worklet_url() -> String {
    crate::page_url("audio-worklet.js")
}

impl AudioDevice for WebAudioOut {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn start(&mut self, feed: Arc<AudioFeed>) -> Result<(), String> {
        self.feed = Some(feed);
        self.posted = 0;
        let p = audio_start(&worklet_url());
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = wasm_bindgen_futures::JsFuture::from(p).await {
                log::warn!("audio output: {e:?}");
            }
        });
        Ok(())
    }

    fn stop(&mut self) {
        self.feed = None;
        audio_stop();
    }

    fn latency_frames(&self) -> u64 {
        let played = audio_played() as u64;
        self.posted.saturating_sub(played) + audio_output_latency() as u64
    }

    fn pump(&mut self) {
        let Some(feed) = &self.feed else { return };
        let played = audio_played() as u64;
        let lead = (LEAD_SECONDS * self.rate as f64) as u64;
        let want = (played + lead).saturating_sub(self.posted) as usize;
        // Only real samples: the output plays silence by itself when it runs dry.
        let n = want.min(feed.queued_frames());
        if n == 0 {
            return;
        }
        self.buf.resize(n * 2, 0.0);
        feed.pull(&mut self.buf);
        audio_push(js_sys::Float32Array::from(&self.buf[..]));
        self.posted += n as u64;
    }
}

impl Drop for WebAudioOut {
    fn drop(&mut self) {
        if self.feed.is_some() {
            audio_stop();
        }
    }
}

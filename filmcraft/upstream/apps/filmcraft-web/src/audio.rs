//! WebAudio output: an `AudioWorklet` plays blocks that the UI thread mixes ahead.
//!
//! The mixer runs on the UI thread (wasm has no audio thread of its own here): every frame
//! [`Handle::pump`] mixes until ~250 ms are queued and posts the blocks to the worklet
//! (`web/audio-worklet.js`). The worklet counts the frames it actually played and reports them
//! with its `currentTime`; [`AudioOut::played_frames`] extrapolates from the last report on the
//! context's clock and never runs past what was queued. So, as on the desktop, playback follows
//! the audio clock: if the worklet starves, the playhead waits for it.

use std::cell::RefCell;
use std::rc::Rc;

use filmcraft_ui_egui::AudioOut;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Seconds of audio kept queued ahead of the worklet.
const AHEAD_S: f64 = 0.25;
/// Frames per posted block.
const BLOCK: usize = 2048;

struct State {
    ctx: web_sys::AudioContext,
    node: Option<web_sys::AudioWorkletNode>,
    fill: Option<Box<dyn FnMut(&mut [f32], usize) + Send>>,
    generation: u32,
    /// Frames posted since `start`.
    queued: u64,
    /// Last report from the worklet: frames played and its `currentTime`.
    played: u64,
    played_at: f64,
    reported: bool,
}

/// Shared handle: one is the app's [`AudioOut`], one is pumped by the host every frame.
#[derive(Clone)]
pub struct Handle(Rc<RefCell<State>>);

impl Handle {
    /// Create the audio context and load the worklet (asynchronously). None without WebAudio.
    pub fn new() -> Option<Self> {
        let opts = web_sys::AudioContextOptions::new();
        opts.set_latency_hint(&"interactive".into());
        let ctx = web_sys::AudioContext::new_with_context_options(&opts).ok()?;
        let h = Handle(Rc::new(RefCell::new(State {
            ctx: ctx.clone(),
            node: None,
            fill: None,
            generation: 0,
            queued: 0,
            played: 0,
            played_at: 0.0,
            reported: false,
        })));
        let weak = Rc::downgrade(&h.0);
        wasm_bindgen_futures::spawn_local(async move {
            let r: Result<web_sys::AudioWorkletNode, JsValue> = async {
                JsFuture::from(ctx.audio_worklet()?.add_module("audio-worklet.js")?).await?;
                let o = web_sys::AudioWorkletNodeOptions::new();
                o.set_number_of_inputs(0);
                o.set_number_of_outputs(1);
                o.set_output_channel_count(&js_sys::Array::of1(&2.into()));
                let node = web_sys::AudioWorkletNode::new_with_options(&ctx, "filmcraft-output", &o)?;
                node.connect_with_audio_node(&ctx.destination())?;
                Ok(node)
            }
            .await;
            let Some(st) = weak.upgrade() else { return };
            match r {
                Ok(node) => {
                    let w = Rc::downgrade(&st);
                    let on_msg = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                        let Some(st) = w.upgrade() else { return };
                        let d = e.data();
                        let num = |k: &str| js_sys::Reflect::get(&d, &k.into()).ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let mut s = st.borrow_mut();
                        if num("gen") as u32 == s.generation {
                            s.played = num("played") as u64;
                            s.played_at = num("time");
                            s.reported = true;
                        }
                    });
                    if let Ok(port) = node.port() {
                        port.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
                    }
                    on_msg.forget();
                    st.borrow_mut().node = Some(node);
                    crate::set_info("audioWorklet", serde_json::json!(true));
                }
                Err(e) => {
                    log::warn!("audio worklet unavailable: {e:?}");
                    crate::set_info("audioWorklet", serde_json::json!(false));
                }
            }
        });
        // Browsers start audio contexts suspended until the user interacts with the page.
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            let c = h.0.borrow().ctx.clone();
            let resume = Closure::<dyn FnMut()>::new(move || {
                if c.state() != web_sys::AudioContextState::Running {
                    let _ = c.resume();
                }
            });
            for ev in ["pointerdown", "keydown"] {
                let _ = doc.add_event_listener_with_callback_and_bool(ev, resume.as_ref().unchecked_ref(), true);
            }
            resume.forget();
        }
        Some(h)
    }

    fn post(node: &web_sys::AudioWorkletNode, msg: &JsValue, transfer: Option<&JsValue>) {
        if let Ok(port) = node.port() {
            let _ = match transfer {
                Some(t) => port.post_message_with_transferable(msg, &js_sys::Array::of1(t)),
                None => port.post_message(msg),
            };
        }
    }

    fn estimate(s: &State) -> u64 {
        if !s.reported {
            return 0;
        }
        let sr = s.ctx.sample_rate() as f64;
        let ahead = ((s.ctx.current_time() - s.played_at).max(0.0) * sr) as u64;
        (s.played + ahead).min(s.queued)
    }

    /// Mix and post audio until [`AHEAD_S`] is queued (called every frame).
    pub fn pump(&self) {
        let mut s = self.0.borrow_mut();
        let Some(node) = s.node.clone() else { return };
        if s.fill.is_none() {
            return;
        }
        let target = (AHEAD_S * s.ctx.sample_rate() as f64) as u64;
        let mut guard = 0;
        while s.queued.saturating_sub(Self::estimate(&s)) < target && guard < 16 {
            guard += 1;
            let mut buf = vec![0f32; BLOCK * 2];
            if let Some(f) = s.fill.as_mut() {
                f(&mut buf, 2);
            }
            let arr = js_sys::Float32Array::from(&buf[..]);
            let msg = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&msg, &"type".into(), &"data".into());
            let _ = js_sys::Reflect::set(&msg, &"gen".into(), &s.generation.into());
            let _ = js_sys::Reflect::set(&msg, &"samples".into(), &arr);
            Self::post(&node, &msg, Some(&arr.buffer()));
            s.queued += BLOCK as u64;
        }
    }

    fn reset(s: &mut State) {
        s.generation = s.generation.wrapping_add(1);
        s.queued = 0;
        s.played = 0;
        s.reported = false;
        if let Some(node) = &s.node {
            let msg = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&msg, &"type".into(), &"reset".into());
            let _ = js_sys::Reflect::set(&msg, &"gen".into(), &s.generation.into());
            Self::post(node, &msg, None);
        }
    }
}

impl AudioOut for Handle {
    fn start(&mut self, fill: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        {
            let mut s = self.0.borrow_mut();
            if s.node.is_none() {
                return Err("audio worklet not loaded".into());
            }
            if s.ctx.state() != web_sys::AudioContextState::Running {
                let _ = s.ctx.resume();
                // without a user gesture the context stays suspended: use the wall clock
                return Err("audio context suspended (no user gesture yet)".into());
            }
            Self::reset(&mut s);
            s.fill = Some(fill);
        }
        self.pump();
        Ok(self.sample_rate())
    }

    fn stop(&mut self) {
        let mut s = self.0.borrow_mut();
        s.fill = None;
        Self::reset(&mut s);
    }

    fn sample_rate(&self) -> u32 {
        self.0.borrow().ctx.sample_rate() as u32
    }

    fn played_frames(&self) -> Option<u64> {
        let s = self.0.borrow();
        s.fill.as_ref().map(|_| Self::estimate(&s))
    }
}

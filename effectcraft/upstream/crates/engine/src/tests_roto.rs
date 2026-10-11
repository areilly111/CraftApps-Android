//! Roto Brush & Refine Edge end to end through the commands, on synthetic footage: a textured
//! disk moving over a textured background.

use effectcraft_project::LayerId;
use effectcraft_raster::Image;
use effectcraft_render::RenderOpts;
use effectcraft_track::roto::rle;
use serde_json::{Value, json};

use crate::Session;
use crate::tests_track::{H, W, frame_time, setup};

/// Keep cache/model mutation tests from clearing segmentations under another fixture.
/// Libtest runs each test on its own thread; the guard lives until that thread exits.
pub(crate) fn hold_roto_cache_test_lock() {
    use std::cell::RefCell;
    use std::sync::{Mutex, MutexGuard, PoisonError};
    static LOCK: Mutex<()> = Mutex::new(());
    thread_local! {
        static HELD: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
    }
    HELD.with(|h| {
        if h.borrow().is_none() {
            *h.borrow_mut() = Some(LOCK.lock().unwrap_or_else(PoisonError::into_inner));
        }
    });
}
const R: f64 = 40.0;

fn noise(x: f64, y: f64, seed: u32) -> f64 {
    fn h(i: i64, j: i64, s: u32) -> f64 {
        let mut v = (i.wrapping_mul(73856093) ^ j.wrapping_mul(19349663) ^ (s as i64).wrapping_mul(83492791)) as u64;
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1e995);
        v ^= v >> 15;
        (v % 10_000) as f64 / 10_000.0
    }
    let mut out = 0.0;
    for (cell, amp) in [(9.0, 0.6), (3.5, 0.4)] {
        let (fx, fy) = (x / cell, y / cell);
        let (i, j) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let a = h(i, j, seed) + (h(i + 1, j, seed) - h(i, j, seed)) * sx;
        let b = h(i, j + 1, seed) + (h(i + 1, j + 1, seed) - h(i, j + 1, seed)) * sx;
        out += amp * (a + (b - a) * sy);
    }
    out
}

/// Disk centre on frame `f`.
fn center(f: u32) -> [f64; 2] {
    let t = f as f64;
    [90.0 + 4.0 * t, 110.0 + 1.5 * t + 5.0 * (t * 0.3).sin()]
}

/// Disk centre with a jump at frame 8 (propagation loses it there).
fn jumping(f: u32) -> [f64; 2] {
    if f < 8 { center(f) } else { [230.0 + 2.0 * (f - 8) as f64, 150.0] }
}

fn draw(c: [f64; 2]) -> Image {
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            let d = (p[0] - c[0]).hypot(p[1] - c[1]);
            let a = (R + 0.5 - d).clamp(0.0, 1.0) as f32;
            let n = noise(p[0] - c[0], p[1] - c[1], 11);
            let f = [(0.75 + 0.25 * n) as f32, (0.35 + 0.3 * n) as f32, (0.1 + 0.15 * n) as f32];
            let (bn, bm) = (noise(p[0], p[1], 5), noise(p[0] * 1.7, p[1] * 1.7, 9));
            let b = [(0.05 + 0.25 * bm) as f32, (0.3 + 0.4 * bn) as f32, (0.45 + 0.4 * bm) as f32];
            img.set(x, y, [f[0] * a + b[0] * (1.0 - a), f[1] * a + b[1] * (1.0 - a), f[2] * a + b[2] * (1.0 - a), 1.0]);
        }
    }
    img
}

fn gt(c: [f64; 2]) -> String {
    let m: Vec<u8> = (0..W * H).map(|i| (((i % W) as f64 + 0.5 - c[0]).hypot((i / W) as f64 + 0.5 - c[1]) <= R) as u8).collect();
    rle::encode(&m)
}

fn line(a: [f64; 2], b: [f64; 2]) -> Value {
    json!((0..=8).map(|k| [a[0] + (b[0] - a[0]) * k as f64 / 8.0, a[1] + (b[1] - a[1]) * k as f64 / 8.0]).collect::<Vec<_>>())
}

/// A session with the clip (the Track solid removed) and foreground / background strokes on
/// frame 0. `name`: the footage file name (segmentations are cached process-wide by content key,
/// which includes the footage's identity: different clips need different names).
fn painted(path: fn(u32) -> [f64; 2], name: &str) -> (Session, LayerId, u64) {
    hold_roto_cache_test_lock();
    let (mut s, clip, solid) = setup(move |f| draw(path(f)));
    let cid = s.active_comp_id().unwrap();
    s.edit("remove solid", None, |p, _| {
        p.comp_mut(cid).unwrap().layers.retain(|l| l.id != solid);
        for it in p.items.values_mut() {
            if let effectcraft_project::ItemKind::Footage(f) = &mut it.kind {
                f.path = name.into();
            }
        }
        Ok(())
    })
    .unwrap();
    s.execute("time.set", json!({"time": 0})).unwrap();
    let c = path(0);
    let r =
        s.execute_checked("roto.stroke", json!({"layer": clip.0, "points": line([c[0] - 22.0, c[1] - 8.0], [c[0] + 22.0, c[1] + 8.0]), "radius": 6})).unwrap();
    let uid = r["effect"].as_u64().unwrap();
    assert_eq!(r["base"], 0);
    assert_eq!(r["span"], json!([0, 20]));
    for (a, b) in [([c[0] - 60.0, c[1] - 58.0], [c[0] + 70.0, c[1] - 58.0]), ([c[0] + 70.0, c[1] + 58.0], [c[0] - 60.0, c[1] + 58.0])] {
        s.execute_checked("roto.stroke", json!({"layer": clip.0, "kind": "bg", "points": line(a, b), "radius": 6})).unwrap();
    }
    (s, clip, uid)
}

/// Drop an instance's derived segmentations from the process-wide cache.
fn forget(s: &Session, clip: LayerId, uid: u64) {
    let comp = s.active_comp().unwrap();
    let l = comp.layer(clip).unwrap();
    let g = l.props.find_group(uid).unwrap();
    let size = crate::roto::layer_size(&s.project, comp, l);
    let ch = effectcraft_effects::roto::Chain::new(&crate::roto::params_static(g), size, comp.frame_rate.as_f64(), 1.0);
    effectcraft_effects::roto::forget(ch.keys.values().copied());
}

fn iou_at(s: &mut Session, clip: LayerId, f: u32, c: [f64; 2]) -> f64 {
    let st = s.execute("roto.status", json!({"layer": clip.0, "frame": f, "compute": true, "compareTo": gt(c)})).unwrap();
    st["iou"].as_f64().unwrap_or_else(|| panic!("no matte on frame {f}: {st}"))
}

fn alpha_at(s: &Session, f: u32) -> Vec<f32> {
    let img = s.render(s.active_comp_id().unwrap(), frame_time(f), RenderOpts { scale: 1.0, ..Default::default() });
    img.data.iter().map(|p| p[3]).collect()
}

#[test]
fn roto_brush_segments_and_propagates() {
    let (mut s, clip, uid) = painted(center, "disk-a.mov");
    let l = s.active_comp().unwrap().layer(clip).unwrap().clone();
    assert_eq!(l.effects().unwrap().groups().filter(|g| crate::roto::is_roto(g)).count(), 1, "one effect for all strokes");
    let j0 = iou_at(&mut s, clip, 0, center(0));
    assert!(j0 > 0.95, "base frame IoU {j0}");
    // Propagate the span (blocking).
    let r = s.execute_checked("roto.propagate", json!({"layer": clip.0, "direction": "both", "wait": true})).unwrap();
    assert_eq!(r["frames"], 21, "{r}");
    assert_eq!(r["status"]["computed"].as_array().unwrap().len(), 21);
    let mut worst: f64 = 1.0;
    for f in 0..=20 {
        worst = worst.min(iou_at(&mut s, clip, f, center(f)));
    }
    assert!(worst > 0.9, "worst IoU over the span {worst}");
    // The render applies the matte; outside the span the layer is transparent.
    let a = alpha_at(&s, 12);
    let c = center(12);
    let mut wrong = 0;
    for (i, v) in a.iter().enumerate() {
        let d = (((i as u32 % W) as f64 + 0.5 - c[0]).hypot((i as u32 / W) as f64 + 0.5 - c[1])) - R;
        if (d < -3.0 && *v < 0.5) || (d > 3.0 && *v > 0.5) {
            wrong += 1;
        }
    }
    assert!(wrong < 150, "{wrong} wrong pixels in the render");
    assert!(alpha_at(&s, 23).iter().all(|v| *v == 0.0));
    // Extending the span propagates further.
    let r = s.execute("roto.propagate", json!({"layer": clip.0, "direction": "forward", "to": 24, "wait": true})).unwrap();
    assert_eq!(r["status"]["span"], json!([0, 24]));
    assert!(iou_at(&mut s, clip, 24, center(24)) > 0.9);
    // Undo / redo of strokes and the span.
    let n = |s: &Session| {
        let st = s.active_comp().unwrap().layer(clip).unwrap().props.find_group(uid).unwrap().get(effectcraft_effects::roto::STROKES).unwrap().value.clone();
        match st {
            effectcraft_keyframe::Value::Str(j) => effectcraft_track::roto::RotoData::from_json(&j),
            _ => panic!(),
        }
    };
    assert_eq!(n(&s).span, [0, 24]);
    assert!(s.undo());
    assert_eq!(n(&s).span, [0, 20]);
    assert_eq!(n(&s).strokes.len(), 3);
    assert!(s.undo());
    assert_eq!(n(&s).strokes.len(), 2);
    assert!(s.redo());
    assert!(s.redo());
    assert_eq!(n(&s).strokes.len(), 3);
    assert_eq!(n(&s).span, [0, 24]);
    // Serde keeps the strokes; the reopened project renders the same matte.
    let back = effectcraft_project::Project::from_json(&s.project.to_json()).unwrap();
    let g = back.comp(s.active_comp_id().unwrap()).unwrap().layer(clip).unwrap().props.find_group(uid).unwrap().clone();
    assert_eq!(
        crate::roto::params_static(&g).get("strokes"),
        crate::roto::params_static(s.active_comp().unwrap().layer(clip).unwrap().props.find_group(uid).unwrap()).get("strokes")
    );
    // Bad parameters are reported.
    assert!(s.execute_checked("roto.stroke", json!({"layer": clip.0, "kind": "sideways", "points": [[1, 1]]})).is_err());
    assert!(s.execute_checked("roto.propagate", json!({"layer": clip.0, "bogus": 1})).is_err());
    assert!(s.execute_checked("roto.stroke", json!({"layer": clip.0, "points": [[1, 1]], "frame": 99})).is_err());
}

#[test]
fn correction_strokes_restart_propagation() {
    let (mut s, clip, _) = painted(jumping, "disk-jump.mov");
    s.execute("roto.propagate", json!({"layer": clip.0, "wait": true})).unwrap();
    assert!(iou_at(&mut s, clip, 6, jumping(6)) > 0.9);
    let bad = iou_at(&mut s, clip, 8, jumping(8));
    assert!(bad < 0.5, "the jump should break propagation ({bad})");
    // Correct frame 8: paint the disk in and its old place out.
    let (c, old) = (jumping(8), center(8));
    s.execute("roto.stroke", json!({"layer": clip.0, "frame": 8, "points": line([c[0] - 20.0, c[1]], [c[0] + 20.0, c[1]]), "radius": 6})).unwrap();
    s.execute(
        "roto.stroke",
        json!({"layer": clip.0, "frame": 8, "kind": "bg", "points": line([old[0] - 15.0, old[1] - 15.0], [old[0] + 15.0, old[1] + 15.0]), "radius": 8}),
    )
    .unwrap();
    let st = s.execute("roto.status", json!({"layer": clip.0})).unwrap();
    let computed: Vec<i64> = serde_json::from_value(st["computed"].clone()).unwrap();
    assert!(computed.contains(&7) && !computed.contains(&8) && !computed.contains(&9), "frames from the correction on are recomputed: {computed:?}");
    assert_eq!(st["strokeFrames"], json!([0, 8]));
    s.execute("roto.propagate", json!({"layer": clip.0, "direction": "forward", "wait": true})).unwrap();
    for f in 8..=20 {
        let j = iou_at(&mut s, clip, f, jumping(f));
        assert!(j > 0.9, "frame {f}: IoU {j}");
    }
    // Clearing the correction brings the bad frame back.
    let r = s.execute("roto.clearStrokes", json!({"layer": clip.0, "frame": 8})).unwrap();
    assert_eq!(r["removed"], 2);
    assert!(iou_at(&mut s, clip, 8, jumping(8)) < 0.5);
}

#[test]
fn freeze_caches_the_mattes() {
    let (mut s, clip, uid) = painted(center, "disk-freeze.mov");
    // A refine band along the edge on the base frame.
    let c = center(0);
    let ring: Vec<[f64; 2]> = (0..=40).map(|k| k as f64 / 40.0 * std::f64::consts::TAU).map(|a| [c[0] + R * a.cos(), c[1] + R * a.sin()]).collect();
    s.execute_checked("roto.stroke", json!({"layer": clip.0, "kind": "refine", "points": ring, "radius": 5})).unwrap();
    s.execute("roto.span", json!({"layer": clip.0, "end": 3})).unwrap();
    let live: Vec<Vec<f32>> = (0..=3).map(|f| alpha_at(&s, f)).collect();
    // Freeze in the background and wait for it.
    let r = s.execute_checked("roto.freeze", json!({"layer": clip.0})).unwrap();
    assert_eq!(r["frames"], 4);
    let t0 = std::time::Instant::now();
    while s.roto_job.is_some() {
        assert!(t0.elapsed().as_secs() < 300);
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.poll_roto(false);
    }
    let st = s.execute("roto.status", json!({"layer": clip.0})).unwrap();
    assert_eq!(st["frozen"], true, "{st}");
    // Frozen renders match the live ones (8-bit alpha), even without the derived segmentations.
    forget(&s, clip, uid);
    s.layer_cache = Default::default();
    for (f, want) in live.iter().enumerate() {
        let got = alpha_at(&s, f as u32);
        let worst = got.iter().zip(want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst <= 1.0 / 255.0 + 1e-4, "frame {f}: {worst}");
    }
    assert!(st["computed"].as_array().unwrap().len() >= 4);
    // Frozen effects refuse strokes; Unfreeze is undoable.
    assert!(s.execute("roto.stroke", json!({"layer": clip.0, "points": [[10, 10]]})).is_err());
    s.execute("roto.unfreeze", json!({"layer": clip.0})).unwrap();
    assert_eq!(s.execute("roto.status", json!({"layer": clip.0})).unwrap()["frozen"], false);
    assert!(s.undo());
    assert_eq!(s.execute("roto.status", json!({"layer": clip.0})).unwrap()["frozen"], true);
    // Changing the source frames (the layer's timing) drops the frozen mattes.
    let cid = s.active_comp_id().unwrap();
    s.edit("shift", None, |p, _| {
        let l = p.comp_mut(cid).unwrap().layer_mut(clip).unwrap();
        l.start_time = frame_time(1);
        Ok(())
    })
    .unwrap();
    assert_eq!(s.execute("roto.status", json!({"layer": clip.0, "effect": uid})).unwrap()["frozen"], false);
}

#[test]
fn propagation_is_deterministic_and_runs_in_background() {
    let (mut s, clip, uid) = painted(center, "disk-bg.mov");
    s.execute("roto.span", json!({"layer": clip.0, "end": 5})).unwrap();
    s.execute("roto.propagate", json!({"layer": clip.0})).unwrap();
    let t0 = std::time::Instant::now();
    while s.roto_job.is_some() {
        assert!(t0.elapsed().as_secs() < 300);
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.poll_roto(false);
    }
    let st = s.execute("roto.status", json!({"layer": clip.0})).unwrap();
    assert_eq!(st["computed"], json!([0, 1, 2, 3, 4, 5]), "{st}");
    let a = s.execute("roto.status", json!({"layer": clip.0, "frame": 5, "matte": true})).unwrap()["matte"].clone();
    // Recomputing from scratch gives the same matte.
    forget(&s, clip, uid);
    let b = s.execute("roto.status", json!({"layer": clip.0, "frame": 5, "matte": true})).unwrap()["matte"].clone();
    assert!(a.is_string());
    assert_eq!(a, b);
    // Tool options.
    let o = s.execute_checked("roto.options", json!({"diameter": 30, "view": "alphaOverlay"})).unwrap();
    assert_eq!(o["view"], "alphaOverlay");
    assert_eq!(s.state.roto.diameter, 30.0);
    assert!(s.execute("roto.options", json!({"view": "sideways"})).is_err());
    // The default radius follows the tool diameter.
    let r = s.execute("roto.stroke", json!({"layer": clip.0, "points": [[90, 110]]})).unwrap();
    assert_eq!(r["radius"], 15.0);
}

/// Runs offloaded jobs at once in a second session sharing the footage, through JSON (what the
/// browser's Web Worker does, minus the thread).
struct InlineWorker(std::sync::Arc<dyn effectcraft_render::FootageSource>);

impl crate::offload::Offload for InlineWorker {
    fn start(&self, req: crate::offload::WorkerRequest, inbox: std::sync::Arc<crate::offload::Inbox>) -> Result<(), String> {
        let req: crate::offload::WorkerRequest = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        let mut w = Session { footage: self.0.clone(), ..Default::default() };
        let post: crate::offload::Post = std::rc::Rc::new(move |r| inbox.push(serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap()));
        crate::offload::run_request(&mut w, req, &post);
        Ok(())
    }
    fn cancel(&self, _: u64) {}
}

#[test]
fn offloaded_propagation_streams_segmentations_back() {
    let (mut s, clip, uid) = painted(center, "disk-offload.mov");
    s.execute("roto.span", json!({"layer": clip.0, "end": 5})).unwrap();
    s.offload = Some(std::sync::Arc::new(InlineWorker(s.footage.clone())));
    let r = s.execute("roto.propagate", json!({"layer": clip.0})).unwrap();
    assert_eq!(r["frames"], 6, "{r}");
    assert!(s.roto_job.is_none(), "offloaded, not on a thread");
    assert!(s.is_roto_running());
    assert_eq!(s.roto_progress().unwrap().task, Some(crate::roto::RotoTask::Propagate));
    assert_eq!(s.roto_target().map(|t| (t.1, t.2)), Some((clip, uid)));
    assert!(s.jobs().iter().any(|j| j.id == "roto"));
    // The worker has its own segmentation cache: here nothing is computed until its replies
    // are applied.
    forget(&s, clip, uid);
    assert_eq!(s.execute("roto.status", json!({"layer": clip.0})).unwrap()["computed"], json!([]));
    let rev = s.revision;
    assert!(s.poll_offload());
    assert!(!s.is_roto_running());
    assert!(s.revision > rev, "frames are redrawn");
    let st = s.execute("roto.status", json!({"layer": clip.0})).unwrap();
    assert_eq!(st["computed"], json!([0, 1, 2, 3, 4, 5]), "{st}");
    assert!(s.events.iter().any(|e| matches!(e, crate::Event::Toast { message, error: false } if message.contains("propagated 6 frame"))));
    // The matte matches one computed here.
    let a = s.execute("roto.status", json!({"layer": clip.0, "frame": 5, "matte": true})).unwrap()["matte"].clone();
    forget(&s, clip, uid);
    let b = s.execute("roto.status", json!({"layer": clip.0, "frame": 5, "matte": true})).unwrap()["matte"].clone();
    assert!(a.is_string());
    assert_eq!(a, b);
    // `wait` still runs here.
    s.execute("roto.propagate", json!({"layer": clip.0, "wait": true})).unwrap();
    assert!(s.offloaded.is_empty());
}

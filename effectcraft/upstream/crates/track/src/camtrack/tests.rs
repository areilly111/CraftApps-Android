//! Synthetic scenes: random 3D points (or a plane) seen by a known moving camera with a known
//! focal length, projected directly (with sub-pixel noise) into feature tracks.

use super::geometry::Rng;
use super::linalg::*;
use super::*;

const W: f64 = 1280.0;
const H: f64 = 720.0;

struct Truth {
    /// Per frame: (R world→cam, centre, focal).
    cams: Vec<(M3, V3, f64)>,
}

fn noise(rng: &mut Rng) -> f64 {
    // Sum of uniforms ≈ Gaussian, σ ≈ 0.1 px.
    let u: f64 = (0..4).map(|_| (rng.next_u64() % 10_000) as f64 / 10_000.0 - 0.5).sum();
    u * 0.17
}

/// Project `pts` through the cameras into tracks (a track ends when its point leaves the frame;
/// re-entering starts a new track).
fn tracks_of(pts: &[V3], t: &Truth, seed: u64, noise_on: bool) -> CameraTracks {
    let mut rng = Rng(seed);
    let mut tracks: Vec<Track2D> = vec![];
    for x in pts {
        let mut cur: Option<Track2D> = None;
        for (k, (r, c, f)) in t.cams.iter().enumerate() {
            let p = mv(r, sub(*x, *c));
            let uv = (p[2] > 0.1).then(|| [f * p[0] / p[2] + W / 2.0, f * p[1] / p[2] + H / 2.0]);
            match uv.filter(|u| u[0] >= 2.0 && u[1] >= 2.0 && u[0] < W - 2.0 && u[1] < H - 2.0) {
                Some(u) => {
                    let (nx, ny) = if noise_on { (noise(&mut rng), noise(&mut rng)) } else { (0.0, 0.0) };
                    let q = [(u[0] + nx) as f32, (u[1] + ny) as f32];
                    match &mut cur {
                        Some(tr) => tr.pts.push(q),
                        None => cur = Some(Track2D { id: 0, start: k as u32, pts: vec![q] }),
                    }
                }
                None => {
                    if let Some(tr) = cur.take() {
                        tracks.push(tr);
                    }
                }
            }
        }
        if let Some(tr) = cur.take() {
            tracks.push(tr);
        }
    }
    tracks.retain(|t| t.pts.len() >= 3);
    for (i, tr) in tracks.iter_mut().enumerate() {
        tr.id = i as u32;
    }
    CameraTracks { version: 1, size: [W, H], start: 0.0, frame_duration: 1.0 / 30.0, frames: t.cams.len() as u32, factor: 1.0, detailed: false, tracks }
}

fn random_points(n: usize, seed: u64, center: V3, ext: V3) -> Vec<V3> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|_| {
            let mut u = || (rng.next_u64() % 100_000) as f64 / 100_000.0 - 0.5;
            [center[0] + u() * ext[0], center[1] + u() * ext[1], center[2] + u() * ext[2]]
        })
        .collect()
}

/// A camera dollying sideways and forward while panning to keep the scene in view.
fn moving_camera(frames: usize, f: impl Fn(usize) -> f64) -> Truth {
    let cams = (0..frames)
        .map(|k| {
            let t = k as f64 / (frames - 1) as f64;
            let c = [-3.0 + 6.0 * t, -0.6 + 0.8 * t + 0.5 * (std::f64::consts::PI * t).sin(), -1.0 + 2.0 * t - 1.5 * (t - 0.5) * (t - 0.5)];
            // Look at the scene centre (0, 0, 10).
            let fwd = normalize(sub([0.3 * t, 0.2, 10.0], c));
            let right = normalize(cross([0.0, 1.0, 0.0], fwd));
            let down = cross(fwd, right);
            let r = [right, down, fwd];
            // A little roll.
            let roll = rodrigues([0.0, 0.0, 0.05 * (t * 6.0).sin()]);
            (mm(&roll, &r), c, f(k))
        })
        .collect();
    Truth { cams }
}

/// Align the solved centres to the true ones (similarity) and return the RMS path error
/// relative to the path length, plus the worst rotation error (degrees).
fn path_error(s: &CameraSolve, t: &Truth) -> (f64, f64) {
    let solved: Vec<usize> = (0..s.frames.len()).filter(|k| s.frames[*k].solved).collect();
    let src: Vec<V3> = solved.iter().map(|k| s.frames[*k].center).collect();
    let dst: Vec<V3> = solved.iter().map(|k| t.cams[*k].1).collect();
    let sim = umeyama(&src, &dst).expect("alignment");
    let len = norm(sub(dst[0], dst[dst.len() - 1])).max(1e-9);
    let rms = (src.iter().zip(&dst).map(|(a, b)| dot(sub(sim.apply(*a), *b), sub(sim.apply(*a), *b))).sum::<f64>() / src.len() as f64).sqrt();
    // Rotation: R_true ≈ R_solved · simᵀ.
    let mut worst: f64 = 0.0;
    for k in &solved {
        let rs = mm(&s.frames[*k].r(), &mt(&sim.r));
        let e = norm(log_rot(&mm(&rs, &mt(&t.cams[*k].0)))).to_degrees();
        worst = worst.max(e);
    }
    (rms / len, worst)
}

fn settings(shot: ShotType, method: SolveMethod) -> SolveSettings {
    SolveSettings { shot, hfov: 0.0, method, deleted: vec![], lens_distortion: false }
}

#[test]
fn fixed_angle_solve_recovers_path_and_focal() {
    let f = 1100.0;
    let t = moving_camera(40, |_| f);
    let pts = random_points(400, 1, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    let tr = tracks_of(&pts, &t, 2, true);
    let t0 = std::time::Instant::now();
    let s = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Typical), None).expect("solve");
    eprintln!("fixed: {:?} err {:.3} px focal {:.1} in {:?}", s.method_used, s.average_error, s.focal(), t0.elapsed());
    assert!(s.frames.iter().all(|c| c.solved));
    assert!(s.average_error < 0.5, "{}", s.average_error);
    assert!((s.focal() - f).abs() / f < 0.02, "focal {}", s.focal());
    let (pe, re) = path_error(&s, &t);
    assert!(pe < 0.01, "path {pe}");
    assert!(re < 0.5, "rotation {re}");
    // Canonical frame: first camera at the origin.
    assert!(norm(s.frames[0].center) < 1e-9 && norm(s.frames[0].rot) < 1e-9);
    // Serde round trip.
    let j = s.to_json();
    assert_eq!(CameraSolve::from_json(&j).unwrap().points.len(), s.points.len());
}

#[test]
fn auto_detect_picks_a_general_model_for_a_translating_camera() {
    let f = 900.0;
    let t = moving_camera(30, |_| f);
    let pts = random_points(300, 3, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    let tr = tracks_of(&pts, &t, 4, true);
    let s = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Auto), None).expect("solve");
    assert_eq!(s.method_used, SolveMethod::Typical);
    assert!(s.average_error < 0.5);
    assert!((s.focal() - f).abs() / f < 0.02, "focal {}", s.focal());
}

#[test]
fn specified_angle_of_view_is_kept() {
    let f = 1000.0;
    let t = moving_camera(25, |_| f);
    let pts = random_points(300, 5, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    let tr = tracks_of(&pts, &t, 6, true);
    let hfov = 2.0 * (W * 0.5 / f).atan().to_degrees();
    let s =
        solve(&tr, &SolveSettings { shot: ShotType::SpecifyAngle, hfov, method: SolveMethod::Typical, deleted: vec![], lens_distortion: false }, None).unwrap();
    assert!((s.focal() - f).abs() < 1e-6);
    assert!(s.average_error < 0.5);
    let (pe, _) = path_error(&s, &t);
    assert!(pe < 0.01, "{pe}");
}

#[test]
fn variable_zoom_recovers_per_frame_focal() {
    let t = moving_camera(40, |k| 950.0 + 6.0 * k as f64);
    let pts = random_points(450, 7, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let tr = tracks_of(&pts, &t, 8, true);
    let s = solve(&tr, &settings(ShotType::VariableZoom, SolveMethod::Typical), None).expect("solve");
    eprintln!("variable: err {:.3}", s.average_error);
    assert!(s.average_error < 0.5);
    for (k, c) in s.frames.iter().enumerate() {
        let f = t.cams[k].2;
        assert!((c.focal - f).abs() / f < 0.02, "frame {k}: {} vs {f}", c.focal);
    }
}

#[test]
fn tripod_pan_solve() {
    let f = 1200.0;
    let cams = (0..40)
        .map(|k| {
            let a = -0.4 + 0.8 * k as f64 / 39.0;
            (mm(&rodrigues([0.0, 0.0, 0.02]), &mm(&rodrigues([0.05 * a, 0.0, 0.0]), &rodrigues([0.0, a, 0.0]))), [0.0; 3], f)
        })
        .collect();
    let t = Truth { cams };
    // Points all around at varying distances (rotation only: depth is unobservable).
    let pts = random_points(600, 9, [0.0, 0.0, 0.0], [60.0, 20.0, 60.0]).into_iter().filter(|p| p[2] > 3.0).collect::<Vec<_>>();
    let tr = tracks_of(&pts, &t, 10, true);
    let s = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Auto), None).expect("solve");
    eprintln!("tripod: {:?} err {:.3} focal {:.1}", s.method_used, s.average_error, s.focal());
    assert_eq!(s.method_used, SolveMethod::TripodPan);
    assert!(s.average_error < 0.5);
    assert!((s.focal() - f).abs() / f < 0.02, "{}", s.focal());
    for (k, c) in s.frames.iter().enumerate() {
        assert!(norm(c.center) < 1e-9);
        // Relative rotation to frame 0 matches.
        let rel_s = mm(&c.r(), &mt(&s.frames[0].r()));
        let rel_t = mm(&t.cams[k].0, &mt(&t.cams[0].0));
        assert!(norm(log_rot(&mm(&rel_s, &mt(&rel_t)))).to_degrees() < 0.2);
    }
}

#[test]
fn mostly_flat_scene_and_ground_plane() {
    let f = 1000.0;
    // A textured floor y = 2 (below the camera), seen by a camera moving forward and sideways.
    let cams = (0..40)
        .map(|k| {
            let t = k as f64 / 39.0;
            let c = [-2.0 + 4.0 * t, -0.3 * (std::f64::consts::PI * t).sin(), 2.0 * t];
            let fwd = normalize([0.1 - 0.2 * t, 0.45, 1.0]);
            let right = normalize(cross([0.0, 1.0, 0.0], fwd));
            let down = cross(fwd, right);
            ([right, down, fwd], c, f)
        })
        .collect();
    let t = Truth { cams };
    let mut rng = Rng(11);
    let pts: Vec<V3> = (0..500).map(|_| [((rng.next_u64() % 2000) as f64 / 100.0) - 10.0, 2.0, 2.0 + (rng.next_u64() % 2000) as f64 / 100.0]).collect();
    let tr = tracks_of(&pts, &t, 12, true);
    for method in [SolveMethod::MostlyFlat, SolveMethod::Auto] {
        let s = solve(&tr, &settings(ShotType::FixedAngle, method), None).expect("solve");
        eprintln!("flat {method:?}: {:?} err {:.3} focal {:.1}", s.method_used, s.average_error, s.focal());
        assert_eq!(s.method_used, SolveMethod::MostlyFlat);
        assert!(s.average_error < 0.5, "{}", s.average_error);
        assert!((s.focal() - f).abs() / f < 0.02, "{}", s.focal());
        let (pe, _) = path_error(&s, &t);
        assert!(pe < 0.01, "{pe}");
    }
    // Ground plane from the solved (coplanar) points: every point lies on it.
    let hfov = 2.0 * (W * 0.5 / f).atan().to_degrees();
    let mut s =
        solve(&tr, &SolveSettings { shot: ShotType::SpecifyAngle, hfov, method: SolveMethod::MostlyFlat, deleted: vec![], lens_distortion: false }, None)
            .expect("solve");
    let ids: Vec<u32> = s.visible(0).map(|p| p.id).take(12).collect();
    let g = s.ground_from(&ids, 0).expect("ground");
    let spread = s.points.iter().map(|p| dot(sub(p.pos, g.origin), g.normal).abs()).fold(0.0, f64::max);
    let depth = norm(sub(s.frames[0].center, g.origin));
    assert!(spread < 0.01 * depth, "{spread}");
    // Up points towards the camera.
    assert!(dot(sub(s.frames[0].center, g.origin), g.normal) > 0.0);
    s.ground = Some(g);
    let w = s.world([W, H], 1000.0);
    // In the world, ground points have y ≈ 0 and the cameras are above (y < 0).
    for p in s.points.iter().take(20) {
        assert!(w.apply(p.pos)[1].abs() < 0.01 * 1000.0 * depth, "{:?}", w.apply(p.pos));
    }
    assert!(w.apply(s.frames[0].center)[1] < 0.0);
    assert!(norm(w.apply(g.origin)) < 1e-6);
}

#[test]
fn deleted_tracks_are_ignored_and_cancel_stops() {
    let t = moving_camera(20, |_| 1000.0);
    let pts = random_points(250, 13, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    let tr = tracks_of(&pts, &t, 14, true);
    let deleted: Vec<u32> = (0..40).collect();
    let hfov = 2.0 * (W * 0.5 / 1000.0f64).atan().to_degrees();
    let s =
        solve(&tr, &SolveSettings { shot: ShotType::SpecifyAngle, hfov, method: SolveMethod::Typical, deleted: deleted.clone(), lens_distortion: false }, None)
            .unwrap();
    assert!(s.points.iter().all(|p| !deleted.contains(&p.id)));
    let stop = std::sync::atomic::AtomicBool::new(true);
    let r = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Typical), Some(&stop));
    assert!(r.is_err());
}

/// End to end: rendered frames of a textured scene through the KLT feature tracker.
#[test]
fn rendered_scene_end_to_end() {
    use effectcraft_raster::Image;
    let (w, h) = (480u32, 270u32);
    let f = 420.0;
    // Three textured planes at different depths (a floor, a back wall and a box face).
    fn tex(u: f64, v: f64) -> f32 {
        fn hsh(i: i64, j: i64) -> f64 {
            let mut x = (i.wrapping_mul(73856093) ^ j.wrapping_mul(19349663)) as u64;
            x ^= x >> 13;
            x = x.wrapping_mul(0x5bd1e995);
            x ^= x >> 15;
            (x % 1000) as f64 / 1000.0
        }
        fn noise(x: f64, y: f64) -> f64 {
            let (i, j) = (x.floor(), y.floor());
            let (tx, ty) = (x - i, y - j);
            let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
            let (i, j) = (i as i64, j as i64);
            let a = hsh(i, j) + (hsh(i + 1, j) - hsh(i, j)) * sx;
            let b = hsh(i, j + 1) + (hsh(i + 1, j + 1) - hsh(i, j + 1)) * sx;
            a + (b - a) * sy
        }
        (0.15 + 0.5 * noise(u * 4.0, v * 4.0) + 0.3 * noise(u * 11.0 + 7.0, v * 11.0 + 3.0)) as f32
    }
    let frames = 24;
    let t = Truth {
        cams: (0..frames)
            .map(|k| {
                let s = k as f64 / (frames - 1) as f64;
                let c = [-1.0 + 2.0 * s, -0.2, 0.5 * s];
                let fwd = normalize(sub([0.0, 0.3, 8.0], c));
                let right = normalize(cross([0.0, 1.0, 0.0], fwd));
                ([right, cross(fwd, right), fwd], c, f)
            })
            .collect(),
    };
    // Planes: (point, normal, u axis, v axis).
    let planes: [(V3, V3, V3, V3); 3] = [
        ([0.0, 1.5, 0.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 10.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 6.0], [0.3, 0.0, -1.0], [1.0, 0.0, 0.3], [0.0, 1.0, 0.0]),
    ];
    let render = |k: usize| {
        let (r, c, f) = t.cams[k];
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = mtv(&r, [(x as f64 + 0.5 - w as f64 / 2.0) / f, (y as f64 + 0.5 - h as f64 / 2.0) / f, 1.0]);
                let mut best = (f64::INFINITY, 0.0f32);
                for (i, (p0, n, ua, va)) in planes.iter().enumerate() {
                    let den = dot(*n, d);
                    if den.abs() < 1e-9 {
                        continue;
                    }
                    let tt = dot(*n, sub(*p0, c)) / den;
                    let hit = add(c, scale(d, tt));
                    // The box face is limited in extent.
                    if i == 2 && (dot(sub(hit, *p0), *ua).abs() > 1.2 || dot(sub(hit, *p0), *va).abs() > 1.0) {
                        continue;
                    }
                    if tt > 0.1 && tt < best.0 {
                        best = (tt, tex(dot(sub(hit, *p0), *ua) + 10.0 * i as f64, dot(sub(hit, *p0), *va)));
                    }
                }
                let v = best.1;
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        img
    };
    let mut an = TrackAnalyzer::new([w as f64, h as f64], AnalyzeOpts::default());
    for k in 0..frames {
        let img = render(k);
        an.push(&crate::Frame::new(&img));
    }
    let tr = an.finish(0.0, 1.0 / 24.0);
    assert!(tr.tracks.len() > 100, "{}", tr.tracks.len());
    let s = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Auto), None).expect("solve");
    eprintln!("rendered: {:?} err {:.3} focal {:.1} points {}", s.method_used, s.average_error, s.focal(), s.points.len());
    assert!(s.average_error < 0.5, "{}", s.average_error);
    assert!((s.focal() - f).abs() / f < 0.05, "{}", s.focal());
    let (pe, _) = path_error(&s, &t);
    assert!(pe < 0.03, "{pe}");
}

/// A long clip (300 frames, ~900 points): `cargo test -p effectcraft-track --release -- --ignored`.
#[test]
#[ignore]
fn perf_long_clip() {
    let f = 1100.0;
    let t = moving_camera(300, |_| f);
    let pts = random_points(900, 21, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let tr = tracks_of(&pts, &t, 22, true);
    let t0 = std::time::Instant::now();
    let s = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Auto), None).expect("solve");
    eprintln!("300 frames, {} tracks: {:?} err {:.3} focal {:.1} in {:?}", tr.tracks.len(), s.method_used, s.average_error, s.focal(), t0.elapsed());
    assert!(s.average_error < 0.5);
}

/// Solve tracks saved from the app (`EC_CAMTRACK_FILE=tracks.json cargo test -p effectcraft-track
/// --release solve_saved_tracks -- --ignored --nocapture`): a debugging aid.
#[test]
#[ignore]
fn solve_saved_tracks() {
    let Some(path) = std::env::var_os("EC_CAMTRACK_FILE") else { return };
    let mut tr = CameraTracks::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    if let Some(maxy) = std::env::var("EC_CAMTRACK_MAXY").ok().and_then(|v| v.parse::<f32>().ok()) {
        tr.tracks.retain(|t| t.pts.iter().all(|p| p[1] < maxy));
    }
    let shot = if std::env::var_os("EC_CAMTRACK_VARIABLE").is_some() { ShotType::VariableZoom } else { ShotType::FixedAngle };
    let t0 = std::time::Instant::now();
    // With the true cameras (`[{pos, poi, zoom}]` per frame, two-node), how consistent are the
    // tracks themselves?
    if let Some(tp) = std::env::var_os("EC_CAMTRACK_TRUTH") {
        let v: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(tp).unwrap()).unwrap();
        let g = |x: &serde_json::Value| -> V3 { [x[0].as_f64().unwrap(), x[1].as_f64().unwrap(), x[2].as_f64().unwrap()] };
        let cams: Vec<(super::geometry::Pose, f64)> = v
            .iter()
            .map(|c| {
                let (pos, poi) = (g(&c["pos"]), g(&c["poi"]));
                let fwd = normalize(sub(poi, pos));
                let d0 = [0.0, 1.0, 0.0];
                let down = normalize(sub(d0, scale(fwd, dot(d0, fwd))));
                let right = cross(down, fwd);
                (super::geometry::Pose { r: [right, down, fwd], c: pos }, c["zoom"].as_f64().unwrap())
            })
            .collect();
        let (w, h) = (tr.size[0], tr.size[1]);
        let mut means = vec![];
        let mut drop_ids = vec![];
        for t in &tr.tracks {
            let views: Vec<(super::geometry::Pose, [f64; 2])> = t
                .pts
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let (pose, z) = cams[t.start as usize + i];
                    (pose, [(p[0] as f64 - w / 2.0) / z, (p[1] as f64 - h / 2.0) / z])
                })
                .collect();
            let Some(x) = super::geometry::triangulate(&views) else { continue };
            let e: Vec<f64> = views.iter().filter_map(|(p, o)| super::geometry::reproj2(p, x, *o).map(|e| e.sqrt() * cams[0].1)).collect();
            means.push((e.iter().sum::<f64>() / e.len().max(1) as f64, t.pts[0]));
            if std::env::var_os("EC_CAMTRACK_TRUTH_FILTER").is_some() && means.last().unwrap().0 > 1.0 {
                drop_ids.push(t.id);
            }
        }
        tr.tracks.retain(|t| !drop_ids.contains(&t.id));
        means.sort_by(|a, b| a.0.total_cmp(&b.0));
        eprintln!(
            "true-camera track errors (px): median {:.2}; quartiles {:?}",
            means[means.len() / 2].0,
            [means[means.len() / 4].0, means[means.len() * 3 / 4].0]
        );
        for m in means.iter().step_by(6) {
            eprintln!("  {:.2} at {:?}", m.0, m.1);
        }
    }
    let mut st = settings(shot, SolveMethod::Auto);
    if let Some(a) = std::env::var("EC_CAMTRACK_AOV").ok().and_then(|v| v.parse::<f64>().ok()) {
        st.shot = ShotType::SpecifyAngle;
        st.hfov = a;
    }
    let s = solve(&tr, &st, None).expect("solve");
    eprintln!(
        "{} frames, {} tracks: {:?} err {:.3} focal {:.1} (hfov {:.1}) points {} in {:?}",
        tr.frames,
        tr.tracks.len(),
        s.method_used,
        s.average_error,
        s.focal(),
        s.hfov(s.focal()),
        s.points.len(),
        t0.elapsed()
    );
}

/// Pass every track position through a lens distortion (around the frame centre).
fn distort_tracks(t: &CameraTracks, d: &Distortion) -> CameraTracks {
    let mut t = t.clone();
    for tr in t.tracks.iter_mut() {
        for p in tr.pts.iter_mut() {
            let q = d.distort([p[0] as f64 - W / 2.0, p[1] as f64 - H / 2.0]);
            *p = [(q[0] + W / 2.0) as f32, (q[1] + H / 2.0) as f32];
        }
    }
    t
}

#[test]
fn lens_distortion_is_recovered_in_the_bundle_adjustment() {
    let f = 1000.0;
    let t = moving_camera(36, |_| f);
    let pts = random_points(450, 7, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let truth = Distortion { k1: -0.12, k2: 0.03, radius: 0.5 * W.hypot(H) };
    let tr = distort_tracks(&tracks_of(&pts, &t, 8, true), &truth);
    let plain = solve(&tr, &settings(ShotType::FixedAngle, SolveMethod::Typical), None).expect("pinhole solve");
    let lens = solve(&tr, &SolveSettings { lens_distortion: true, ..settings(ShotType::FixedAngle, SolveMethod::Typical) }, None).expect("lens solve");
    let d = lens.distortion.expect("distortion solved");
    eprintln!(
        "pinhole error {:.3} px (focal {:.1}); with distortion {:.3} px (focal {:.1}), k1 {:.4} k2 {:.4}",
        plain.average_error,
        plain.focal(),
        lens.average_error,
        lens.focal(),
        d.k1,
        d.k2
    );
    assert!(plain.distortion.is_none());
    assert!((d.k1 - truth.k1).abs() < 0.005, "k1 {}", d.k1);
    assert!((d.k2 - truth.k2).abs() < 0.01, "k2 {}", d.k2);
    // The distortion model explains the tracks: about the noise level, far below the pinhole fit.
    assert!(lens.average_error < 0.3, "{}", lens.average_error);
    assert!(lens.average_error < 0.5 * plain.average_error, "{} vs {}", lens.average_error, plain.average_error);
    assert!((lens.focal() - f).abs() < 0.02 * f, "focal {}", lens.focal());
    let (path, rot) = path_error(&lens, &t);
    assert!(path < 0.01 && rot < 0.3, "path {path} rot {rot}");
    // Projection through the lens lands on the (distorted) tracks; undistorting inverts it.
    let p = [100.0, 80.0];
    let q = lens.to_image(p);
    let back = lens.to_ideal(q);
    assert!((back[0] - p[0]).abs() < 1e-6 && (back[1] - p[1]).abs() < 1e-6);
    let sp = &lens.points[0];
    let k = sp.first as usize;
    let obs = tr.track(sp.id).and_then(|t| t.at(k as u32)).expect("observed");
    let proj = lens.project(k, sp.pos, false).expect("in front");
    assert!((proj[0] - obs[0]).hypot(proj[1] - obs[1]) < 2.0, "{proj:?} vs {obs:?}");
}

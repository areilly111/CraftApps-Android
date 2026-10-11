//! End-to-end tests: synthetic logos made in code, with known geometry, traced and measured.

use crate::{Method, Mode, Raster, TraceError, TraceParams, TraceResult, trace_within};
use kurbo::{ParamCurve, Shape};
use vectorcraft_geom::Point;

/// A soft, anti-aliased render of `shade` (`Some(colour)` where the shape is), supersampled `ss × ss`
/// and blurred by `blur` pixels, on a transparent background.
fn render(w: u32, h: u32, blur: f64, shade: impl Fn(f64, f64) -> Option<[u8; 3]>) -> Raster {
    let ss = 8usize;
    let (wu, hu) = (w as usize, h as usize);
    let mut px = vec![[0.0f64; 4]; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            let mut acc = [0.0f64; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let (fx, fy) = (x as f64 + (sx as f64 + 0.5) / ss as f64, y as f64 + (sy as f64 + 0.5) / ss as f64);
                    if let Some(c) = shade(fx, fy) {
                        acc[0] += f64::from(c[0]) / 255.0;
                        acc[1] += f64::from(c[1]) / 255.0;
                        acc[2] += f64::from(c[2]) / 255.0;
                        acc[3] += 1.0;
                    }
                }
            }
            let n = (ss * ss) as f64;
            px[y * wu + x] = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
        }
    }
    if blur > 0.0 {
        let r = (3.0 * blur).ceil() as i64;
        let k: Vec<f64> = (-r..=r).map(|i| (-0.5 * (i as f64 / blur).powi(2)).exp()).collect();
        let sum: f64 = k.iter().sum();
        let k: Vec<f64> = k.iter().map(|v| v / sum).collect();
        for horizontal in [true, false] {
            let src = px.clone();
            for y in 0..hu as i64 {
                for x in 0..wu as i64 {
                    let mut acc = [0.0f64; 4];
                    for (j, kv) in k.iter().enumerate() {
                        let d = j as i64 - r;
                        let (xx, yy) = if horizontal { ((x + d).clamp(0, wu as i64 - 1), y) } else { (x, (y + d).clamp(0, hu as i64 - 1)) };
                        let p = src[(yy as usize) * wu + xx as usize];
                        for c in 0..4 {
                            acc[c] += kv * p[c];
                        }
                    }
                    px[(y as usize) * wu + x as usize] = acc;
                }
            }
        }
    }
    let mut rgba = Vec::with_capacity(wu * hu * 4);
    for p in px {
        let a = p[3];
        let un = |v: f64| if a > 1e-9 { ((v / a) * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    Raster::new(w, h, rgba)
}

const RED: [u8; 3] = [210, 40, 50];
const BLUE: [u8; 3] = [40, 80, 200];

fn logo_params() -> TraceParams {
    TraceParams {
        mode: Mode::Logo,
        colors: 6,
        paths: 60.0,
        corners: 50.0,
        noise: 6,
        snap_curves_to_lines: true,
        method: Method::Abutting,
        ..TraceParams::default()
    }
}

fn trace(img: &Raster, p: &TraceParams) -> TraceResult {
    trace_within(img, p, 100_000).expect("trace")
}

fn samples(res: &TraceResult) -> Vec<Point> {
    let mut out = Vec::new();
    for p in &res.paths {
        for seg in p.path.to_bezpath().segments() {
            out.extend((0..=40).map(|i| seg.eval(f64::from(i) / 40.0)));
        }
    }
    out
}

fn covered(res: &TraceResult, p: Point) -> bool {
    res.paths.iter().any(|t| t.path.to_bezpath().winding(p) != 0)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= tol)
}

#[test]
fn a_blurred_disc_becomes_a_true_circle_within_a_tenth_of_a_pixel() {
    let (cx, cy, r) = (31.7, 32.4, 19.3);
    let img = render(64, 64, 0.7, |x, y| ((x - cx).hypot(y - cy) <= r).then_some(RED));
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1, "{} paths", res.paths.len());
    assert!(near(res.paths[0].color, RED, 3), "colour {:?}", res.paths[0].color);
    let errs: Vec<f64> = samples(&res).iter().map(|p| (p.distance(Point::new(cx, cy)) - r).abs()).collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    let max = errs.iter().copied().fold(0.0, f64::max);
    assert!(mean < 0.06 && max < 0.15, "mean {mean:.3} max {max:.3}");
    assert!(res.anchor_count() <= 6, "{} anchors for a circle", res.anchor_count());
}

#[test]
fn a_blurred_rectangle_keeps_square_corners_and_four_anchors() {
    let (x0, x1, y0, y1) = (10.4, 77.9, 8.7, 38.2);
    let img = render(90, 48, 0.8, |x, y| ((x0..=x1).contains(&x) && (y0..=y1).contains(&y)).then_some(BLUE));
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.anchor_count(), 4, "a rectangle is four corners");
    let b = res.paths[0].path.bounds().expect("bounds");
    for (got, want, what) in [(b.x0, x0, "left"), (b.x1, x1, "right"), (b.y0, y0, "top"), (b.y1, y1, "bottom")] {
        // Sub-pixel: within 0.15 px of the true edge, from a blurred 90 px image.
        assert!((got - want).abs() < 0.15, "{what} edge {got:.3} vs {want}");
    }
}

#[test]
fn touching_colours_leave_no_gap_between_them() {
    let edge = 40.35;
    let img = render(80, 50, 0.7, |x, y| {
        if !(10.2..=40.6).contains(&y) {
            None
        } else if (10.3..=edge).contains(&x) {
            Some(RED)
        } else if (edge..=70.6).contains(&x) {
            Some(BLUE)
        } else {
            None
        }
    });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 2);
    assert!(!near(res.paths[0].color, res.paths[1].color, 20));
    for i in 0..200 {
        let y = 12.0 + 27.0 * f64::from(i) / 199.0;
        for dx in [-0.03, 0.0, 0.03] {
            assert!(covered(&res, Point::new(edge + dx, y)), "gap at ({}, {y})", edge + dx);
        }
    }
}

#[test]
fn letters_knocked_out_of_a_badge_become_white_or_holes() {
    let ring = |x: f64, y: f64| {
        let d = (x - 32.0).hypot(y - 32.0);
        (12.0..=25.0).contains(&d).then_some(BLUE)
    };
    let img = render(64, 64, 0.7, ring);
    // Default: the knockout is painted white, on top of the badge.
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 2, "badge + white knockout");
    assert!(res.paths.iter().any(|p| p.color == [255, 255, 255]));
    assert!(covered(&res, Point::new(32.0, 32.0)));
    // Ignore white: the knockout is a real hole in the badge.
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].path.subpaths.len(), 2, "outer outline and a hole");
    assert!(!covered(&res, Point::new(32.0, 32.0)), "the hole must be empty");
    assert!(covered(&res, Point::new(32.0, 32.0 + 18.0)), "the ring itself is filled");
}

#[test]
fn a_gradient_is_refused_not_posterised() {
    let img = Raster::from_fn(64, 32, |x, y| [(x * 4) as u8, 255 - (x * 4) as u8, (y * 6) as u8, 255]);
    match trace_within(&img, &logo_params(), 100_000) {
        Err(TraceError::NotFlat { fraction }) => assert!(fraction < 0.93),
        other => panic!("expected NotFlat, got {other:?}"),
    }
    let msg = TraceError::NotFlat { fraction: 0.69 }.to_string();
    assert!(msg.contains("69%") && msg.contains("Color mode"), "{msg}");
}

#[test]
fn a_white_background_logo_drops_the_white_when_asked() {
    let img = Raster::from_fn(80, 60, |x, y| {
        let d = (x as f64 + 0.5 - 40.0).hypot(y as f64 + 0.5 - 30.0);
        if (10.0..=24.0).contains(&d) { [20, 20, 20, 255] } else { [255, 255, 255, 255] }
    });
    let kept = trace(&img, &logo_params());
    assert!(kept.paths.iter().any(|p| p.color == [255, 255, 255]), "white background kept by default");
    let dropped = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert!(dropped.paths.iter().all(|p| p.color != [255, 255, 255]));
    assert!(!dropped.paths.is_empty());
}

#[test]
fn the_named_palette_is_used_exactly_and_a_bad_one_is_an_error() {
    let img = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(RED));
    let named = TraceParams { logo_colors: vec!["#d22832".into()], ..logo_params() };
    let res = trace(&img, &named);
    assert_eq!(res.paths[0].color, [0xd2, 0x28, 0x32]);
    let bad = TraceParams { logo_colors: vec!["red".into()], ..logo_params() };
    assert!(matches!(trace_within(&img, &bad, 1000), Err(TraceError::BadPalette(s)) if s == "red"));
}

#[test]
fn a_large_image_is_traced_at_reduced_size_and_scaled_back() {
    let (cx, cy, r) = (1200.0, 600.0, 400.0);
    let img = Raster::from_fn(2400, 1200, |x, y| {
        if (f64::from(x) + 0.5 - cx).hypot(f64::from(y) + 0.5 - cy) <= r { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] }
    });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    let b = res.paths[0].path.bounds().expect("bounds");
    for (got, want) in [(b.x0, cx - r), (b.x1, cx + r), (b.y0, cy - r), (b.y1, cy + r)] {
        assert!((got - want).abs() < 2.0, "{got} vs {want}");
    }
}

/// Every anchor of every path is finite and inside (a small margin around) the image.
fn assert_sane(res: &TraceResult, w: u32, h: u32) {
    for t in &res.paths {
        for sp in &t.path.subpaths {
            for a in &sp.anchors {
                assert!(a.p.x.is_finite() && a.p.y.is_finite() && a.h_in.x.is_finite() && a.h_out.y.is_finite(), "non-finite anchor");
                assert!(a.p.x > -2.0 && a.p.y > -2.0 && a.p.x < f64::from(w) + 2.0 && a.p.y < f64::from(h) + 2.0, "anchor {:?} outside {w}x{h}", a.p);
            }
        }
    }
}

#[test]
fn degenerate_images_are_fine() {
    let p = logo_params();
    for (w, h, img) in [
        (0, 0, Raster::new(0, 0, vec![])),
        (1, 1, Raster::new(1, 1, vec![200, 30, 30, 255])),
        (3, 3, Raster::new(3, 3, vec![0; 36])),
        (2, 2, Raster::new(2, 2, vec![255; 16])),
        (5, 1, Raster::new(5, 1, vec![9; 20])),
        (1, 7, Raster::new(1, 7, vec![130; 28])),
    ] {
        match trace_within(&img, &p, 1000) {
            Ok(res) => assert_sane(&res, w, h),
            Err(TraceError::NotFlat { .. }) => {}
            Err(e) => panic!("unexpected error for {w}x{h}: {e}"),
        }
    }
}

#[test]
fn extreme_parameters_never_panic() {
    let img = render(48, 48, 0.7, |x, y| ((x - 24.0).hypot(y - 24.0) <= 15.0).then_some(RED));
    let weird = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -50.0, 0.0, 100.0, 1e9];
    for &paths in &weird {
        for &corners in &weird {
            let p = TraceParams { paths, corners, ..logo_params() };
            let _ = trace_within(&img, &p, 10_000);
        }
    }
    for colors in [0, 1, 2, 300, u32::MAX] {
        for noise in [0, 1, 1_000_000, u32::MAX] {
            let p = TraceParams { colors, noise, ..logo_params() };
            let _ = trace_within(&img, &p, 10_000);
        }
    }
    // A tiny anchor budget is reported as too complex, not exceeded.
    let r = trace_within(&img, &logo_params(), 0);
    assert!(r.is_err() || r.is_ok_and(|t| t.anchor_count() == 0));
}

#[test]
fn noise_specks_are_dropped() {
    let img = render(64, 64, 0.5, |x, y| if (x - 32.0).hypot(y - 32.0) <= 18.0 || (x - 6.0).hypot(y - 6.0) <= 0.6 { Some(RED) } else { None });
    let res = trace(&img, &TraceParams { noise: 20, ..logo_params() });
    assert_eq!(res.paths.len(), 1, "the speck should be gone");
}

#[test]
fn nested_rings_keep_their_own_holes_under_ignore_white() {
    // Red ring, white band, red ring, white centre.
    let img = render(80, 80, 0.7, |x, y| {
        let d = (x - 40.0).hypot(y - 40.0);
        ((22.0..=28.0).contains(&d) || (8.0..=16.0).contains(&d)).then_some(RED)
    });
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert_eq!(res.paths.len(), 2, "two red rings");
    for p in &res.paths {
        assert_eq!(p.path.subpaths.len(), 2, "each ring is an outline and its own hole, no more");
    }
    assert!(!covered(&res, Point::new(40.0, 40.0)), "centre is empty");
    assert!(covered(&res, Point::new(52.0, 40.0)), "inner ring is filled");
    assert!(!covered(&res, Point::new(59.0, 40.0)), "the band between the rings is empty");
    assert!(covered(&res, Point::new(65.0, 40.0)), "outer ring is filled");
}

#[test]
fn too_many_outlines_are_refused_not_silently_truncated() {
    let specks =
        |x: f64, y: f64| (x.rem_euclid(5.0) >= 1.0 && x.rem_euclid(5.0) < 4.0 && y.rem_euclid(5.0) >= 1.0 && y.rem_euclid(5.0) < 4.0).then_some(RED);
    let img = render(200, 200, 0.0, specks);
    assert!(matches!(trace_within(&img, &logo_params(), 1_000_000), Err(TraceError::TooComplex { .. })));
    // Specks under the noise threshold do not count towards the limit.
    let res = trace_within(&img, &TraceParams { noise: 200, ..logo_params() }, 1_000_000).expect("trace");
    assert!(res.paths.is_empty());
}

#[test]
fn a_large_odd_sized_image_never_overshoots_its_edges() {
    // 1025 wide: the last working pixel is narrower than the others.
    let img = Raster::from_fn(1025, 20, |x, y| if x >= 600 && (2..18).contains(&y) { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] });
    let res = trace(&img, &logo_params());
    assert_eq!(res.paths.len(), 1);
    let b = res.paths[0].path.bounds().expect("bounds");
    assert!(b.x1 <= 1025.15 && b.x1 >= 1024.7, "right edge {}", b.x1);
    assert!((b.x0 - 600.0).abs() < 0.6 && (b.y0 - 2.0).abs() < 0.6 && (b.y1 - 18.0).abs() < 0.6, "{b:?}");
    assert_sane(&res, 1025, 20);
}

#[test]
fn a_hostile_palette_is_bounded_and_near_duplicates_merge() {
    let img = render(64, 64, 0.7, |x, y| ((x - 32.0).hypot(y - 32.0) <= 20.0).then_some(RED));
    let many: Vec<String> = std::iter::once("#d22832".to_string()).chain((1..200_000u32).map(|i| format!("#{:06x}", i * 37 % 0xfffff0))).collect();
    let started = std::time::Instant::now();
    let res = trace_within(&img, &TraceParams { logo_colors: many, ..logo_params() }, 100_000);
    assert!(started.elapsed().as_secs() < 20, "palette handling took {:?}", started.elapsed());
    if let Ok(r) = res {
        assert!(r.palette.len() <= 11);
    }
    // Ten shades within a couple of units of one red are one colour, so one path, not ten layers.
    let shades: Vec<String> = (0..10).map(|i| format!("#{:02x}{:02x}{:02x}", 210 + i % 3, 40 + i % 2, 50 + i % 3)).collect();
    let res = trace(&img, &TraceParams { logo_colors: shades, ..logo_params() });
    assert_eq!(res.paths.len(), 1, "{} paths", res.paths.len());
    // White cannot be named: naming only white leaves nothing to trace.
    assert!(trace(&img, &TraceParams { logo_colors: vec!["#ffffff".into()], ..logo_params() }).paths.is_empty());
}

#[test]
fn a_small_accent_colour_is_not_lost() {
    let img = render(100, 100, 0.6, |x, y| {
        if (x - 50.0).hypot(y - 50.0) <= 40.0 {
            Some(BLUE)
        } else if (x - 90.0).hypot(y - 90.0) <= 3.5 {
            Some(RED)
        } else {
            None
        }
    });
    let res = trace(&img, &logo_params());
    assert!(res.paths.iter().any(|p| near(p.color, RED, 12)), "palette {:?}", res.palette);
    assert!(res.paths.iter().any(|p| near(p.color, BLUE, 12)));
}

#[test]
fn a_hard_edged_source_still_gets_smooth_circles() {
    // No anti-aliasing at all: every pixel is exactly one colour.
    let img = Raster::from_fn(80, 80, |x, y| {
        let d = (f64::from(x) + 0.5 - 40.0).hypot(f64::from(y) + 0.5 - 40.0);
        if (10.0..=28.0).contains(&d) { [RED[0], RED[1], RED[2], 255] } else { [0, 0, 0, 0] }
    });
    let res = trace(&img, &TraceParams { ignore_white: true, ..logo_params() });
    assert!(res.anchor_count() <= 40, "{} anchors for two circles from a hard-edged source", res.anchor_count());
    let errs: Vec<f64> = samples(&res)
        .iter()
        .map(|p| {
            let d = p.distance(Point::new(40.0, 40.0));
            (d - 28.0).abs().min((d - 10.0).abs())
        })
        .collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    assert!(mean < 0.3 && errs.iter().copied().fold(0.0, f64::max) < 0.8, "mean {mean:.3}");
}

#[test]
fn random_small_images_and_settings_never_panic() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let weird = [f64::NAN, f64::INFINITY, -3.0, 0.0, 35.0, 60.0, 100.0, 1e12];
    let colours = [[210u8, 40, 50], [40, 80, 200], [255, 255, 255], [20, 160, 70], [0, 0, 0]];
    for _ in 0..250 {
        let (w, h) = ((next() % 24 + 1) as u32, (next() % 24 + 1) as u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let r = next();
                let c = colours[((u64::from(x) / 3 + u64::from(y) / 4 + r % 2) % 5) as usize];
                rgba.extend_from_slice(&match r % 11 {
                    0 => [0, 0, 0, 0],
                    1 => [(r >> 8) as u8, (r >> 16) as u8, (r >> 24) as u8, (r >> 32) as u8],
                    _ => [c[0], c[1], c[2], 255],
                });
            }
        }
        let img = Raster::new(w, h, rgba);
        let p = TraceParams {
            paths: weird[(next() % 8) as usize],
            corners: weird[(next() % 8) as usize],
            noise: [0, 1, 6, 1000, u32::MAX][(next() % 5) as usize],
            colors: [0, 2, 6, 40][(next() % 4) as usize],
            ignore_white: next() % 2 == 0,
            snap_curves_to_lines: next() % 2 == 0,
            ..logo_params()
        };
        if let Ok(res) = trace_within(&img, &p, 20_000) {
            assert_sane(&res, w, h);
        }
    }
}

//! Shape wipes driven by a scalar field: the incoming clip appears where `field(x, y) < T(p)`,
//! with `T` sweeping from below the field's minimum to above its maximum. Feather, border width,
//! border colour and anti-aliasing quality are shared by every wipe and iris.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use super::{Tx, mix, paint, smoothstep};
use crate::image::Image;

/// Edge styling read from the common wipe params (missing params = 0 / off).
#[derive(Clone, Copy, Debug)]
pub struct WipeStyle {
    pub feather: f32,
    pub border: f32,
    pub border_color: [f32; 4],
    pub aa: f32,
}

impl WipeStyle {
    pub(crate) fn of(t: &Tx) -> Self {
        let aa = [0.5, 1.0, 1.75, 2.5][t.choice("antialias").min(3) as usize] * t.scale.clamp(0.25, 1.0);
        WipeStyle { feather: t.px("feather"), border: t.px("border_width"), border_color: t.color("border_color"), aa }
    }
    fn margin(&self) -> f32 {
        self.border + self.feather + 2.0 * self.aa + 1.0
    }
}

/// Coverage of (B, B+border) at signed distance `d` (negative = inside the B region).
#[inline]
pub(crate) fn coverage(st: &WipeStyle, d: f32) -> (f32, f32) {
    let s = (st.feather + st.aa) * 0.5;
    let kb = 1.0 - smoothstep(-s, s, d);
    let ko = if st.border > 0.0 { 1.0 - smoothstep(st.border - s, st.border + s, d) } else { kb };
    (kb, ko.max(kb))
}

/// Composite one pixel of A/B/border for signed distance `d`.
#[inline]
pub(crate) fn wipe_px(st: &WipeStyle, a: [f32; 4], b: [f32; 4], d: f32) -> [f32; 4] {
    let (kb, ko) = coverage(st, d);
    let mut o = [0.0; 4];
    for k in 0..4 {
        o[k] = a[k] * (1.0 - ko) + st.border_color[k] * (ko - kb) + b[k] * kb;
    }
    o
}

/// Min/max of `field` over a coarse grid covering the frame (edges included).
fn field_range(w: f32, h: f32, field: &(impl Fn(f32, f32) -> f32 + Sync)) -> (f32, f32) {
    let n = 48;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for j in 0..=n {
        for i in 0..=n {
            let v = field(w * i as f32 / n as f32, h * j as f32 / n as f32);
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    if lo.is_finite() { (lo, hi) } else { (0.0, 1.0) }
}

/// The sweeping wipe: B where `field < T(p)` (field in pixels).
pub(crate) fn field_wipe(t: &Tx, st: WipeStyle, field: impl Fn(f32, f32) -> f32 + Sync) -> Image {
    let (lo, hi) = field_range(t.wf, t.hf, &field);
    let m = st.margin() + (hi - lo) * 0.01;
    let thr = lo - m + t.p * (hi - lo + 2.0 * m);
    paint(t.w, t.h, |x, y| wipe_px(&st, t.pa(x, y), t.pb(x, y), field(x, y) - thr))
}

/// Unit vector of travel for an angle param (0° = up, 90° = right).
pub(crate) fn angle_dir(deg: f32) -> (f32, f32) {
    let r = deg.to_radians();
    (r.sin(), -r.cos())
}

/// Distance metric of a shape of unit "radius" (value < R ⇔ inside a shape of size R).
pub(crate) fn shape_dist(shape: u32, x: f32, y: f32) -> f32 {
    let r = (x * x + y * y).sqrt();
    let th = (-y).atan2(x); // y up
    let ngon = |n: f32, rot: f32| {
        let seg = TAU / n;
        let a = (th - rot).rem_euclid(seg) - seg / 2.0;
        r * a.cos() / (PI / n).cos()
    };
    match shape {
        1 => x.abs().max(y.abs()),
        2 => (x.abs() + y.abs()) * std::f32::consts::FRAC_1_SQRT_2,
        3 => star_dist(x, y, 5, 0.45),
        4 => {
            // cardioid "heart": cusp at the top, point at the bottom
            let s = 0.5 * (1.0 - th.sin()) + 0.08;
            r / s * 0.5
        }
        5 => ngon(3.0, FRAC_PI_2 - PI / 3.0),
        6 => ngon(6.0, 0.0),
        _ => r,
    }
}

/// Star metric with `n` points and inner radius `inner` (fraction of outer): straight edges
/// between each point and the neighbouring inner vertex.
pub(crate) fn star_dist(x: f32, y: f32, n: u32, inner: f32) -> f32 {
    let r = (x * x + y * y).sqrt();
    if r < 1e-6 {
        return 0.0;
    }
    let th = (-y).atan2(x) - FRAC_PI_2; // first point up
    let seg = TAU / n.max(2) as f32;
    let a = (th.rem_euclid(seg) - seg / 2.0).abs(); // 0 at an inner vertex, seg/2 at a point
    let a = seg / 2.0 - a; // angle from the point
    let ri = inner.clamp(0.05, 1.0);
    // edge from P0 = (1, 0) to P1 = ri·(cos(seg/2), sin(seg/2)); ray at angle a
    let (p1x, p1y) = (ri * (seg / 2.0).cos(), ri * (seg / 2.0).sin());
    let (ex, ey) = (p1x - 1.0, p1y);
    let (dx, dy) = (a.cos(), a.sin());
    let denom = dx * ey - dy * ex;
    let tr = if denom.abs() < 1e-9 { 1.0 } else { (ey) / denom }; // cross(P0, E) / cross(D, E) with P0 = (1, 0)
    r / tr.max(0.05)
}

fn rot(x: f32, y: f32, deg: f32) -> (f32, f32) {
    let (s, c) = deg.to_radians().sin_cos();
    (x * c + y * s, -x * s + y * c)
}

pub(super) fn apply(id: &str, t: &Tx) -> Option<Image> {
    let st = WipeStyle::of(t);
    let (cx, cy) = t.center();
    let (w, h) = (t.wf, t.hf);
    let asp = w / h.max(1.0);
    Some(match id {
        "linear_wipe" | "soft_wipe" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            field_wipe(t, st, move |x, y| x * dx + y * dy)
        }
        "clock_wipe" => {
            let start = t.num("start").to_radians();
            let scale = (w + h) * 0.5;
            field_wipe(t, st, move |x, y| {
                let a = (x - cx).atan2(-(y - cy)) - start;
                a.rem_euclid(TAU) / TAU * scale
            })
        }
        "radial_wipe" => {
            let corner = t.choice("corner");
            let (ox, oy, a0) = match corner {
                1 => (w, 0.0, FRAC_PI_2),
                2 => (w, h, PI),
                3 => (0.0, h, -FRAC_PI_2),
                _ => (0.0, 0.0, 0.0),
            };
            let scale = (w + h) * 0.5;
            field_wipe(t, st, move |x, y| {
                // angle measured clockwise (screen) from the corner's first edge
                let a = (y - oy).atan2(x - ox) - a0;
                let a = a.rem_euclid(TAU);
                let a = if a > PI * 1.5 { 0.0 } else { a.min(FRAC_PI_2) };
                a / FRAC_PI_2 * scale
            })
        }
        "star_wipe" => {
            let n = t.num("points").round().clamp(3.0, 24.0) as u32;
            let inner = t.frac("inner");
            let r0 = t.num("rotation");
            field_wipe(t, st, move |x, y| {
                let (u, v) = rot(x - cx, y - cy, r0);
                star_dist(u, v, n, inner)
            })
        }
        "panel_wipe" => {
            let (dx, dy) = t.dir();
            let n = t.num("panels").round().clamp(1.0, 64.0);
            let along_len = t.travel();
            let across_len = if dx != 0.0 { h } else { w };
            field_wipe(t, st, move |x, y| {
                let along = x * dx + y * dy;
                let across = if dx != 0.0 { y } else { x };
                let idx = (across / across_len * n).floor().clamp(0.0, n - 1.0);
                along + idx * along_len * 0.6 / n
            })
        }
        "plateau_wipe" => {
            let (dx, dy) = super::wipe::angle_dir(t.num("angle"));
            let n = t.num("plateaus").round().clamp(1.0, 64.0);
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| -x * dy + y * dx);
            let span = (hi - lo).max(1.0);
            let step = (w.max(h)) * 0.35 / n.max(1.0);
            field_wipe(t, st, move |x, y| {
                let along = x * dx + y * dy;
                let across = ((-x * dy + y * dx) - lo) / span;
                let idx = (across * n).floor().clamp(0.0, n - 1.0);
                along + (idx - (n - 1.0) / 2.0).abs() * step
            })
        }
        "stretch_wipe" => {
            let (dx, dy) = t.dir();
            let s_len = t.px("stretch").max(1.0);
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy);
            let e = lo - s_len + t.p * (hi - lo + 2.0 * s_len);
            paint(t.w, t.h, |x, y| {
                let s = x * dx + y * dy;
                if s < e {
                    t.pb(x, y)
                } else if s < e + s_len {
                    let back = s - e;
                    let smear = t.sbc(x - dx * back, y - dy * back);
                    mix(smear, t.pa(x, y), smoothstep(0.0, 1.0, back / s_len))
                } else {
                    t.pa(x, y)
                }
            })
        }
        "neon_wipe" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            let glow = t.px("glow").max(1.0);
            let col = t.color("color");
            let amt = t.frac("amount");
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy);
            let m = glow * 0.5 + 2.0;
            let thr = lo - m + t.p * (hi - lo + 2.0 * m);
            let fade = (t.p * 10.0).min((1.0 - t.p) * 10.0).min(1.0);
            let st = WipeStyle { feather: 0.0, border: 0.0, border_color: [0.0; 4], aa: st.aa };
            paint(t.w, t.h, |x, y| {
                let d = x * dx + y * dy - thr;
                let base = wipe_px(&st, t.pa(x, y), t.pb(x, y), d);
                let core = (-(d / (glow * 0.08)).powi(2)).exp();
                let halo = (-d.abs() / (glow * 0.5)).exp();
                let k = (core * 1.5 + halo * 0.8) * amt * fade;
                super::add_light(base, col, k)
            })
        }
        // Legacy wipes rebuilt on the field engine (border / anti-aliasing / centre).
        "iris_round" => field_wipe(t, st, move |x, y| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()),
        "iris_box" => field_wipe(t, st, move |x, y| (x - cx).abs().max((y - cy).abs() * asp)),
        "iris_diamond" => field_wipe(t, st, move |x, y| (x - cx).abs() + (y - cy).abs() * asp),
        "iris_cross" => field_wipe(t, st, move |x, y| {
            let (u, v) = ((x - cx).abs(), (y - cy).abs() * asp);
            let k = 0.3;
            (u / k).max(v).min(u.max(v / k))
        }),
        "barn_doors" => {
            let vertical = t.choice("orientation") == 1;
            field_wipe(t, st, move |x, y| if vertical { (x - cx).abs() } else { (y - cy).abs() * asp })
        }
        "inset" => {
            let (ox, oy) = match t.choice("corner") {
                1 => (w, 0.0),
                2 => (w, h),
                3 => (0.0, h),
                _ => (0.0, 0.0),
            };
            field_wipe(t, st, move |x, y| (x - ox).abs().max((y - oy).abs() * asp))
        }
        "vr_iris_wipe" => {
            // great-circle distance on the equirectangular sphere
            let lon = |x: f32| x / w * TAU - PI;
            let lat = |y: f32| FRAC_PI_2 - y / h * PI;
            let (l0, p0) = (lon(cx), lat(cy));
            let (sp0, cp0) = p0.sin_cos();
            let scale = w / TAU;
            field_wipe(t, st, move |x, y| {
                let (l1, p1) = (lon(x), lat(y));
                let c = sp0 * p1.sin() + cp0 * p1.cos() * (l1 - l0).cos();
                c.clamp(-1.0, 1.0).acos() * scale
            })
        }
        _ => return None,
    })
}

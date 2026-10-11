//! Grunge & Distort: procedural damage (shake, flicker, glitches, glass, liquid) that peaks at the
//! midpoint while the clips swap.

use std::f32::consts::{PI, TAU};

use super::wipe::{WipeStyle, field_wipe};
use super::{Tx, add_light, bell, fbm, hash, mid_mix, mix, paint, shutter, smoothstep, temporal, vnoise};
use crate::image::Image;

/// Per-pixel A/B mix at weight `k`, both sampled (clamped) at (sx, sy).
#[inline]
fn ab(t: &Tx, sx: f32, sy: f32, k: f32) -> [f32; 4] {
    if k <= 0.0 {
        t.sac(sx, sy)
    } else if k >= 1.0 {
        t.sbc(sx, sy)
    } else {
        mix(t.sac(sx, sy), t.sbc(sx, sy), k)
    }
}

/// RGB-split sample: red from (+d), blue from (−d).
#[inline]
fn split(t: &Tx, x: f32, y: f32, d: f32, k: f32) -> [f32; 4] {
    let c = ab(t, x, y, k);
    if d.abs() < 0.25 {
        return c;
    }
    let r = ab(t, x + d, y, k);
    let b = ab(t, x - d, y, k);
    [r[0], c[1], b[2], c[3].max(r[3]).max(b[3])]
}

pub(super) fn apply(id: &str, t: &Tx) -> Option<Image> {
    let p = t.p;
    let (w, h) = (t.wf, t.hf);
    let env = bell(p);
    let k = mid_mix(p);
    let s = t.seed();
    let u = t.unit().max(0.01);
    Some(match id {
        "chaos" => {
            let amt = t.frac("amount");
            let frame = (p * 30.0).floor() as i32;
            let amp = amt * env * w * 0.15;
            paint(t.w, t.h, |x, y| {
                // two block grids of different sizes, re-randomised a few times per second
                let (bx, by) = ((x / (w / 8.0)).floor() as i32, (y / (h / 6.0)).floor() as i32);
                let (cx, cy) = ((x / (w / 23.0)).floor() as i32, (y / (h / 13.0)).floor() as i32);
                let r = hash(bx, by, s.wrapping_add(frame as u32));
                let fine = hash(cx, cy, s.wrapping_add(frame as u32 * 7 + 1));
                let (ox, oy) = if r > 0.55 { ((hash(bx, by, s + 99) - 0.5) * amp, (hash(bx, by, s + 7) - 0.5) * amp * 0.3) } else { (0.0, 0.0) };
                let swap = fine < k;
                let kk = if swap { 1.0 } else { 0.0 };
                let kk = mix([kk; 4], [k; 4], 1.0 - env)[0];
                split(t, x - ox, y - oy, (fine - 0.5) * amt * env * 30.0 * u, kk)
            })
        }
        "earthquake" => {
            let shake = t.px("shake");
            let (sh, n) = shutter(t, 0.03);
            let rot = 0.04 * shake / w.max(1.0) * 10.0;
            paint(t.w, t.h, |x, y| {
                temporal(x, y, p, sh, n, &|x, y, q| {
                    let e = bell(q);
                    let ox = (vnoise(q * 45.0, 0.5, s) - 0.5) * 2.0 * shake * e;
                    let oy = (vnoise(q * 45.0, 7.5, s + 1) - 0.5) * 2.0 * shake * e;
                    let r = (vnoise(q * 30.0, 3.5, s + 2) - 0.5) * rot * e;
                    let zoom = 1.0 + 0.08 * e;
                    let (sn, cs) = r.sin_cos();
                    let (ux, uy) = ((x - w / 2.0) / zoom, (y - h / 2.0) / zoom);
                    let (sx, sy) = (ux * cs - uy * sn + w / 2.0 - ox, ux * sn + uy * cs + h / 2.0 - oy);
                    ab(t, sx, sy, mid_mix(q))
                })
            })
        }
        "flicker" => {
            let nf = t.num("flickers").round().clamp(1.0, 40.0);
            let j = (p * nf * 2.0).floor() as i32;
            let show_b = hash(j, 3, s) < smoothstep(0.1, 0.9, p);
            let gain = 1.0 + (hash(j, 5, s) - 0.5) * 1.2 * env;
            paint(t.w, t.h, |x, y| {
                let c = if show_b { t.pb(x, y) } else { t.pa(x, y) };
                [c[0] * gain, c[1] * gain, c[2] * gain, c[3]]
            })
        }
        "glass" => {
            let cell = t.px("cell_size").max(4.0);
            let refr = t.px("refraction");
            paint(t.w, t.h, |x, y| {
                let (gx, gy) = ((x / cell).floor() as i32, (y / cell).floor() as i32);
                let (mut best, mut second, mut id) = (f32::INFINITY, f32::INFINITY, (0, 0));
                for j in -1..=1 {
                    for i in -1..=1 {
                        let (cx, cy) = (gx + i, gy + j);
                        let px = (cx as f32 + hash(cx, cy, s)) * cell;
                        let py = (cy as f32 + hash(cx, cy, s + 1)) * cell;
                        let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                        if d < best {
                            second = best;
                            best = d;
                            id = (cx, cy);
                        } else if d < second {
                            second = d;
                        }
                    }
                }
                let tau = 0.3 + 0.4 * hash(id.0, id.1, s + 2);
                let kk = smoothstep(tau - 0.1, tau + 0.1, p);
                let ang = hash(id.0, id.1, s + 3) * TAU;
                let off = refr * env * hash(id.0, id.1, s + 4);
                let c = ab(t, x + ang.cos() * off, y + ang.sin() * off, kk);
                let edge = (-(second - best) / (1.5 * u.max(0.5))).exp() * env;
                let facet = 1.0 + (hash(id.0, id.1, s + 5) - 0.5) * 0.4 * env;
                add_light([c[0] * facet, c[1] * facet, c[2] * facet, c[3]], [1.0; 4], edge * 0.8)
            })
        }
        "glitch" => {
            let amt = t.frac("amount");
            let frame = (p * 24.0).floor() as u32;
            paint(t.w, t.h, |x, y| {
                let band_h = h / (6.0 + 18.0 * hash(frame as i32, 0, s));
                let band = (y / band_h.max(1.0)).floor() as i32;
                let r = hash(band, frame as i32, s);
                let ox = if r > 0.6 { (hash(band, frame as i32, s + 1) - 0.5) * amt * env * w * 0.25 } else { 0.0 };
                let tau = 0.4 + 0.2 * hash(band, 9, s);
                let kk = if p > tau { 1.0 } else { 0.0 };
                let kk = kk * env + k * (1.0 - env);
                let c = split(t, x - ox, y, amt * env * 18.0 * u * (r - 0.3).max(0.0) * 2.0, kk);
                // occasional blocky corruption
                let (bx, by) = ((x / (24.0 * u.max(0.2))).floor() as i32, (y / (12.0 * u.max(0.2))).floor() as i32);
                if hash(bx, by, s + frame * 3) > 1.0 - 0.04 * amt * env {
                    let v = hash(bx, by, s + 77);
                    return mix(c, [v, 1.0 - v, v * 0.5, 1.0], 0.8);
                }
                c
            })
        }
        "grunge" => {
            let scale = t.px("scale").max(4.0);
            let st = WipeStyle { feather: 2.0 * t.scale, border: t.px("edge_width"), border_color: t.color("edge_color"), aa: 1.0 };
            field_wipe(t, st, move |x, y| {
                let n = fbm(x / scale, y / scale, s) * 0.8 + vnoise(x / (scale * 0.15), y / (scale * 0.15), s + 9) * 0.2;
                n * scale * 3.0
            })
        }
        "kaleidoscope" => {
            let n = t.num("segments").round().clamp(2.0, 32.0);
            let zoom = 1.0 + t.frac("zoom") * env;
            let seg = TAU / n;
            paint(t.w, t.h, |x, y| {
                let (rx, ry) = (x - w / 2.0, y - h / 2.0);
                let r = (rx * rx + ry * ry).sqrt() / zoom;
                let th = ry.atan2(rx) + p * PI * 0.5;
                let a = (th.rem_euclid(seg) - seg / 2.0).abs();
                let (kx, ky) = (w / 2.0 + r * a.cos(), h / 2.0 + r * a.sin());
                let kal = ab(t, kx, ky, k);
                let plain = ab(t, x, y, k);
                mix(plain, kal, smoothstep(0.0, 0.6, env))
            })
        }
        "liquid_distortion" => {
            let amt = t.px("amount");
            let sc = t.px("scale").max(4.0);
            paint(t.w, t.h, |x, y| {
                let ox = (fbm(x / sc + p * 2.0, y / sc, s) - 0.5) * 2.0 * amt * env;
                let oy = (fbm(x / sc, y / sc - p * 2.0, s + 5) - 0.5) * 2.0 * amt * env;
                ab(t, x + ox, y + oy, k)
            })
        }
        "tv_power" => {
            let glow = t.color("glow_color");
            let line = t.px("line").max(1.0);
            let (img, q) = if p < 0.5 { (t.a, p * 2.0) } else { (t.b, (1.0 - p) * 2.0) };
            // q: 0 = full picture, 1 = collapsed to a dot
            let sy = 1.0 - smoothstep(0.0, 0.65, q);
            let sx = 1.0 - smoothstep(0.6, 1.0, q);
            let hy = (h / 2.0 * sy).max(line / 2.0 * sx);
            let hx = w / 2.0 * sx;
            let white = smoothstep(0.2, 0.7, q);
            paint(t.w, t.h, |x, y| {
                let (rx, ry) = (x - w / 2.0, y - h / 2.0);
                let mut c = [0.0, 0.0, 0.0, 1.0];
                if rx.abs() <= hx && ry.abs() <= hy && hx > 0.0 {
                    let lx = w / 2.0 + rx / sx.max(1e-3);
                    let ly = h / 2.0 + ry / sy.max(1e-3);
                    let src = img.sample_bilinear_clamped(lx, ly.clamp(0.0, h));
                    c = over(c, mix(src, glow, white));
                }
                let dy = (ry.abs() - hy).max(0.0);
                let dx = (rx.abs() - hx).max(0.0);
                let halo = (-(dx * dx + dy * dy).sqrt() / (line * 3.0)).exp() * white;
                add_light(c, glow, halo * 0.8)
            })
        }
        "vhs_damage" => {
            let amt = t.frac("amount");
            let frame = (p * 30.0).floor() as u32;
            let band_y = h * (1.2 - 1.4 * p);
            paint(t.w, t.h, |x, y| {
                let row = (y / (2.0 * u.max(0.5))).floor() as i32;
                let jitter = (hash(row, frame as i32, s) - 0.5) * amt * env * w * 0.02;
                let in_band = (-((y - band_y) / (h * 0.06)).powi(2)).exp();
                let ox = jitter + in_band * amt * env * w * 0.08 * (vnoise(y * 0.05, p * 20.0, s) - 0.5) * 2.0;
                let c = split(t, x - ox, y, amt * env * 10.0 * u, k);
                let grain = (hash(x as i32, y as i32, s + frame) - 0.5) * 0.25 * amt * env;
                let band_noise = in_band * env * amt * 0.5 * hash(x as i32 / 3, row, frame);
                let l = super::luma(c);
                let desat = mix(c, [l, l, l, c[3]], 0.4 * amt * env);
                [desat[0] + grain + band_noise, desat[1] + grain + band_noise, desat[2] + grain + band_noise, desat[3]]
            })
        }
        _ => return None,
    })
}

#[inline]
fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    super::over(dst, src)
}

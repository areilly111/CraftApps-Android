//! Moving-frame transitions: Animation, Slide, Transformers and the camera-style Smart Tools.

use std::f32::consts::{FRAC_PI_2, PI};

use super::cards::{Card, V3, eye, focal_for};
use super::{Tx, bell, ease_in, ease_in_out, ease_out, hash, line_blur, mid_mix, mix, over, paint, scale4, shutter, smoothstep, temporal, zoom_blur};
use crate::image::Image;

/// One textured card in a 3D scene.
#[derive(Clone, Copy)]
pub(crate) struct Layer<'a> {
    pub card: Card,
    pub front: &'a Image,
    /// Image on the back face (mirrored so it reads correctly after a half turn about
    /// the y axis (`mirror_y = false`) or the x axis (`mirror_y = true`)); `None` = one-sided.
    pub back: Option<&'a Image>,
    pub mirror_y: bool,
    /// Visible part of the card in image pixels (x0, y0, x1, y1).
    pub clip: [f32; 4],
    pub shade: bool,
    pub opacity: f32,
}

impl<'a> Layer<'a> {
    pub fn new(card: Card, front: &'a Image) -> Self {
        Layer { card, front, back: None, mirror_y: false, clip: [0.0, 0.0, front.w as f32, front.h as f32], shade: false, opacity: 1.0 }
    }
    fn with_back(mut self, back: &'a Image, mirror_y: bool) -> Self {
        self.back = Some(back);
        self.mirror_y = mirror_y;
        self
    }
    fn clipped(mut self, c: [f32; 4]) -> Self {
        self.clip = c;
        self
    }
    fn shaded(mut self) -> Self {
        self.shade = true;
        self
    }
}

/// Composite the layers seen through screen pixel (x, y), nearest last.
pub(crate) fn composite(layers: &[Layer], eye: V3, x: f32, y: f32) -> [f32; 4] {
    const MAX: usize = 8;
    let mut hits = [(0.0f32, [0.0f32; 4]); MAX];
    let mut nh = 0;
    for l in layers {
        let Some((lx, ly, depth, front)) = l.card.hit(eye, x, y) else { continue };
        let [x0, y0, x1, y1] = l.clip;
        let edge = (lx - x0).min(x1 - lx).min(ly - y0).min(y1 - ly);
        let cov = (edge + 0.5).clamp(0.0, 1.0);
        if cov <= 0.0 {
            continue;
        }
        let (img, sx, sy) = if front {
            (l.front, lx, ly)
        } else if let Some(b) = l.back {
            if l.mirror_y { (b, lx, b.h as f32 - ly) } else { (b, b.w as f32 - lx, ly) }
        } else {
            continue;
        };
        let mut c = img.sample_bilinear_clamped(sx, sy);
        let mut k = cov * l.opacity;
        if l.shade {
            let n = cross(l.card.ex, l.card.ey);
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
            let facing = (n[2] / len).abs();
            let light = 0.45 + 0.55 * facing;
            c = [c[0] * light, c[1] * light, c[2] * light, c[3]];
        }
        k = k.clamp(0.0, 1.0);
        if nh < MAX {
            hits[nh] = (depth, scale4(c, k));
            nh += 1;
        }
    }
    let hits = &mut hits[..nh];
    hits.sort_by(|a, b| b.0.total_cmp(&a.0));
    hits.iter().fold([0.0; 4], |acc, (_, c)| over(acc, *c))
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Render a card scene built for each motion-blur time sample.
fn scene<'a>(t: &Tx<'a>, focal: f32, blur: (f32, usize), build: impl Fn(f32) -> Vec<Layer<'a>> + Sync) -> Image {
    let (sh, n) = blur;
    let qs: Vec<f32> = if n <= 1 || sh <= 0.0 { vec![t.p] } else { (0..n).map(|i| (t.p + (i as f32 / (n - 1) as f32 - 0.5) * sh).clamp(0.0, 1.0)).collect() };
    let scenes: Vec<Vec<Layer>> = qs.iter().map(|&q| build(q)).collect();
    let e = eye(t.wf, t.hf, focal);
    let inv = 1.0 / scenes.len() as f32;
    paint(t.w, t.h, |x, y| {
        let mut acc = [0.0; 4];
        for s in &scenes {
            let c = composite(s, e, x, y);
            for k in 0..4 {
                acc[k] += c[k];
            }
        }
        scale4(acc, inv)
    })
}

fn center3(t: &Tx) -> V3 {
    [t.wf / 2.0, t.hf / 2.0, 0.0]
}

/// "Back-out" easing with overshoot `s` (0 = plain ease-out).
fn back_out(x: f32, s: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) - 1.0;
    1.0 + (s + 1.0) * x * x * x + s * x * x
}

/// Damped spring from 0 to exactly 1.
fn spring(x: f32, bounce: f32) -> f32 {
    let w = (2.0 + 4.0 * bounce) * PI;
    let k = 4.0 + 6.0 * (1.0 - bounce);
    let f = |u: f32| 1.0 - (-k * u).exp() * (w * u).cos();
    // a slower start (u = x^1.6) so the move reads before the bounce
    let x = x.clamp(0.0, 1.0).powf(1.6);
    f(x) + x * (1.0 - f(1.0))
}

pub(super) fn apply(id: &str, t: &Tx) -> Option<Image> {
    let c = center3(t);
    let (w, h) = (t.wf, t.hf);
    let (dx, dy) = t.dir();
    let horiz = dx != 0.0;
    let travel = t.travel();
    let far = focal_for(w, h, 0.0);
    Some(match id {
        // ---------------- Slide ----------------
        "push" => scene(t, far, shutter(t, 0.12), |q| {
            let e = ease_in_out(q);
            vec![
                Layer::new(Card::flat().translate([dx * travel * e, dy * travel * e, 0.0]), t.a),
                Layer::new(Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]), t.b),
            ]
        }),
        "slide" => scene(t, far, shutter(t, 0.12), |q| {
            let e = ease_in_out(q);
            vec![Layer::new(Card::flat(), t.a), Layer::new(Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]), t.b)]
        }),
        "split" => {
            let vertical_cut = t.choice("orientation") == 0;
            scene(t, far, shutter(t, 0.12), move |q| {
                let e = ease_in_out(q);
                let (half_l, half_r) =
                    if vertical_cut { ([0.0, 0.0, w / 2.0, h], [w / 2.0, 0.0, w, h]) } else { ([0.0, 0.0, w, h / 2.0], [0.0, h / 2.0, w, h]) };
                let (ox, oy) = if vertical_cut { (w / 2.0 * e, 0.0) } else { (0.0, h / 2.0 * e) };
                vec![
                    Layer::new(Card::flat(), t.b),
                    Layer::new(Card::flat().translate([-ox, -oy, -0.01]), t.a).clipped(half_l),
                    Layer::new(Card::flat().translate([ox, oy, -0.01]), t.a).clipped(half_r),
                ]
            })
        }
        "whip" => {
            let e = {
                let p = t.p;
                // fast in the middle: a steep sigmoid normalised to 0..1
                let s = |u: f32| 1.0 / (1.0 + (-(u - 0.5) * 14.0).exp());
                (s(p) - s(0.0)) / (s(1.0) - s(0.0))
            };
            let pushed = scene(t, far, (0.0, 1), |_| {
                vec![
                    Layer::new(Card::flat().translate([dx * travel * e, dy * travel * e, 0.0]), t.a),
                    Layer::new(Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]), t.b),
                ]
            });
            let len = t.frac("blur") * bell(t.p) * travel * 0.18;
            if len < 0.75 {
                pushed
            } else {
                let n = super::taps(len * 2.0);
                paint(t.w, t.h, |x, y| line_blur(&pushed, x, y, dx * len, dy * len, n))
            }
        }
        "roll" => {
            // A and B ride a big wheel whose hub sits below (or beside) the frame.
            let r = if horiz { h * 2.0 } else { w * 2.0 };
            let hub: V3 = if horiz { [w / 2.0, h / 2.0 + r, 0.0] } else { [w / 2.0 - r, h / 2.0, 0.0] };
            let gap = (if horiz { w } else { h }) * 1.15 / r;
            let sign = if horiz { dx.signum() } else { dy.signum() };
            scene(t, far, shutter(t, 0.1), move |q| {
                let a = ease_in_out(q) * gap * sign;
                vec![Layer::new(Card::flat().rot_z(hub, a), t.a), Layer::new(Card::flat().rot_z(hub, a - gap * sign), t.b)]
            })
        }
        "roll_3d" => {
            // a cube: A on the front face, B on the face that turns to the front
            let side = if horiz { w } else { h };
            let cc: V3 = [w / 2.0, h / 2.0, side / 2.0];
            let focal = focal_for(w, h, t.frac("perspective"));
            let sign = if horiz { dx.signum() } else { -dy.signum() };
            scene(t, focal, (0.0, 1), move |q| {
                let a = ease_in_out(q) * FRAC_PI_2 * sign;
                let back = side * 0.45 * bell(q);
                let rot = |k: Card, ang: f32| if horiz { k.rot_y(cc, ang) } else { k.rot_x(cc, ang) };
                vec![
                    Layer::new(rot(Card::flat(), a).translate([0.0, 0.0, back]), t.a).shaded(),
                    Layer::new(rot(rot(Card::flat(), -FRAC_PI_2 * sign), a).translate([0.0, 0.0, back]), t.b).shaded(),
                ]
            })
        }
        "film_roll" => {
            let gap = t.px("gap");
            let gcol = t.color("gap_color");
            let step = travel + gap;
            let (sh, n) = shutter(t, 0.1);
            let inside = move |u: f32, v: f32| u >= 0.0 && v >= 0.0 && u < w && v < h;
            paint(t.w, t.h, |x, y| {
                temporal(x, y, t.p, sh, n, &|x, y, q| {
                    let off = step * ease_in_out(q);
                    let (ax, ay) = (x - dx * off, y - dy * off);
                    if inside(ax, ay) {
                        return t.sac(ax, ay);
                    }
                    let (bx, by) = (ax + dx * step, ay + dy * step);
                    if inside(bx, by) {
                        return t.sbc(bx, by);
                    }
                    // the gap between the frames, with sprocket holes along both edges
                    let across = if horiz { y } else { x };
                    let across_len = if horiz { h } else { w };
                    let along = if horiz { ax } else { ay };
                    let edge_band = (across / across_len < 0.12) || (across / across_len > 0.88);
                    let pitch = across_len / 14.0;
                    let f = (across / pitch).fract();
                    let g = ((along.rem_euclid(step) - travel) / gap.max(1.0)).clamp(0.0, 1.0);
                    let hole = !edge_band && (0.25..0.75).contains(&f) && (0.3..0.7).contains(&g);
                    if hole { [0.5, 0.5, 0.5, 1.0] } else { gcol }
                })
            })
        }
        "stretch" => {
            let e = ease_in_out(t.p);
            // B grows from the entry side while A squeezes towards the exit side
            let (len, axis_x) = if horiz { (w, true) } else { (h, false) };
            let fwd = if horiz { dx > 0.0 } else { dy > 0.0 };
            let split = if fwd { len * e } else { len * (1.0 - e) };
            paint(t.w, t.h, |x, y| {
                let s = if axis_x { x } else { y };
                let (in_b, local) = if fwd {
                    if s < split { (true, s / split.max(1e-3) * len) } else { (false, (s - split) / (len - split).max(1e-3) * len) }
                } else if s >= split {
                    (true, (s - split) / (len - split).max(1e-3) * len)
                } else {
                    (false, s / split.max(1e-3) * len)
                };
                let (sx, sy) = if axis_x { (local, y) } else { (x, local) };
                if in_b { t.sbc(sx, sy) } else { t.sac(sx, sy) }
            })
        }
        // ---------------- Animation ----------------
        "block_motion" => {
            let cols = t.num("blocks").round().clamp(2.0, 32.0);
            let rows = (cols * h / w).round().clamp(1.0, 32.0);
            let (lanes, cells) = if horiz { (rows, cols) } else { (cols, rows) };
            let (sh, n) = shutter(t, 0.08);
            let n = n.min(4);
            let seed = 17;
            paint(t.w, t.h, |x, y| {
                temporal(x, y, t.p, sh, n, &|x, y, q| {
                    let (along, across, len_al, len_ac) = if horiz { (x, y, w, h) } else { (y, x, h, w) };
                    let sgn = if horiz { dx } else { dy };
                    let lane = (across / len_ac * lanes).floor().clamp(0.0, lanes - 1.0);
                    let cell_len = len_al / cells;
                    let mut col = [0.0f32; 4];
                    let mut got_a = false;
                    let mut got_b = [0.0f32; 4];
                    let mut has_b = false;
                    for ci in 0..cells as i32 {
                        // leading blocks (towards the exit side) leave first
                        let order = if sgn > 0.0 { (cells as i32 - 1 - ci) as f32 } else { ci as f32 } / cells;
                        let delay = 0.6 * (0.6 * order + 0.4 * hash(ci, lane as i32, seed));
                        let qi = ((q - delay) / (1.0 - 0.6)).clamp(0.0, 1.0);
                        let e = ease_in_out(qi);
                        let off_a = sgn * len_al * e;
                        let off_b = sgn * len_al * (e - 1.0);
                        let (lo, hi) = (ci as f32 * cell_len, (ci + 1) as f32 * cell_len);
                        let pa = along - off_a;
                        if !got_a && pa >= lo && pa < hi {
                            let (sx, sy) = if horiz { (pa, y) } else { (x, pa) };
                            col = t.sac(sx, sy);
                            got_a = true;
                        }
                        let pb = along - off_b;
                        if !has_b && pb >= lo && pb < hi {
                            let (sx, sy) = if horiz { (pb, y) } else { (x, pb) };
                            got_b = t.sbc(sx, sy);
                            has_b = true;
                        }
                    }
                    if got_a { over(got_b, col) } else { got_b }
                })
            })
        }
        "flip_motion" | "spin_3d" => {
            let focal = focal_for(w, h, t.frac("perspective"));
            let flip = id == "flip_motion";
            let sign = if horiz { dx.signum() } else { -dy.signum() };
            let blur = if flip { shutter(t, 0.06) } else { (0.0, 1) };
            scene(t, focal, blur, move |q| {
                let (e, back) = if flip {
                    // ease in, then settle with a small overshoot
                    (if q < 0.5 { ease_in(q * 2.0) * 0.5 } else { 0.5 + back_out(q * 2.0 - 1.0, 1.2) * 0.5 }, 0.25 * bell(q))
                } else {
                    (ease_in_out(q), 0.8 * bell(q))
                };
                let a = e * PI * sign;
                let card = if horiz { Card::flat().rot_y(c, a) } else { Card::flat().rot_x(c, a) };
                let card = card.translate([0.0, 0.0, back * w.max(h)]);
                vec![Layer::new(card, t.a).with_back(t.b, !horiz).shaded()]
            })
        }
        "spinback_3d" => {
            let depth = t.frac("depth");
            let focal = focal_for(w, h, 0.6);
            let sign = if horiz { dx.signum() } else { -dy.signum() };
            scene(t, focal, (0.0, 1), move |q| {
                let z = depth * w.max(h) * 1.5 * bell(q);
                if q < 0.5 {
                    let a = ease_in(q * 2.0) * FRAC_PI_2 * sign;
                    let k = if horiz { Card::flat().rot_x(c, a) } else { Card::flat().rot_y(c, a) };
                    vec![Layer::new(k.translate([0.0, 0.0, z]), t.a).shaded()]
                } else {
                    let a = -(1.0 - ease_out((q - 0.5) * 2.0)) * FRAC_PI_2 * sign;
                    let k = if horiz { Card::flat().rot_x(c, a) } else { Card::flat().rot_y(c, a) };
                    vec![Layer::new(k.translate([0.0, 0.0, z]), t.b).shaded()]
                }
            })
        }
        "fold_motion" => {
            // The entry half folds over onto the exit half (showing its back), then the folded
            // stack swings towards the viewer about the exit edge, revealing B.
            let focal = focal_for(w, h, t.frac("perspective"));
            let sg = if dx > 0.0 || dy > 0.0 { 1.0 } else { -1.0 };
            let (entry, mid_axis, exit_axis): ([f32; 4], V3, V3) = match (horiz, sg > 0.0) {
                (true, true) => ([0.0, 0.0, w / 2.0, h], [w / 2.0, 0.0, 0.0], [w, 0.0, 0.0]),
                (true, false) => ([w / 2.0, 0.0, w, h], [w / 2.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                (false, true) => ([0.0, 0.0, w, h / 2.0], [0.0, h / 2.0, 0.0], [0.0, h, 0.0]),
                (false, false) => ([0.0, h / 2.0, w, h], [0.0, h / 2.0, 0.0], [0.0, 0.0, 0.0]),
            };
            let exit = match (horiz, sg > 0.0) {
                (true, true) => [w / 2.0, 0.0, w, h],
                (true, false) => [0.0, 0.0, w / 2.0, h],
                (false, true) => [0.0, h / 2.0, w, h],
                (false, false) => [0.0, 0.0, w, h / 2.0],
            };
            scene(t, focal, (0.0, 1), move |q| {
                let phi = PI * ease_in_out((q * 2.0).min(1.0)) * sg;
                let psi = FRAC_PI_2 * ease_in(((q - 0.5) * 2.0).clamp(0.0, 1.0)) * sg;
                let rot = |k: Card, axis: V3, a: f32| if horiz { k.rot_y(axis, a) } else { k.rot_x(axis, -a) };
                let flap = rot(rot(Card::flat().translate([0.0, 0.0, -0.5]), mid_axis, phi), exit_axis, psi);
                let base = rot(Card::flat(), exit_axis, psi);
                vec![
                    Layer::new(Card::flat().translate([0.0, 0.0, 1.0]), t.b),
                    Layer::new(base, t.a).clipped(exit).shaded(),
                    Layer::new(flap, t.a).with_back(t.a, !horiz).clipped(entry).shaded(),
                ]
            })
        }
        "pop_motion" => {
            let s = t.frac("overshoot") * 4.0;
            scene(t, far, shutter(t, 0.05), move |q| {
                let sa = 1.0 - 0.3 * ease_in((q / 0.6).min(1.0));
                let qb = ((q - 0.15) / 0.85).clamp(0.0, 1.0);
                let sb = back_out(qb, s).max(0.0);
                let mut la = Layer::new(Card::flat().scale_about(c, sa), t.a);
                la.opacity = 1.0 - smoothstep(0.35, 0.8, q);
                let mut lb = Layer::new(Card::flat().scale_about(c, sb.max(1e-3)).translate([0.0, 0.0, -0.01]), t.b);
                lb.opacity = smoothstep(0.1, 0.3, q);
                vec![la, lb]
            })
        }
        "pull_motion" => scene(t, far, shutter(t, 0.1), move |q| {
            let e = ease_in_out(q);
            let s = 1.0 - 0.25 * bell(q);
            vec![
                Layer::new(Card::flat().scale_about(c, s).translate([dx * travel * e, dy * travel * e, 0.0]), t.a),
                Layer::new(Card::flat().scale_about(c, s).translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]), t.b),
            ]
        }),
        "spin_motion" => {
            let turn = t.num("rotation").to_radians() * if t.choice("spin") == 1 { -1.0 } else { 1.0 };
            scene(t, far, shutter(t, 0.06), move |q| {
                if q < 0.5 {
                    let k = ease_in(q * 2.0);
                    vec![Layer::new(Card::flat().rot_z(c, turn * 0.5 * k).scale_about(c, (1.0 - k).max(1e-3)), t.a)]
                } else {
                    let k = ease_out((q - 0.5) * 2.0);
                    vec![Layer::new(Card::flat().rot_z(c, -turn * 0.5 * (1.0 - k)).scale_about(c, k.max(1e-3)), t.b)]
                }
            })
        }
        "spring_motion" => {
            let bounce = t.frac("bounce");
            scene(t, far, shutter(t, 0.08), move |q| {
                let e = spring(q, bounce);
                vec![
                    Layer::new(Card::flat().translate([dx * travel * e, dy * travel * e, 0.0]), t.a),
                    Layer::new(Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]), t.b),
                ]
            })
        }
        "travel_motion" => {
            let zoom = t.frac("zoom");
            let step = travel * 1.08;
            scene(t, far, shutter(t, 0.08), move |q| {
                let s = 1.0 - zoom * bell(q);
                let pan = step * ease_in_out(q);
                vec![
                    Layer::new(Card::flat().translate([dx * pan, dy * pan, 0.0]).scale_about(c, s), t.a),
                    Layer::new(Card::flat().translate([dx * (pan - step), dy * (pan - step), 0.0]).scale_about(c, s), t.b),
                ]
            })
        }
        // ---------------- Transformers ----------------
        "frame" => {
            let fw = t.px("frame_width");
            let fc = t.color("frame_color");
            let smin = t.frac("scale").clamp(0.05, 1.0);
            let p = t.p;
            // phase: shrink into a frame, travel across, grow out of the frame
            let shrink = smoothstep(0.0, 0.3, p) * (1.0 - smoothstep(0.7, 1.0, p));
            let s = 1.0 - (1.0 - smin) * shrink;
            let move_t = ease_in_out(((p - 0.25) / 0.5).clamp(0.0, 1.0));
            let border = fw * shrink;
            // just far enough that the off-stage frame (and its border) is fully out of view
            let step = travel * 0.5 * (1.0 + s) + border * 2.0 + 2.0;
            let layers = [(t.a, move_t * step), (t.b, (move_t - 1.0) * step)];
            paint(t.w, t.h, |x, y| {
                let mut out = [0.0; 4];
                for (img, off) in layers {
                    // inverse of: scale about centre, then translate along the direction
                    let (ux, uy) = (x - dx * off, y - dy * off);
                    let lx = (ux - w / 2.0) / s + w / 2.0;
                    let ly = (uy - h / 2.0) / s + h / 2.0;
                    let bpx = border / s;
                    let inside = lx >= 0.0 && lx < w && ly >= 0.0 && ly < h;
                    let in_frame = lx >= -bpx && lx < w + bpx && ly >= -bpx && ly < h + bpx;
                    let c = if inside {
                        img.sample_bilinear_clamped(lx, ly)
                    } else if in_frame && bpx > 0.0 {
                        fc
                    } else {
                        continue;
                    };
                    out = over(out, c);
                }
                out
            })
        }
        "louver" => {
            let n = t.num("slats").round().clamp(1.0, 64.0);
            let horizontal_slats = t.choice("orientation") == 0;
            let p = t.p;
            paint(t.w, t.h, |x, y| {
                let (pos, len) = if horizontal_slats { (y, h) } else { (x, w) };
                let sl = len / n;
                let i = (pos / sl).floor().clamp(0.0, n - 1.0);
                let mid = (i + 0.5) * sl;
                let stagger = 0.4 * i / n.max(1.0);
                let q = ((p - stagger) / 0.6).clamp(0.0, 1.0);
                let th = ease_in_out(q) * PI;
                let co = th.cos();
                if co.abs() < 1e-3 {
                    return [0.0, 0.0, 0.0, 1.0];
                }
                let d = pos - mid;
                let local = d / co; // A (front) for cos > 0, B (back, re-flipped) for cos < 0
                if local.abs() > sl / 2.0 {
                    return [0.0, 0.0, 0.0, 1.0];
                }
                let src = mid + if co > 0.0 { local } else { -local };
                let (sx, sy) = if horizontal_slats { (x, src) } else { (src, y) };
                let c = if co > 0.0 { t.sac(sx, sy) } else { t.sbc(sx, sy) };
                let light = 0.5 + 0.5 * co.abs();
                [c[0] * light, c[1] * light, c[2] * light, c[3]]
            })
        }
        "mirror_transition" => {
            let (lo, hi) = (0.0f32, travel);
            let refl = travel * 0.3;
            let e = lo - refl + ease_in_out(t.p) * (hi - lo + 2.0 * refl);
            let neg = dx + dy < 0.0;
            paint(t.w, t.h, |x, y| {
                // along-axis coordinate measured from the entry side
                let s = if horiz {
                    if neg { w - x } else { x }
                } else if neg {
                    h - y
                } else {
                    y
                };
                if s < e {
                    return t.pb(x, y);
                }
                let a = t.pa(x, y);
                let d = s - e;
                if d < refl {
                    let m = e - d; // mirror of s about the moving line
                    let (sx, sy) = if horiz { (if neg { w - m } else { m }, y) } else { (x, if neg { h - m } else { m }) };
                    let r = t.sbc(sx, sy);
                    return mix(a, r, 0.75 * (1.0 - d / refl));
                }
                a
            })
        }
        "page_peel" => page_peel(t),
        "slice" => {
            let n = t.num("slices").round().clamp(1.0, 64.0);
            let ang = t.num("angle").to_radians();
            let (ax, ay) = (ang.cos(), ang.sin()); // slice direction (they slide along this)
            let (px_, py_) = (-ay, ax); // across
            let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for (cx, cy) in corners {
                let v = cx * px_ + cy * py_;
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let span = (hi - lo).max(1.0);
            let dist = w.abs() * ax.abs() + h * ay.abs() + span * 0.2;
            let (sh, ns) = shutter(t, 0.08);
            paint(t.w, t.h, |x, y| {
                temporal(x, y, t.p, sh, ns, &|x, y, q| {
                    let i = (((x * px_ + y * py_) - lo) / span * n).floor().clamp(0.0, n - 1.0);
                    let sign = if (i as i32) % 2 == 0 { 1.0 } else { -1.0 };
                    let qi = ((q - 0.3 * i / n.max(1.0)) / 0.7).clamp(0.0, 1.0);
                    let off = sign * dist * ease_in(qi);
                    let (sx, sy) = (x - ax * off, y - ay * off);
                    let zb = 1.0 + 0.05 * (1.0 - q);
                    let b = t.sbc((x - w / 2.0) / zb + w / 2.0, (y - h / 2.0) / zb + h / 2.0);
                    if sx >= 0.0 && sx < w && sy >= 0.0 && sy < h {
                        // keep the slice's own band only (slices do not bleed into each other)
                        let j = (((sx * px_ + sy * py_) - lo) / span * n).floor().clamp(0.0, n - 1.0);
                        if j == i {
                            return over(b, t.sac(sx, sy));
                        }
                    }
                    b
                })
            })
        }
        "wave" => {
            let amp = t.px("amplitude");
            let lambda = t.px("wavelength").max(4.0);
            let neg = dx + dy < 0.0;
            let len = travel;
            let band = len * 0.25;
            let e = -band + t.p * (len + 2.0 * band);
            let env = bell(t.p);
            paint(t.w, t.h, |x, y| {
                let s = if horiz {
                    if neg { w - x } else { x }
                } else if neg {
                    h - y
                } else {
                    y
                };
                let across = if horiz { y } else { x };
                let near = (-((s - e) / band).powi(2)).exp();
                let disp = amp * env * near * (2.0 * PI * (across / lambda) - t.p * 6.0).sin();
                let (sx, sy) = if horiz { (x + disp, y + disp * 0.3) } else { (x + disp * 0.3, y + disp) };
                let k = 1.0 - smoothstep(-band * 0.3, band * 0.3, s - e);
                mix(t.sac(sx, sy), t.sbc(sx, sy), k)
            })
        }
        // ---------------- Smart Tools (camera-style) ----------------
        "motion_camera" => {
            let zoom = t.frac("zoom");
            let rot = t.num("rotation").to_radians();
            let mb = t.frac("motion_blur");
            let p = t.p;
            let k = mid_mix(p);
            // A pushes in and turns; B arrives from the same push, settling to rest
            let (za, ra) = (1.0 + zoom * ease_in(p.min(0.7) / 0.7), rot * ease_in(p));
            let (zb, rb) = (1.0 + zoom * (1.0 - ease_out(((p - 0.3) / 0.7).clamp(0.0, 1.0))), -rot * (1.0 - ease_out(p)));
            let blur = mb * bell(p) * 0.35;
            let n = super::taps(blur * w * 0.3);
            paint(t.w, t.h, |x, y| {
                let sample = |img: &Image, z: f32, r: f32| {
                    let (s, co) = (-r).sin_cos();
                    let (ux, uy) = ((x - w / 2.0) / z, (y - h / 2.0) / z);
                    let (lx, ly) = (ux * co - uy * s + w / 2.0, ux * s + uy * co + h / 2.0);
                    zoom_blur(img, lx, ly, w / 2.0, h / 2.0, blur, n)
                };
                let a = if k < 1.0 { sample(t.a, za, ra) } else { [0.0; 4] };
                let b = if k > 0.0 { sample(t.b, zb, rb) } else { [0.0; 4] };
                mix(a, b, k)
            })
        }
        "motion_tween" => {
            let s = t.frac("scale");
            let p = t.p;
            let e = ease_in_out(p);
            let (za, zb) = (1.0 + s * e, 1.0 + s * (e - 1.0));
            paint(t.w, t.h, |x, y| {
                let a = t.sac((x - w / 2.0) / za + w / 2.0, (y - h / 2.0) / za + h / 2.0);
                let b = t.sbc((x - w / 2.0) / zb + w / 2.0, (y - h / 2.0) / zb + h / 2.0);
                mix(a, b, e)
            })
        }
        _ => return None,
    })
}

/// Page Peel: the corner lifts and rolls over a cylinder of radius `r`; the peeled part lies
/// face-down on top of the page (its back shows a washed-out mirror of A) and B shows beneath.
fn page_peel(t: &Tx) -> Image {
    let (w, h) = (t.wf, t.hf);
    let r = t.px("radius").max(1.0);
    let shadow = t.flag("shadow");
    let (ox, oy) = match t.choice("corner") {
        0 => (0.0, 0.0),
        1 => (w, 0.0),
        3 => (0.0, h),
        _ => (w, h),
    };
    let (mut nx, mut ny) = (w / 2.0 - ox, h / 2.0 - oy);
    let nl = (nx * nx + ny * ny).sqrt().max(1e-3);
    nx /= nl;
    ny /= nl;
    let smax = (w - 2.0 * ox).abs() * nx.abs() + (h - 2.0 * oy).abs() * ny.abs();
    let f = -1.0 + t.p * (smax + r * 1.2 + 2.0);
    let inside = |px: f32, py: f32| px >= 0.0 && py >= 0.0 && px < w && py < h;
    paint(t.w, t.h, |x, y| {
        let s = (x - ox) * nx + (y - oy) * ny;
        let page_at = |so: f32| (x - nx * (s - so), y - ny * (s - so));
        let back = |c: [f32; 4], light: f32| {
            let m = mix(c, [c[3], c[3], c[3], c[3]], 0.6);
            [m[0] * light, m[1] * light, m[2] * light, m[3]]
        };
        // 1. the flipped flat part (height 2r) over the unpeeled page
        if s >= f {
            let so = 2.0 * f - s - PI * r;
            let (px, py) = page_at(so);
            if inside(px, py) {
                return back(t.sac(px, py), 0.95);
            }
        }
        let d = f - s;
        if (0.0..r).contains(&d) {
            // 2. upper half of the cylinder (back side)
            let phi = PI - (d / r).asin();
            let so = f - r * phi;
            let (px, py) = page_at(so);
            if inside(px, py) {
                return back(t.sac(px, py), 0.75 + 0.25 * phi.sin());
            }
            // 3. lower half (front side, shaded)
            let phi = (d / r).asin();
            let so = f - r * phi;
            let (px, py) = page_at(so);
            if inside(px, py) {
                let c = t.sac(px, py);
                let l = 1.0 - 0.45 * (phi / FRAC_PI_2);
                return [c[0] * l, c[1] * l, c[2] * l, c[3]];
            }
        }
        if s >= f {
            return t.pa(x, y);
        }
        // 4. revealed incoming clip, with a soft shadow along the curl
        let b = t.pb(x, y);
        if shadow {
            let k = 1.0 - 0.45 * (-(d - r).max(0.0) / (r * 1.5)).exp() * smoothstep(0.0, 0.05, t.p) * (1.0 - smoothstep(0.9, 1.0, t.p));
            return [b[0] * k, b[1] * k, b[2] * k, b[3]];
        }
        b
    })
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn easings_hit_their_ends() {
        for f in [spring(0.0, 0.4), back_out(0.0, 1.0)] {
            assert!(f.abs() < 1e-5);
        }
        assert!((spring(1.0, 0.4) - 1.0).abs() < 1e-5);
        assert!((back_out(1.0, 1.0) - 1.0).abs() < 1e-5);
        assert!(back_out(0.8, 1.6) > 1.0, "overshoots");
    }

    #[test]
    fn flat_card_maps_pixels_to_themselves() {
        let e = eye(64.0, 36.0, 500.0);
        let (lx, ly, _, front) = Card::flat().hit(e, 10.5, 20.5).unwrap();
        assert!((lx - 10.5).abs() < 1e-3 && (ly - 20.5).abs() < 1e-3 && front);
        let flipped = Card::flat().rot_y([32.0, 18.0, 0.0], PI);
        let (lx, _, _, front) = flipped.hit(e, 10.5, 20.5).unwrap();
        assert!(!front && (lx - 53.5).abs() < 1e-2, "{lx}");
    }
}

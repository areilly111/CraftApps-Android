//! Simulation effects, part 2: layer-shattering and surface simulations — CC Ball Action,
//! CC Pixel Polly, CC Scatterize, Card Dance, Shatter (planar pieces in perspective), Caustics,
//! Wave World (stepped 2D wave equation) and Foam (stepped bubble sim).
//!
//! Written from the public descriptions of the effects' behaviour. Pieces are textured convex
//! polygons placed in 3D and drawn through a per-piece inverse homography, far to near.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::card3d::{Lighting, Proj};
use crate::generate::value_noise;
use crate::sim::{Acc, Post, Shape, Sprite, SpritePlan, h, hs, pct, pt, raster, spec, splat};
use crate::util::{Plane, SimCache, gauss_plane, layer_or_self, params_key, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

// ------------------------------------------------------------------ 3D pieces

pub(crate) type M3 = [[f64; 3]; 3];

fn mat_mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// Rotation from Euler angles (radians), applied X then Y then Z.
fn rot_xyz(x: f64, y: f64, z: f64) -> M3 {
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mat_mul(&rz, &mat_mul(&ry, &rx))
}

/// Rotation by `angle` about unit `axis` (Rodrigues).
pub(crate) fn rot_axis(axis: [f64; 3], angle: f64) -> M3 {
    let l = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if l < 1e-12 || angle == 0.0 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (x, y, z) = (axis[0] / l, axis[1] / l, axis[2] / l);
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

fn inv3(m: &M3) -> Option<M3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d],
    ])
}

/// Perspective camera looking down +z at the layer plane z = 0, `d` pixels away, centred on
/// `c` (buffer pixels): screen = c + (P - c) · d / (d + z).
#[derive(Clone, Copy)]
struct Cam {
    c: [f64; 2],
    d: f64,
}

impl Cam {
    fn proj(self) -> Proj {
        Proj::simple(self.c, self.d)
    }
}

/// A planar textured piece ready to draw.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    /// Screen (buffer px, homogeneous) → piece-local (a, b, w).
    pub inv: [[f64; 3]; 3],
    /// Rest centre of the piece in texture pixels.
    pub c0: [f32; 2],
    /// Convex polygon (texture pixels), counter-clockwise in y-down space.
    pub poly: [[f32; 2]; 6],
    pub n: u8,
    pub bbox: [f32; 4],
    pub shade: f32,
    /// Lighting: colour multiplier and additive specular highlight.
    pub tint: [f32; 3],
    pub spec: [f32; 3],
    pub alpha: f32,
    /// Flat straight colour instead of the texture.
    pub flat: Option<[f32; 3]>,
    /// Facing away from the camera (draw the back texture).
    pub back: bool,
    /// Back faces sample their texture mirrored about the piece centre: 0 no, 1 vertically
    /// (flipped about the X axis), 2 horizontally (about the Y axis).
    pub back_mirror: u8,
    depth: f64,
}

/// A piece texture: the layer itself or another image (buffer-aligned).
#[derive(Clone, Debug)]
pub enum PieceTex {
    Layer,
    Image(Image),
}

/// What a piece-drawing effect (Card Dance, Shatter, Card Wipe) draws this frame: pieces sorted
/// far to near over a transparent frame, textured by `front` (and `back` for pieces facing
/// away; none = they take `front`).
#[derive(Clone, Debug)]
pub struct PiecePlan {
    pub pieces: Vec<Piece>,
    pub front: PieceTex,
    pub back: Option<PieceTex>,
}

impl PiecePlan {
    /// Rasterise over a transparent frame of the layer `b`'s size.
    pub fn finish(&self, mut b: Buf) -> Buf {
        let mut out = Image::new(b.img.width, b.img.height);
        let front = match &self.front {
            PieceTex::Layer => &b.img,
            PieceTex::Image(i) => i,
        };
        let back = self.back.as_ref().map(|t| match t {
            PieceTex::Layer => &b.img,
            PieceTex::Image(i) => i,
        });
        draw_pieces(&mut out, &self.pieces, front, back);
        b.img = out;
        b
    }
}

/// The piece plan of Card Dance, Shatter (Rendered view) or Card Wipe for layer `b` (its pixels
/// may be read: gradient maps and textures default to the layer), `None` for other effects or
/// views drawn otherwise.
pub fn piece_plan(id: &str, ctx: &EffectCtx, b: &Buf) -> Option<PiecePlan> {
    match id {
        "ec.sim.carddance" => Some(card_dance_plan(ctx, b)),
        "ec.sim.shatter" if ctx.params.e("view") == 0 => match shatter_impl(ctx, b) {
            ShatterPlan::Pieces(plan) => Some(plan),
            ShatterPlan::Wire(_) => None,
        },
        "ec.transition.cardwipe" => crate::transition::card_wipe_plan(ctx, b),
        _ => None,
    }
}

impl Piece {
    /// Place polygon `poly` (texture px, around rest centre `c0`) with rotation `r` and its
    /// centre at 3D `pos` (buffer px; z away from the camera), seen through `cam`.
    pub(crate) fn new(poly: &[[f32; 2]], c0: [f32; 2], r: &M3, pos: [f64; 3], cam: impl Into<Proj>) -> Option<Piece> {
        let cam: Proj = cam.into();
        let n = poly.len().min(6);
        if n < 3 {
            return None;
        }
        let m = &cam.m;
        let row = |i: usize| -> [f64; 3] {
            [
                m[i][0] * r[0][0] + m[i][1] * r[1][0] + m[i][2] * r[2][0],
                m[i][0] * r[0][1] + m[i][1] * r[1][1] + m[i][2] * r[2][1],
                m[i][0] * pos[0] + m[i][1] * pos[1] + m[i][2] * pos[2] + m[i][3],
            ]
        };
        let hm: M3 = [row(0), row(1), row(2)];
        let mut bb = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for v in &poly[..n] {
            let (a, b) = ((v[0] - c0[0]) as f64, (v[1] - c0[1]) as f64);
            let w = hm[2][0] * a + hm[2][1] * b + hm[2][2];
            if w < cam.near {
                return None;
            }
            let sx = ((hm[0][0] * a + hm[0][1] * b + hm[0][2]) / w) as f32;
            let sy = ((hm[1][0] * a + hm[1][1] * b + hm[1][2]) / w) as f32;
            bb = [bb[0].min(sx), bb[1].min(sy), bb[2].max(sx), bb[3].max(sy)];
        }
        let inv = inv3(&hm)?;
        let mut pp = [[0.0f32; 2]; 6];
        pp[..n].copy_from_slice(&poly[..n]);
        // Normalise winding so "inside" is cross >= 0 for every edge.
        let area: f32 = (0..n).map(|i| pp[i][0] * pp[(i + 1) % n][1] - pp[(i + 1) % n][0] * pp[i][1]).sum();
        if area < 0.0 {
            pp[..n].reverse();
        }
        // The front faces -z at rest; it faces the camera when its normal points at the eye.
        let eye = cam.eye();
        let nrm = [-r[0][2], -r[1][2], -r[2][2]];
        let to_eye = [eye[0] - pos[0], eye[1] - pos[1], eye[2] - pos[2]];
        let facing = nrm[0] * to_eye[0] + nrm[1] * to_eye[1] + nrm[2] * to_eye[2];
        let depth = hm[2][2];
        Some(Piece {
            inv,
            c0,
            poly: pp,
            n: n as u8,
            bbox: [bb[0] - 1.0, bb[1] - 1.0, bb[2] + 1.0, bb[3] + 1.0],
            shade: 1.0,
            tint: [1.0; 3],
            spec: [0.0; 3],
            alpha: 1.0,
            flat: None,
            back: facing < 0.0,
            back_mirror: 0,
            depth,
        })
    }

    /// Lambert factor of the piece normal against a light shining along +z.
    fn facing(r: &M3) -> f32 {
        r[2][2].abs() as f32
    }

    /// Light the piece (rotation `r`, centre `pos`) with Lighting / Material.
    pub(crate) fn light(&mut self, l: &Lighting, r: &M3, pos: [f64; 3], eye: [f64; 3]) {
        let (t, s) = l.shade([-r[0][2], -r[1][2], -r[2][2]], pos, eye);
        self.tint = t;
        self.spec = s;
    }
}

impl From<Cam> for Proj {
    fn from(c: Cam) -> Proj {
        c.proj()
    }
}

/// Draw pieces (already sorted far → near) over `out`.
pub(crate) fn draw_pieces(out: &mut Image, pieces: &[Piece], front: &Image, back: Option<&Image>) {
    raster(
        out,
        pieces,
        |q| Some(q.bbox),
        |q, x, y| {
            let (xf, yf) = (x as f64, y as f64);
            let m = &q.inv;
            let w = m[2][0] * xf + m[2][1] * yf + m[2][2];
            if w.abs() < 1e-12 {
                return None;
            }
            let u = ((m[0][0] * xf + m[0][1] * yf + m[0][2]) / w) as f32 + q.c0[0];
            let v = ((m[1][0] * xf + m[1][1] * yf + m[1][2]) / w) as f32 + q.c0[1];
            let n = q.n as usize;
            let mut md = f32::INFINITY;
            for i in 0..n {
                let a = q.poly[i];
                let b = q.poly[(i + 1) % n];
                let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                let l = (ex * ex + ey * ey).sqrt().max(1e-6);
                md = md.min((ex * (v - a[1]) - ey * (u - a[0])) / l);
            }
            // Coverage extends half a pixel past the edge so abutting pieces leave no seams.
            let cov = (md + 1.0).clamp(0.0, 1.0);
            if cov <= 0.0 {
                return None;
            }
            let (c, a) = match q.flat {
                Some(c) => (c, 1.0),
                None => {
                    let tex = if q.back { back.unwrap_or(front) } else { front };
                    let (u, v) = match (q.back, q.back_mirror) {
                        (true, 1) => (u, 2.0 * q.c0[1] - v),
                        (true, 2) => (2.0 * q.c0[0] - u, v),
                        _ => (u, v),
                    };
                    unpremul(tex.sample_bilinear(u as f64, v as f64))
                }
            };
            let a = a * cov * q.alpha;
            if a <= 1e-6 {
                return None;
            }
            let k = q.shade;
            let o: [f32; 3] = std::array::from_fn(|i| c[i] * k * q.tint[i] + q.spec[i]);
            Some([o[0] * a, o[1] * a, o[2] * a, a])
        },
        Acc::Over,
    );
}

pub(crate) fn sort_far_first(pieces: &mut [Piece]) {
    pieces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
}

// ------------------------------------------------------------------ CC Ball Action

fn ball_action(ctx: &EffectCtx, b: Buf) -> Buf {
    ball_action_plan(ctx, &b).finish(b)
}

/// CC Ball Action's balls (far to near), coloured by the layer's pixels.
fn ball_action_plan(ctx: &EffectCtx, b: &Buf) -> SpritePlan {
    let pr = ctx.params;
    let scatter = pr.f("scatter") as f32;
    let axis = pr.e("rotationAxis");
    let rot = pr.f("rotation").to_radians();
    let twist_prop = pr.e("twistProperty");
    let twist = pr.f("twistAngle").to_radians();
    let spacing = pr.f("gridSpacing").max(1.0) as f32;
    let size = pr.f("ballSize") as f32 / 100.0;
    let instab = pr.f("instabilityState").to_radians() as f32;
    let seed = ctx.seed.wrapping_mul(0x2545f491);
    let (lw, lh) = (ctx.layer_size[0] as f32, ctx.layer_size[1] as f32);
    let s = b.scale as f32;
    let nx = (lw / spacing).ceil().max(1.0) as u32;
    let ny = (lh / spacing).ceil().max(1.0) as u32;
    let d = 2.0 * lw.max(lh) as f64;
    let (cx, cy) = (lw as f64 * 0.5, lh as f64 * 0.5);
    let src = &b.img;
    let ax: [f64; 3] = match axis {
        0 => [1.0, 0.0, 0.0],
        1 => [0.0, 1.0, 0.0],
        2 => [0.0, 0.0, 1.0],
        3 => [1.0, 1.0, 0.0],
        4 => [1.0, 0.0, 1.0],
        5 => [0.0, 1.0, 1.0],
        _ => [1.0, 1.0, 1.0],
    };
    let maxr = ((lw * lw + lh * lh).sqrt() * 0.5).max(1.0);
    let mut balls: Vec<(f64, Sprite)> = (0..nx * ny)
        .into_par_iter()
        .filter_map(|i| {
            let gx = (i % nx) as f32 * spacing + spacing * 0.5;
            let gy = (i / nx) as f32 * spacing + spacing * 0.5;
            let (sx, sy) = b.to_px([gx as f64, gy as f64]);
            let (c, a) = unpremul(src.sample_bilinear(sx, sy));
            if a <= 0.0 || size <= 0.0 {
                return None;
            }
            let tp = match twist_prop {
                0 => gx / lw.max(1.0),
                1 => gy / lh.max(1.0),
                2 => ((gx - cx as f32).hypot(gy - cy as f32)) / maxr,
                _ => crate::util::rgb3([c[0], c[1], c[2], 0.0]).iter().sum::<f32>() / 3.0,
            } as f64;
            let ang = rot + twist * tp;
            // Scatter: random 3D offsets whose direction drifts with the instability state.
            let ph = h(i, 1, seed) * std::f32::consts::TAU + instab;
            let sc = scatter * h(i, 2, seed);
            let off = [(sc * ph.cos()) as f64, (sc * ph.sin()) as f64, (scatter * hs(i, 3, seed)) as f64];
            let p0 = [gx as f64 - cx + off[0], gy as f64 - cy + off[1], off[2]];
            let m = rot_axis(ax, ang);
            let q: [f64; 3] = std::array::from_fn(|k| m[k][0] * p0[0] + m[k][1] * p0[1] + m[k][2] * p0[2]);
            if d + q[2] <= d * 0.05 {
                return None;
            }
            let persp = d / (d + q[2]);
            let (bx, by) = b.to_px([cx + q[0] * persp, cy + q[1] * persp]);
            let r = spacing * 0.5 * size * persp as f32 * s;
            Some((q[2], Sprite::new(bx as f32, by as f32, r, [c[0], c[1], c[2], a], Shape::Sphere)))
        })
        .collect();
    balls.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sprites: Vec<Sprite> = balls.into_iter().map(|(_, s)| s).collect();
    SpritePlan { sprites, tints: vec![], acc: Acc::Over, post: Post::Replace }
}

// ------------------------------------------------------------------ CC Pixel Polly

fn pixel_polly(ctx: &EffectCtx, b: Buf) -> Buf {
    match pixel_polly_plan(ctx, &b) {
        Some(plan) => plan.finish(b),
        None => b,
    }
}

/// CC Pixel Polly's pieces, `None` before Start Time (the layer as is).
fn pixel_polly_plan(ctx: &EffectCtx, b: &Buf) -> Option<PiecePlan> {
    let pr = ctx.params;
    let t = ctx.time - pr.f("startTime");
    if t <= 0.0 {
        return None;
    }
    let force = pr.f("force");
    let gravity = pr.f("gravity");
    let spin = pr.f("spinning").to_radians();
    let fc = pr.v2("forceCenter");
    let dir_rand = pr.f("directionRandomness") / 100.0;
    let spd_rand = pr.f("speedRandomness") / 100.0;
    let g = pr.f("gridSpacing").max(1.0);
    let object = pr.e("object");
    let depth_sort = pr.b("enableDepthSort");
    let seed = ctx.seed.wrapping_mul(0x632be5ab);
    let [lw, lh] = ctx.layer_size;
    let s = b.scale;
    let cam = Cam { c: [b.offset[0] + lw * 0.5 * s, b.offset[1] + lh * 0.5 * s], d: 2.0 * lw.max(lh) * s };
    let nx = (lw / g).ceil().max(1.0) as u32;
    let ny = (lh / g).ceil().max(1.0) as u32;
    let tri = object <= 1;
    let textured = object == 1 || object == 3;
    let maxd = lw.max(lh);
    let src = &b.img;
    let per = if tri { 2 } else { 1 };
    let mut pieces: Vec<Piece> = (0..nx * ny * per)
        .into_par_iter()
        .filter_map(|i| {
            let cell = i / per;
            let (x0, y0) = ((cell % nx) as f64 * g, (cell / nx) as f64 * g);
            let poly: Vec<[f64; 2]> = if !tri {
                vec![[x0, y0], [x0 + g, y0], [x0 + g, y0 + g], [x0, y0 + g]]
            } else if i % 2 == 0 {
                vec![[x0, y0], [x0 + g, y0], [x0, y0 + g]]
            } else {
                vec![[x0 + g, y0], [x0 + g, y0 + g], [x0, y0 + g]]
            };
            let c = [poly.iter().map(|v| v[0]).sum::<f64>() / poly.len() as f64, poly.iter().map(|v| v[1]).sum::<f64>() / poly.len() as f64];
            let (dx, dy) = (c[0] - fc[0], c[1] - fc[1]);
            let dist = (dx * dx + dy * dy).sqrt().max(1e-6);
            let ang = dy.atan2(dx) + dir_rand * std::f64::consts::PI * hs(i, 1, seed) as f64;
            let speed = force / 100.0 * 0.6 * maxd / (1.0 + dist / (0.5 * maxd)) * (1.0 + spd_rand * hs(i, 2, seed) as f64);
            let vz = -speed * 0.5 * h(i, 3, seed) as f64;
            let pos = [c[0] + ang.cos() * speed * t, c[1] + ang.sin() * speed * t + 0.5 * gravity * 0.5 * lh * t * t, vz * t];
            let axis = [hs(i, 4, seed) as f64, hs(i, 5, seed) as f64, hs(i, 6, seed) as f64];
            let r = rot_axis(axis, t * (std::f64::consts::PI * h(i, 7, seed) as f64 + spin));
            let tex: Vec<[f32; 2]> = poly.iter().map(|v| [(v[0] * s + b.offset[0]) as f32, (v[1] * s + b.offset[1]) as f32]).collect();
            let c0 = [(c[0] * s + b.offset[0]) as f32, (c[1] * s + b.offset[1]) as f32];
            let posb = [pos[0] * s + b.offset[0], pos[1] * s + b.offset[1], pos[2] * s];
            let mut pc = Piece::new(&tex, c0, &r, posb, cam)?;
            pc.shade = 0.35 + 0.65 * Piece::facing(&r);
            if !textured {
                let (cc, a) = unpremul(src.sample_bilinear(c0[0] as f64, c0[1] as f64));
                if a <= 0.0 {
                    return None;
                }
                pc.flat = Some(cc);
                pc.alpha = a;
            }
            Some(pc)
        })
        .collect();
    if depth_sort {
        sort_far_first(&mut pieces);
    }
    Some(PiecePlan { pieces, front: PieceTex::Layer, back: None })
}

// ------------------------------------------------------------------ CC Scatterize

fn scatterize(ctx: &EffectCtx, b: Buf) -> Buf {
    match scatterize_plan(ctx, &b) {
        Some(plan) => plan.finish(b),
        None => b,
    }
}

/// CC Scatterize's pixel dots, `None` when nothing moves (the layer as is).
fn scatterize_plan(ctx: &EffectCtx, b: &Buf) -> Option<SpritePlan> {
    let pr = ctx.params;
    let scatter = pr.f("scatter") as f32;
    let right = pr.f("rightTwist").to_radians();
    let left = pr.f("leftTwist").to_radians();
    if scatter == 0.0 && right == 0.0 && left == 0.0 {
        return None;
    }
    let add = pr.e("transferMode") == 1;
    let seed = ctx.seed.wrapping_mul(0x7feb352d);
    let s = b.scale;
    let (w, hh) = (b.img.width, b.img.height);
    let (cx, cy) = (b.offset[0] + ctx.layer_size[0] * 0.5 * s, b.offset[1] + ctx.layer_size[1] * 0.5 * s);
    let half = (ctx.layer_size[0] * 0.5 * s).max(1.0);
    let d = 2.0 * ctx.layer_size[0].max(ctx.layer_size[1]) * s;
    let src = &b.img;
    let sprites: Vec<Sprite> = (0..w * hh)
        .into_par_iter()
        .filter_map(|i| {
            let px = src.data[i as usize];
            if px[3] <= 0.0 {
                return None;
            }
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            // Twist about the horizontal axis through the centre, growing towards each side.
            let k = ((x - cx) / half).abs().min(1.0);
            let ang = if x < cx { left } else { right } * k;
            let (sa, ca) = ang.sin_cos();
            let (yy, zz) = ((y - cy) * ca, (y - cy) * sa);
            let persp = d / (d + zz).max(d * 0.05);
            let jx = hs(i, 1, seed) * scatter * s as f32;
            let jy = hs(i, 2, seed) * scatter * s as f32;
            let sx = cx + (x - cx) * persp + jx as f64;
            let sy = cy + yy * persp + jy as f64;
            let (c, a) = unpremul(px);
            Some(Sprite::new(sx as f32, sy as f32, 0.6 * persp as f32, [c[0], c[1], c[2], a], Shape::Disc))
        })
        .collect();
    Some(SpritePlan { sprites, tints: vec![], acc: if add { Acc::Add } else { Acc::Over }, post: Post::Replace })
}

/// What a particle effect that reads the layer's pixels draws this frame (CC Ball Action, CC
/// Pixel Polly, CC Scatterize, Shatter's wireframe views), for layer `b`; `None` for other
/// effects or a frame showing the layer as is.
pub enum PixelPlan {
    Sprites(SpritePlan),
    Pieces(PiecePlan),
}

/// The [`PixelPlan`] of effect `id` for layer `b` (its pixels are read).
pub fn pixel_plan(id: &str, ctx: &EffectCtx, b: &Buf) -> Option<PixelPlan> {
    match id {
        "ec.sim.ccballaction" => Some(PixelPlan::Sprites(ball_action_plan(ctx, b))),
        "ec.sim.ccpixelpolly" => pixel_polly_plan(ctx, b).map(PixelPlan::Pieces),
        "ec.sim.ccscatterize" => scatterize_plan(ctx, b).map(PixelPlan::Sprites),
        "ec.sim.shatter" => Some(match shatter_impl(ctx, b) {
            ShatterPlan::Pieces(plan) => PixelPlan::Pieces(plan),
            ShatterPlan::Wire(plan) => PixelPlan::Sprites(plan),
        }),
        _ => None,
    }
}

// ------------------------------------------------------------------ Card Dance

const CD_SOURCES: &[&str] = &["None", "Intensity 1", "Red 1", "Green 1", "Blue 1", "Alpha 1", "Intensity 2", "Red 2", "Green 2", "Blue 2", "Alpha 2"];
/// (param id prefix, twirl-down group match id) per transformed card property.
const CD_PROPS: [(&str, &str); 8] = [
    ("xPos", "xPosition"),
    ("yPos", "yPosition"),
    ("zPos", "zPosition"),
    ("xRot", "xRotation"),
    ("yRot", "yRotation"),
    ("zRot", "zRotation"),
    ("xScale", "xScale"),
    ("yScale", "yScale"),
];

fn card_source(src: u32, g1: [f32; 4], g2: [f32; 4]) -> f32 {
    let (c1, a1) = unpremul(g1);
    let (c2, a2) = unpremul(g2);
    let lum = |c: [f32; 3]| effectcraft_color::luminance(c[0], c[1], c[2]);
    match src {
        1 => lum(c1),
        2 => c1[0],
        3 => c1[1],
        4 => c1[2],
        5 => a1,
        6 => lum(c2),
        7 => c2[0],
        8 => c2[1],
        9 => c2[2],
        10 => a2,
        _ => 0.0,
    }
}

fn card_dance(ctx: &EffectCtx, b: Buf) -> Buf {
    card_dance_plan(ctx, &b).finish(b)
}

fn card_dance_plan(ctx: &EffectCtx, b: &Buf) -> PiecePlan {
    let pr = ctx.params;
    let rows = pr.f("rows").clamp(1.0, 1000.0) as u32;
    let cols = if pr.e("rowsColumns") == 1 { rows } else { pr.f("columns").clamp(1.0, 1000.0) as u32 };
    let g1 = layer_or_self(ctx, b, "gradientLayer1", true, true);
    let g2 = layer_or_self(ctx, b, "gradientLayer2", true, true);
    let back = ctx.layer_param("backLayer", true).map(|o| crate::util::fit_layer(ctx, b, &o, true));
    let [lw, lh] = ctx.layer_size;
    let s = b.scale;
    let (cw, ch) = (lw / cols as f64, lh / rows as f64);
    let cam = crate::card3d::projection(ctx, b);
    let eye = cam.eye();
    let light = Lighting::from(ctx, b);
    let props: Vec<(u32, f64, f64)> =
        CD_PROPS.iter().map(|(id, g)| (pr.e(&format!("{g}/{id}Source")), pr.f(&format!("{g}/{id}Multiplier")), pr.f(&format!("{g}/{id}Offset")))).collect();
    let mut pieces: Vec<Piece> = (0..rows * cols)
        .into_par_iter()
        .filter_map(|i| {
            let (cx0, cy0) = ((i % cols) as f64 * cw, (i / cols) as f64 * ch);
            let c = [cx0 + cw * 0.5, cy0 + ch * 0.5];
            let (bx, by) = (c[0] * s + b.offset[0], c[1] * s + b.offset[1]);
            let (gp1, gp2) = (g1.sample_bilinear(bx, by), g2.sample_bilinear(bx, by));
            let val = |k: usize| {
                let (sr, m, o) = props[k];
                o + m * card_source(sr, gp1, gp2) as f64
            };
            let (sx, sy) = (val(6), val(7));
            let r = rot_xyz(val(3).to_radians(), val(4).to_radians(), val(5).to_radians());
            let rs: M3 = std::array::from_fn(|k| [r[k][0] * sx, r[k][1] * sy, r[k][2]]);
            let pos = [bx + val(0) * cw * s, by + val(1) * ch * s, val(2) * cw.max(ch) * s];
            let (tx0, ty0) = ((cx0 * s + b.offset[0]) as f32, (cy0 * s + b.offset[1]) as f32);
            let (tw, th) = ((cw * s) as f32, (ch * s) as f32);
            let poly = [[tx0, ty0], [tx0 + tw, ty0], [tx0 + tw, ty0 + th], [tx0, ty0 + th]];
            let mut pc = Piece::new(&poly, [bx as f32, by as f32], &rs, pos, cam)?;
            pc.light(&light, &r, pos, eye);
            Some(pc)
        })
        .collect();
    sort_far_first(&mut pieces);
    PiecePlan { pieces, front: PieceTex::Layer, back: back.map(PieceTex::Image) }
}

// ------------------------------------------------------------------ Shatter

/// Shatter's Pattern options; the last, Custom, takes its pieces from Custom Shatter Map.
const SHATTER_PATTERNS: &[&str] = &["Bricks", "Glass", "Hexagons", "Squares", "Triangles", "Custom"];

/// Pattern cells (polygons in pattern space, before rotation) covering `[x0, x1]×[y0, y1]`.
fn pattern_cells(kind: u32, cell: f64, x0: f64, y0: f64, x1: f64, y1: f64, seed: u32) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let i0 = (x0 / cell).floor() as i64 - 1;
    let i1 = (x1 / cell).ceil() as i64 + 1;
    let j0 = (y0 / cell).floor() as i64 - 1;
    let j1 = (y1 / cell).ceil() as i64 + 1;
    match kind {
        0 => {
            let bh = cell * 0.5;
            let r0 = (y0 / bh).floor() as i64 - 1;
            let r1 = (y1 / bh).ceil() as i64 + 1;
            for r in r0..=r1 {
                let sh = if r.rem_euclid(2) == 1 { cell * 0.5 } else { 0.0 };
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell + sh, r as f64 * bh);
                    out.push(vec![[x, y], [x + cell, y], [x + cell, y + bh], [x, y + bh]]);
                }
            }
        }
        1 => {
            // Jittered lattice split into triangles along a random diagonal.
            let jit = |i: i64, j: i64| -> [f64; 2] {
                let (a, b) = (i as u32, j as u32);
                [i as f64 * cell + hs(a, b.wrapping_add(17), seed) as f64 * cell * 0.35, j as f64 * cell + hs(a, b.wrapping_add(91), seed) as f64 * cell * 0.35]
            };
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (p00, p10, p11, p01) = (jit(i, j), jit(i + 1, j), jit(i + 1, j + 1), jit(i, j + 1));
                    if h(i as u32, j as u32, seed ^ 0x55) < 0.5 {
                        out.push(vec![p00, p10, p11]);
                        out.push(vec![p00, p11, p01]);
                    } else {
                        out.push(vec![p00, p10, p01]);
                        out.push(vec![p10, p11, p01]);
                    }
                }
            }
        }
        2 => {
            let r = cell / 3f64.sqrt();
            let dx = 1.5 * r;
            let dy = 3f64.sqrt() * r;
            let c0 = (x0 / dx).floor() as i64 - 1;
            let c1 = (x1 / dx).ceil() as i64 + 1;
            let rr0 = (y0 / dy).floor() as i64 - 1;
            let rr1 = (y1 / dy).ceil() as i64 + 1;
            for ci in c0..=c1 {
                for rj in rr0..=rr1 {
                    let cx = ci as f64 * dx;
                    let cy = (rj as f64 + if ci.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) * dy;
                    out.push(
                        (0..6)
                            .map(|k| {
                                let a = k as f64 * std::f64::consts::PI / 3.0;
                                [cx + r * a.cos(), cy + r * a.sin()]
                            })
                            .collect(),
                    );
                }
            }
        }
        3 => {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell, j as f64 * cell);
                    out.push(vec![[x, y], [x + cell, y], [x + cell, y + cell], [x, y + cell]]);
                }
            }
        }
        _ => {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell, j as f64 * cell);
                    if (i + j).rem_euclid(2) == 0 {
                        out.push(vec![[x, y], [x + cell, y], [x, y + cell]]);
                        out.push(vec![[x + cell, y], [x + cell, y + cell], [x, y + cell]]);
                    } else {
                        out.push(vec![[x, y], [x + cell, y], [x + cell, y + cell]]);
                        out.push(vec![[x, y], [x + cell, y + cell], [x, y + cell]]);
                    }
                }
            }
        }
    }
    out
}

/// `∫0^t e^{-ks} ds` and `∫0^t ∫0^u e^{-ks} ds du` (drag-damped ballistic factors).
fn drag_factors(k: f64, t: f64) -> (f64, f64) {
    if k < 1e-6 {
        (t, 0.5 * t * t)
    } else {
        let e = (-k * t).exp();
        ((1.0 - e) / k, t / k - (1.0 - e) / (k * k))
    }
}
/// Shatter's Front / Side / Back Mode options.
const SHATTER_TEXTURE_MODES: [&str; 3] = ["Color", "Layer", "Tinted Layer"];

/// One face's texture: a flat colour, a layer, or a layer tinted by the Textures colour.
struct Face {
    mode: u32,
    img: Option<Image>,
}

impl Face {
    fn from(ctx: &EffectCtx, b: &Buf, mode: &str, layer: &str, default_mode: u32) -> Face {
        let mode = ctx.params.get(mode).map(|v| v.as_enum()).unwrap_or(default_mode);
        // A face layer of None is the effect's own layer.
        let img = (mode != 0).then(|| layer_or_self(ctx, b, layer, true, true));
        Face { mode, img }
    }
}

/// Group key of a Custom Shatter Map colour (pieces of one colour break as one).
fn map_key(c: [f32; 4]) -> u32 {
    let (c, a) = unpremul(c);
    if a < 0.5 {
        return u32::MAX;
    }
    let q = |v: f32| ((v.clamp(0.0, 1.0) * 15.0).round() as u32) & 15;
    q(c[0]) << 8 | q(c[1]) << 4 | q(c[2])
}

fn shatter(ctx: &EffectCtx, b: Buf) -> Buf {
    match shatter_impl(ctx, &b) {
        ShatterPlan::Pieces(plan) => plan.finish(b),
        ShatterPlan::Wire(plan) => plan.finish(b),
    }
}

/// What Shatter draws: the pieces (Rendered view) or the wireframe lines.
enum ShatterPlan {
    Pieces(PiecePlan),
    Wire(SpritePlan),
}

fn shatter_impl(ctx: &EffectCtx, b: &Buf) -> ShatterPlan {
    let pr = ctx.params;
    let view = pr.e("view");
    let render = pr.e("render");
    let kind = pr.e("shape/pattern");
    let custom = kind as usize == SHATTER_PATTERNS.len() - 1;
    let reps = pr.f("shape/repetitions").clamp(1.0, 500.0);
    let dir = pr.f("shape/direction").to_radians();
    let origin = pr.v2("shape/origin");
    let forces = [
        (pr.v2("force1/force1Position"), pr.f("force1/force1Depth"), pr.f("force1/force1Radius"), pr.f("force1/force1Strength")),
        (pr.v2("force2/force2Position"), pr.f("force2/force2Depth"), pr.f("force2/force2Radius"), pr.f("force2/force2Strength")),
    ];
    let rot_speed = pr.f("physics/rotationSpeed");
    let tumble = pr.e("physics/tumbleAxis");
    let randomness = pr.f("physics/randomness");
    let k = pr.f("physics/viscosity").max(0.0) * 5.0;
    let mass_var = pr.f("physics/massVariance") / 100.0;
    let gravity = pr.f("physics/gravity");
    let gdir = pr.f("physics/gravityDirection").to_radians();
    let ginc = pr.f("physics/gravityInclination").to_radians();
    let seed = (pr.f("randomSeed") as u32).wrapping_mul(0x9e3779b1) ^ ctx.seed;
    let [lw, lh] = ctx.layer_size;
    let s = b.scale;
    let t = ctx.time.max(0.0);
    let unit = lw * 0.1;
    let cell = lw / reps;
    let extrude = pr.f("shape/extrusionDepth").max(0.0) * cell * s;
    let cam = crate::card3d::projection(ctx, b);
    let eye = cam.eye();
    let light = Lighting::from(ctx, b);
    // Gradient: with a gradient layer only pieces at least as bright as 1 − Shatter Threshold
    // (Invert Gradient: as dark) break.
    let gradient = ctx.layer_param("gradient/gradientLayer", true).map(|o| crate::util::fit_layer(ctx, b, &o, true));
    let threshold = pr.get("gradient/shatterThreshold").map(Value::as_f64).unwrap_or(0.0) / 100.0;
    let invert_gradient = pr.b("gradient/invertGradient");
    // Textures.
    let tex_color = pr.get("textures/color").map(|v| v.as_color()).unwrap_or([1.0; 4]);
    let tex_opacity = pr.get("textures/opacity").map(Value::as_f64).unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    let front = Face::from(ctx, b, "textures/frontMode", "textures/frontLayer", 1);
    let side = Face::from(ctx, b, "textures/sideMode", "textures/sideLayer", 0);
    let backf = Face::from(ctx, b, "textures/backMode", "textures/backLayer", 1);
    let custom_map = custom.then(|| layer_or_self(ctx, b, "shape/customShatterMap", true, true));
    let white_fixed = pr.b("shape/whiteTilesFixed");
    let (sd, cd) = dir.sin_cos();
    let to_layer = |q: [f64; 2]| [origin[0] + q[0] * cd - q[1] * sd, origin[1] + q[0] * sd + q[1] * cd];
    // Pattern-space bounds of the layer rectangle.
    let corners = [[0.0, 0.0], [lw, 0.0], [lw, lh], [0.0, lh]].map(|c: [f64; 2]| {
        let (dx, dy) = (c[0] - origin[0], c[1] - origin[1]);
        [dx * cd + dy * sd, -dx * sd + dy * cd]
    });
    let (px0, px1) = (corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min), corners.iter().map(|c| c[0]).fold(f64::NEG_INFINITY, f64::max));
    let (py0, py1) = (corners.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min), corners.iter().map(|c| c[1]).fold(f64::NEG_INFINITY, f64::max));
    // Custom Shatter Map: square cells grouped by the map's colour.
    let cells = pattern_cells(if custom { 3 } else { kind }, cell, px0, py0, px1, py1, seed);
    let polys: Vec<Vec<[f64; 2]>> = cells.iter().map(|poly| poly.iter().map(|&q| to_layer(q)).collect()).collect();
    let centre = |lp: &[[f64; 2]]| [lp.iter().map(|v| v[0]).sum::<f64>() / lp.len() as f64, lp.iter().map(|v| v[1]).sum::<f64>() / lp.len() as f64];
    let keys: Vec<u32> = match &custom_map {
        Some(m) => polys
            .iter()
            .map(|lp| {
                let c = centre(lp);
                map_key(m.sample_bilinear(c[0] * s + b.offset[0], c[1] * s + b.offset[1]))
            })
            .collect(),
        None => (0..polys.len() as u32).collect(),
    };
    // Group centres (custom map) — each group moves as one rigid piece.
    let mut groups: std::collections::HashMap<u32, ([f64; 2], f64)> = std::collections::HashMap::new();
    for (lp, &key) in polys.iter().zip(&keys) {
        let c = centre(lp);
        let e = groups.entry(key).or_insert(([0.0; 2], 0.0));
        e.0[0] += c[0];
        e.0[1] += c[1];
        e.1 += 1.0;
    }
    let g3 = [gdir.sin() * ginc.cos() * gravity * unit, -gdir.cos() * ginc.cos() * gravity * unit, ginc.sin() * gravity * unit];
    let mut pieces: Vec<Piece> = polys
        .par_iter()
        .zip(keys.par_iter())
        .flat_map_iter(|(lp, &key)| {
            let mut out: Vec<Piece> = vec![];
            let (mnx, mxx) = (lp.iter().map(|v| v[0]).fold(f64::INFINITY, f64::min), lp.iter().map(|v| v[0]).fold(f64::NEG_INFINITY, f64::max));
            let (mny, mxy) = (lp.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min), lp.iter().map(|v| v[1]).fold(f64::NEG_INFINITY, f64::max));
            if mxx < 0.0 || mxy < 0.0 || mnx > lw || mny > lh || key == u32::MAX {
                return out.into_iter();
            }
            let pc_c = centre(lp);
            let (gsum, gn) = groups[&key];
            let gc = [gsum[0] / gn, gsum[1] / gn];
            let i = key;
            // Broken when inside a force sphere (centred `depth` in front of the layer).
            let mut v = [0.0f64; 3];
            let mut broken = false;
            for (fp, fd, fr, fs) in forces {
                let rad = fr * lw;
                if rad <= 0.0 {
                    continue;
                }
                let dv = [gc[0] - fp[0], gc[1] - fp[1], fd * lw];
                let dist = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2]).sqrt();
                if dist < rad {
                    broken = true;
                    let f = fs * (1.0 - dist / rad) * unit / dist.max(1e-6);
                    for q in 0..3 {
                        v[q] += dv[q] * f;
                    }
                }
            }
            if broken && let Some(g) = &gradient {
                let (c, _) = unpremul(g.sample_bilinear(gc[0] * s + b.offset[0], gc[1] * s + b.offset[1]));
                let mut l = effectcraft_color::luminance(c[0], c[1], c[2]).clamp(0.0, 1.0) as f64;
                if invert_gradient {
                    l = 1.0 - l;
                }
                broken = threshold > 0.0 && l >= 1.0 - threshold;
            }
            if broken && custom && white_fixed && key == 0xfff {
                broken = false;
            }
            if (broken && render == 1) || (!broken && render == 2) {
                return out.into_iter();
            }
            let mut gpos = [gc[0], gc[1], 0.0];
            let mut r = rot_axis([1.0, 0.0, 0.0], 0.0);
            let moving = broken && t > 0.0;
            if moving {
                let mass = 1.0 + mass_var * hs(i, 1, seed) as f64;
                for q in 0..3 {
                    v[q] = v[q] / mass + randomness * unit * hs(i, 2 + q as u32, seed) as f64 * 2.0;
                }
                let (f1, f2) = drag_factors(k, t);
                for q in 0..3 {
                    gpos[q] += v[q] * f1 + g3[q] * f2;
                }
                let axis = match tumble {
                    1 => [0.0, 0.0, 0.0],
                    2 => [1.0, 0.0, 0.0],
                    3 => [0.0, 1.0, 0.0],
                    4 => [0.0, 0.0, 1.0],
                    5 => [hs(i, 6, seed) as f64, hs(i, 7, seed) as f64, 0.0],
                    6 => [hs(i, 6, seed) as f64, 0.0, hs(i, 8, seed) as f64],
                    7 => [0.0, hs(i, 7, seed) as f64, hs(i, 8, seed) as f64],
                    _ => [hs(i, 6, seed) as f64, hs(i, 7, seed) as f64, hs(i, 8, seed) as f64],
                };
                let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() / unit;
                let omega = rot_speed * std::f64::consts::TAU * (0.5 + h(i, 9, seed) as f64) * speed.min(10.0);
                r = rot_axis(axis, omega * f1);
            }
            // The piece's centre: its offset from the group centre, rotated with the group.
            let off = [pc_c[0] - gc[0], pc_c[1] - gc[1], 0.0];
            let pos: [f64; 3] = std::array::from_fn(|q| gpos[q] + (0..3).map(|k| r[q][k] * off[k]).sum::<f64>());
            let tex: Vec<[f32; 2]> = lp.iter().map(|v| [(v[0] * s + b.offset[0]) as f32, (v[1] * s + b.offset[1]) as f32]).collect();
            let c0 = [(pc_c[0] * s + b.offset[0]) as f32, (pc_c[1] * s + b.offset[1]) as f32];
            let posb = [pos[0] * s + b.offset[0], pos[1] * s + b.offset[1], pos[2] * s];
            let Some(mut pc) = Piece::new(&tex, c0, &r, posb, cam) else { return out.into_iter() };
            pc.light(&light, &r, posb, eye);
            pc.alpha = tex_opacity;
            let face = if pc.back { &backf } else { &front };
            if face.mode == 0 {
                pc.flat = Some([tex_color[0], tex_color[1], tex_color[2]]);
            } else if face.mode == 2 {
                pc.tint = std::array::from_fn(|q| pc.tint[q] * tex_color[q]);
            }
            // Extrusion: the sides of a moving piece, from its front outline back by the depth.
            if moving && extrude > 0.0 {
                let nb = [r[0][2], r[1][2], r[2][2]];
                let n = tex.len();
                for e in 0..n {
                    let (a, bv) = (tex[e], tex[(e + 1) % n]);
                    let ed = [(bv[0] - a[0]) as f64, (bv[1] - a[1]) as f64];
                    let len = (ed[0] * ed[0] + ed[1] * ed[1]).sqrt();
                    if len < 1e-6 {
                        continue;
                    }
                    let ew: [f64; 3] = std::array::from_fn(|q| (r[q][0] * ed[0] + r[q][1] * ed[1]) / len);
                    let third = [ew[1] * nb[2] - ew[2] * nb[1], ew[2] * nb[0] - ew[0] * nb[2], ew[0] * nb[1] - ew[1] * nb[0]];
                    let rs: M3 = std::array::from_fn(|q| [ew[q], nb[q], third[q]]);
                    // Side centre in world: edge midpoint (world) plus half the depth backwards.
                    let mid = [((a[0] + bv[0]) * 0.5 - c0[0]) as f64, ((a[1] + bv[1]) * 0.5 - c0[1]) as f64];
                    let mw: [f64; 3] = std::array::from_fn(|q| posb[q] + r[q][0] * mid[0] + r[q][1] * mid[1] + nb[q] * extrude * 0.5);
                    let (hl, hd) = ((len * 0.5) as f32, (extrude * 0.5) as f32);
                    let cs = [(a[0] + bv[0]) * 0.5, (a[1] + bv[1]) * 0.5];
                    let quad = [[cs[0] - hl, cs[1] - hd], [cs[0] + hl, cs[1] - hd], [cs[0] + hl, cs[1] + hd], [cs[0] - hl, cs[1] + hd]];
                    if let Some(mut sp) = Piece::new(&quad, cs, &rs, mw, cam) {
                        sp.light(&light, &rs, mw, eye);
                        sp.alpha = tex_opacity;
                        let colour = match (&side.img, side.mode) {
                            (Some(img), m) if m != 0 => {
                                let (c, _) = unpremul(img.sample_bilinear(cs[0] as f64, cs[1] as f64));
                                if m == 2 { std::array::from_fn(|q| c[q] * tex_color[q]) } else { c }
                            }
                            _ => [tex_color[0], tex_color[1], tex_color[2]],
                        };
                        sp.flat = Some(colour);
                        out.push(sp);
                    }
                }
            }
            out.push(pc);
            out.into_iter()
        })
        .collect();
    sort_far_first(&mut pieces);
    if view == 0 {
        let tex = |i: &Option<Image>| i.clone().map(PieceTex::Image);
        let front_t = tex(&front.img).unwrap_or(PieceTex::Layer);
        let back_t = tex(&backf.img).unwrap_or_else(|| front_t.clone());
        return ShatterPlan::Pieces(PiecePlan { pieces, front: front_t, back: Some(back_t) });
    }
    {
        // Wireframes: piece outlines (front view: at rest; otherwise animated), plus forces.
        let animated = view == 2 || view == 4;
        let mut lines = Vec::new();
        for pc in &pieces {
            let n = pc.n as usize;
            let pts: Vec<[f32; 2]> = if animated {
                let fw = inv3(&pc.inv);
                match fw {
                    Some(m) => pc.poly[..n]
                        .iter()
                        .map(|v| {
                            let (a, bb) = ((v[0] - pc.c0[0]) as f64, (v[1] - pc.c0[1]) as f64);
                            let w = m[2][0] * a + m[2][1] * bb + m[2][2];
                            [((m[0][0] * a + m[0][1] * bb + m[0][2]) / w) as f32, ((m[1][0] * a + m[1][1] * bb + m[1][2]) / w) as f32]
                        })
                        .collect(),
                    None => continue,
                }
            } else {
                pc.poly[..n].to_vec()
            };
            for e in 0..n {
                let (a, c) = (pts[e], pts[(e + 1) % n]);
                lines.push(Sprite::new(a[0], a[1], 0.5, [0.75, 0.85, 1.0, 1.0], Shape::Line { dx: c[0] - a[0], dy: c[1] - a[1] }));
            }
        }
        if view >= 3 {
            for (fp, _, fr, _) in forces {
                let (fx, fy) = b.to_px(fp);
                let rad = fr * lw * s;
                for k in 0..64 {
                    let (a0, a1) = (k as f64 / 64.0 * std::f64::consts::TAU, (k + 1) as f64 / 64.0 * std::f64::consts::TAU);
                    let (x0, y0) = (fx + rad * a0.cos(), fy + rad * a0.sin());
                    let (x1, y1) = (fx + rad * a1.cos(), fy + rad * a1.sin());
                    lines.push(Sprite::new(x0 as f32, y0 as f32, 0.5, [1.0, 0.3, 0.3, 1.0], Shape::Line { dx: (x1 - x0) as f32, dy: (y1 - y0) as f32 }));
                }
            }
        }
        ShatterPlan::Wire(SpritePlan { sprites: lines, tints: vec![], acc: Acc::Over, post: Post::Replace })
    }
}

// ------------------------------------------------------------------ Caustics

/// Sample with repeat mode: 0 Once (transparent outside), 1 Tiled, 2 Reflected.
fn sample_repeat(img: &Image, x: f64, y: f64, mode: u32) -> Px {
    let (w, hh) = (img.width as f64, img.height as f64);
    match mode {
        1 => img.sample_bilinear_clamped(x.rem_euclid(w), y.rem_euclid(hh)),
        2 => {
            let m = |v: f64, n: f64| {
                let r = v.rem_euclid(2.0 * n);
                if r > n { 2.0 * n - r } else { r }
            };
            img.sample_bilinear_clamped(m(x, w), m(y, hh))
        }
        _ => img.sample_bilinear(x, y),
    }
}

/// Everything Caustics' per-pixel pass needs, prepared on the CPU (the other layers it reads
/// and the parameter conversions); the layer's own pixels are not read.
pub struct CausticsSetup {
    /// The Bottom layer fitted to the buffer, `None` = the layer itself.
    pub bottom: Option<Image>,
    /// Gaussian σ (buffer px) blurring the bottom (transparent edges), 0 = none.
    pub bottom_sigma: f64,
    /// The water surface's height field (smoothed luminance), `None` = flat.
    pub height: Option<Plane>,
    /// The Sky layer fitted to the buffer.
    pub sky: Option<Image>,
    pub scaling: f64,
    pub repeat: u32,
    pub wave_h: f64,
    /// Displacement per unit of height gradient.
    pub k: f64,
    /// Layer centre (buffer px).
    pub cx: f64,
    pub cy: f64,
    /// Layer width (buffer px).
    pub lw: f64,
    pub surf: [f32; 4],
    pub surf_op: f32,
    pub cstr: f32,
    pub lc: [f32; 4],
    pub li: f32,
    /// Light position (buffer px) and height (layer widths); `point` = from the position to
    /// each pixel, else one direction from above the layer centre.
    pub lx: f64,
    pub ly: f64,
    pub lheight: f64,
    pub point: bool,
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub sharp: f32,
    pub sky_scaling: f64,
    pub sky_repeat: u32,
    pub sky_int: f32,
    pub convergence: f64,
}

/// Caustics' setup for layer geometry `b` (see [`CausticsSetup`]).
pub fn caustics_setup(ctx: &EffectCtx, b: &Buf) -> CausticsSetup {
    let pr = ctx.params;
    let s = b.scale;
    let bottom = ctx.layer_param("bottom/bottom", true).map(|o| crate::util::fit_layer(ctx, b, &o, pr.e("bottom/bottomSizeDiffers") == 1));
    let blur = pr.f("bottom/blur") * s;
    let wave_h = pr.f("water/waveHeight");
    let smoothing = pr.f("water/smoothing") * s;
    let depth = pr.f("water/waterDepth");
    let ior = pr.f("water/refractiveIndex").max(1.0);
    let lpos = pr.v2("lighting/lightPosition");
    // Height field of the water surface (luminance of the chosen layer; none = flat).
    let height = ctx.layer_param("water/waterSurface", true).map(|o| {
        let img = crate::util::fit_layer(ctx, b, &o, true);
        let pl = Plane::luma(&img);
        if smoothing > 0.05 { gauss_plane(&pl, smoothing * 0.5, smoothing * 0.5) } else { pl }
    });
    let lw = ctx.layer_size[0] * s;
    let k = wave_h * depth * (1.0 - 1.0 / ior) * lw * 2.0;
    let (cx, cy) = (b.offset[0] + ctx.layer_size[0] * 0.5 * s, b.offset[1] + ctx.layer_size[1] * 0.5 * s);
    let (lx, ly) = b.to_px(lpos);
    let (lc, li, lheight) = (pr.color("lighting/lightColor"), pr.f("lighting/lightIntensity") as f32, pr.f("lighting/lightHeight").max(0.01));
    // Light Type: Distant Source (one direction, from the light position above the layer
    // centre), Point Source (from the light position to each pixel) or First Comp Light.
    let light_type = pr.e("lighting/lightType");
    let comp_light = if light_type == 2 { ctx.env.host.and_then(|h| h.comp_scene()).and_then(|sc| sc.light) } else { None };
    let (lc, li, lx, ly, lheight, point) = match (light_type, comp_light) {
        (2, Some(l)) => {
            let c = [l.color[0], l.color[1], l.color[2], 1.0];
            // Comp space: z away from the viewer; the light above the water is at −z.
            let (px, py) = (l.pos[0] * s + b.offset[0], l.pos[1] * s + b.offset[1]);
            (c, li, px, py, (-l.pos[2] / ctx.layer_size[0].max(1.0)).max(0.01), l.kind == 1)
        }
        (2, None) => (lc, 0.0, lx, ly, lheight, false),
        (1, _) => (lc, li, lx, ly, lheight, true),
        _ => (lc, li, lx, ly, lheight, false),
    };
    // Sky: the layer the water surface reflects (Surface Opacity 1 = a mirror of it).
    let sky = ctx.layer_param("sky/sky", true).map(|o| crate::util::fit_layer(ctx, b, &o, pr.e("sky/skySizeDiffers") == 1));
    CausticsSetup {
        bottom,
        bottom_sigma: if blur > 0.05 { blur * 0.5 } else { 0.0 },
        height,
        sky,
        scaling: pr.f("bottom/scaling").max(0.01),
        repeat: pr.e("bottom/repeatMode"),
        wave_h,
        k,
        cx,
        cy,
        lw,
        surf: pr.color("water/surfaceColor"),
        surf_op: pr.f("water/surfaceOpacity") as f32,
        cstr: pr.f("water/causticsStrength") as f32,
        lc,
        li,
        lx,
        ly,
        lheight,
        point,
        ambient: pr.f("lighting/ambientLight") as f32,
        diffuse: pr.f("material/diffuse") as f32,
        specular: pr.f("material/specular") as f32,
        sharp: pr.f("material/highlightSharpness").max(1.0) as f32,
        sky_scaling: pr.get("sky/scaling").map(Value::as_f64).unwrap_or(1.0).max(0.01),
        sky_repeat: pr.e("sky/repeatMode"),
        sky_int: pr.get("sky/intensity").map(Value::as_f64).unwrap_or(0.3) as f32,
        convergence: pr.get("sky/convergence").map(Value::as_f64).unwrap_or(0.5).clamp(0.0, 1.0),
    }
}

fn caustics(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let st = caustics_setup(ctx, &b);
    let mut bottom = st.bottom.clone().unwrap_or_else(|| b.img.clone());
    if st.bottom_sigma > 0.0 {
        bottom = effectcraft_raster::gaussian_blur(&bottom, st.bottom_sigma, st.bottom_sigma, false);
    }
    let CausticsSetup {
        scaling,
        repeat,
        wave_h,
        k,
        cx,
        cy,
        lw,
        surf,
        surf_op,
        cstr,
        lc,
        li,
        lx,
        ly,
        lheight,
        point,
        ambient,
        diffuse,
        specular,
        sharp,
        sky_scaling,
        sky_repeat,
        sky_int,
        convergence,
        ..
    } = st;
    let (height, sky) = (&st.height, &st.sky);
    let (w, hh) = (b.img.width as usize, b.img.height as usize);
    let norm3 = |v: [f64; 3]| {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        [(v[0] / n) as f32, (v[1] / n) as f32, (v[2] / n) as f32]
    };
    let lv_at = |x: f64, y: f64| {
        if point {
            norm3([(lx - x) / lw.max(1.0), (ly - y) / lw.max(1.0), lheight])
        } else {
            norm3([(lx - cx) / lw.max(1.0), (ly - cy) / lw.max(1.0), lheight])
        }
    };
    let grad = |x: usize, y: usize| -> (f64, f64) {
        match height {
            Some(pl) => {
                let gx = (pl.get_clamped(x as i64 + 1, y as i64) - pl.get_clamped(x as i64 - 1, y as i64)) as f64 * 0.5;
                let gy = (pl.get_clamped(x as i64, y as i64 + 1) - pl.get_clamped(x as i64, y as i64 - 1)) as f64 * 0.5;
                (gx, gy)
            }
            None => (0.0, 0.0),
        }
    };
    // Displacement and its Jacobian (caustic focusing) per pixel.
    let disp: Vec<(f64, f64)> = (0..w * hh)
        .into_par_iter()
        .map(|i| {
            let (gx, gy) = grad(i % w, i / w);
            (-gx * k, -gy * k)
        })
        .collect();
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let (dx, dy) = disp[i];
            let d = |xx: usize, yy: usize| disp[yy.min(hh - 1) * w + xx.min(w - 1)];
            let (dxr, _) = d(x + 1, y);
            let (_, dyd) = d(x, y + 1);
            let (dxl, _) = d(x.saturating_sub(1), y);
            let (_, dyu) = d(x, y.saturating_sub(1));
            let jac = (1.0 + (dxr - dxl) * 0.5) * (1.0 + (dyd - dyu) * 0.5);
            let focus = ((1.0 / jac.abs().max(0.2)) as f32 - 1.0).clamp(-1.0, 4.0);
            let sx = cx + (x as f64 + 0.5 + dx - cx) / scaling;
            let sy = cy + (y as f64 + 0.5 + dy - cy) / scaling;
            let p = sample_repeat(&bottom, sx, sy, repeat);
            let (c, a) = unpremul(p);
            let (gx, gy) = grad(x, y);
            let n = {
                let v = [-gx * wave_h * 20.0, -gy * wave_h * 20.0, 1.0];
                let m = (v[0] * v[0] + v[1] * v[1] + 1.0).sqrt();
                [(v[0] / m) as f32, (v[1] / m) as f32, (v[2] / m) as f32]
            };
            let lv = lv_at(x as f64 + 0.5, y as f64 + 0.5);
            let nl = (n[0] * lv[0] + n[1] * lv[1] + n[2] * lv[2]).max(0.0);
            let hv = {
                let v = [lv[0], lv[1], lv[2] + 1.0];
                let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                [v[0] / m, v[1] / m, v[2] / m]
            };
            let spec = (n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2]).max(0.0).powf(sharp) * specular * li;
            let light = ambient + diffuse * li * nl + cstr * focus * li;
            // The surface: its colour lit like the water, or the sky it reflects (sampled along
            // the surface normal; Convergence pulls the reflection towards the pixel itself).
            let surface: [f32; 3] = match &sky {
                Some(sk) => {
                    let reach = (1.0 - convergence) * lw * 0.5;
                    let (rx, ry) = (x as f64 + 0.5 + n[0] as f64 * reach, y as f64 + 0.5 + n[1] as f64 * reach);
                    let (rx, ry) = (cx + (rx - cx) / sky_scaling, cy + (ry - cy) / sky_scaling);
                    let (sc, _) = unpremul(sample_repeat(sk, rx, ry, sky_repeat));
                    std::array::from_fn(|q| sc[q] * sky_int * 3.0 * surf[q])
                }
                None => std::array::from_fn(|q| surf[q] * (ambient + diffuse * li * nl)),
            };
            let cc: [f32; 3] = std::array::from_fn(|q| {
                let base = c[q] * light * lc[q];
                base * (1.0 - surf_op) + surface[q] * surf_op + spec * lc[q]
            });
            let a = (a + surf_op * (1.0 - a)).min(1.0);
            *px = [cc[0] * a, cc[1] * a, cc[2] * a, a];
        }
    });
    b
}

// ------------------------------------------------------------------ Wave World

/// Wave World's simulated surface: an `nx` × `ny` grid of heights `u` and velocities `v`.
#[derive(Clone, Debug)]
pub struct Waves {
    pub nx: usize,
    pub ny: usize,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
}

struct Producer {
    ring: bool,
    pos: [f32; 2],
    len: f32,
    width: f32,
    angle: f32,
    amp: f32,
    freq: f32,
    phase: f32,
}

impl Producer {
    /// Weight of a grid point (layer-fraction coordinates, x in widths, y in widths).
    fn weight(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (x - self.pos[0], y - self.pos[1]);
        let (sa, ca) = self.angle.sin_cos();
        let (u, v) = (dx * ca + dy * sa, -dx * sa + dy * ca);
        let d = if self.ring {
            let (a, bb) = ((self.len * 0.5).max(1e-3), (self.width * 0.5).max(1e-3));
            ((u / a).powi(2) + (v / bb).powi(2)).sqrt()
        } else {
            let l = (self.len * 0.5).max(1e-3);
            let wd = (self.width * 0.5).max(1e-3);
            (u.abs() / l).max(v.abs() / wd)
        };
        (1.0 - d).clamp(0.0, 1.0).min(1.0)
    }
}

static WAVE_CACHE: SimCache<Waves> = SimCache::new(4);

/// What Wave World draws: the Height Map view of the simulated surface (grid heights and,
/// with a ground layer, water depths per grid point), or the wireframe preview's lines.
pub enum WavePlan {
    Height { st: std::sync::Arc<Waves>, depth: Option<Vec<f32>> },
    Wire(crate::sim::SpritePlan),
}

fn wave_world(ctx: &EffectCtx, b: Buf) -> Buf {
    match wave_world_impl(ctx, &b, false) {
        Ok(img) => Buf { img, ..b },
        Err(WavePlan::Wire(plan)) => plan.finish(b),
        Err(WavePlan::Height { .. }) => b,
    }
}

/// Wave World's simulation state for the frame, as a [`WavePlan`] (its pixels are not read).
pub fn wave_world_plan(ctx: &EffectCtx, b: &Buf) -> Option<WavePlan> {
    wave_world_impl(ctx, b, true).err()
}

fn wave_world_impl(ctx: &EffectCtx, b: &Buf, plan_only: bool) -> Result<Image, WavePlan> {
    let pr = ctx.params;
    let [lw, lh] = ctx.layer_size;
    let res = pr.f("simulation/gridResolution").clamp(1.0, 400.0) as usize;
    let res = if pr.b("simulation/gridResDownsamples") { ((res as f64) * b.scale).round().max(1.0) as usize } else { res };
    let nx = res.max(2) + 1;
    let ny = ((res as f64 * lh / lw.max(1.0)).round() as usize).max(2) + 1;
    let speed = pr.f("simulation/waveSpeed").max(0.0) as f32;
    let damping = pr.f("simulation/damping").max(0.0) as f32;
    let reflect = pr.e("simulation/reflectEdges");
    let preroll = pr.f("simulation/preRoll").max(0.0);
    let prods: Vec<Producer> = [1, 2]
        .iter()
        .map(|k| {
            let pos = pr.v2(&format!("producer{k}/producer{k}Position"));
            Producer {
                ring: pr.e(&format!("producer{k}/producer{k}Type")) == 0,
                pos: [(pos[0] / lw.max(1.0)) as f32, (pos[1] / lw.max(1.0)) as f32],
                len: pr.f(&format!("producer{k}/producer{k}Length")) as f32,
                width: pr.f(&format!("producer{k}/producer{k}Width")) as f32,
                angle: (pr.f(&format!("producer{k}/producer{k}Angle")) as f32).to_radians(),
                amp: pr.f(&format!("producer{k}/producer{k}Amplitude")) as f32,
                freq: pr.f(&format!("producer{k}/producer{k}Frequency")) as f32,
                phase: (pr.f(&format!("producer{k}/producer{k}Phase")) as f32).to_radians(),
            }
        })
        .collect();
    let sps = crate::sim::SPS as f32;
    // Wave speed in layer widths per second → cells per second; sub-steps keep it stable.
    let cps = speed * (nx - 1) as f32;
    let sub = ((cps * 2.0 / sps).ceil() as usize).clamp(1, 64);
    let dt = 1.0 / (sps * sub as f32);
    let cell = 1.0 / (nx - 1) as f32;
    let weights: Vec<Vec<f32>> = prods.iter().map(|p| (0..nx * ny).map(|i| p.weight((i % nx) as f32 * cell, (i / nx) as f32 * cell)).collect()).collect();
    // Ground: the ground layer's first frame, its luminance an elevation (× Steepness). Where
    // it rises above Height the grid is dry (a wall the waves reflect from); shallow water
    // slows the waves by Wave Strength.
    let ground = pr.get("ground/ground").and_then(Value::as_layer).and_then(|id| {
        let host = ctx.env.host?;
        host.layer_at(id, ctx.env.comp_time - ctx.time, true)
    });
    let steep = pr.get("ground/steepness").map(Value::as_f64).unwrap_or(0.25) as f32;
    let level = pr.get("ground/height").map(Value::as_f64).unwrap_or(0.25) as f32;
    let wave_strength = pr.f("ground/waveStrength").clamp(0.0, 1.0) as f32;
    // Water depth per grid point (None without ground: deep everywhere).
    let depth: Option<Vec<f32>> = ground.as_ref().map(|lp| {
        let (gw, gh) = (lp.size[0].max(1.0), lp.size[1].max(1.0));
        (0..nx * ny)
            .map(|i| {
                let (fx, fy) = ((i % nx) as f64 / (nx - 1) as f64, (i / nx) as f64 / (ny - 1) as f64);
                let (c, a) = unpremul(lp.buf.img.sample_bilinear_clamped(fx * gw * lp.buf.scale + lp.buf.offset[0], fy * gh * lp.buf.scale + lp.buf.offset[1]));
                level - steep * effectcraft_color::luminance(c[0], c[1], c[2]) * a
            })
            .collect()
    });
    let speed_k: Vec<f32> = match &depth {
        Some(d) => d.iter().map(|&v| if v <= 0.0 { 0.0 } else { 1.0 - wave_strength * (1.0 - (v / level.max(1e-3)).clamp(0.0, 1.0).sqrt()) }).collect(),
        None => vec![1.0; nx * ny],
    };
    let mut key =
        params_key(ctx, &Buf { img: Image::new(0, 0), offset: [0.0; 2], scale: if pr.b("simulation/gridResDownsamples") { b.scale } else { 1.0 } }, 4);
    if let Some(d) = &depth {
        key ^= d.iter().fold(0u64, |a, v| a.rotate_left(3) ^ v.to_bits() as u64);
    }
    let steps = crate::sim::steps_at(ctx.time + preroll);
    let st = WAVE_CACHE.run(
        key,
        steps,
        || Waves { nx, ny, u: vec![0.0; nx * ny], v: vec![0.0; nx * ny] },
        |w, step| {
            for k in 0..sub {
                let t = (step as f32 + k as f32 / sub as f32) / sps - preroll as f32;
                let c2 = cps * cps;
                let mut acc = vec![0.0f32; nx * ny];
                for y in 0..ny {
                    for x in 0..nx {
                        let i = y * nx + x;
                        let g = |xx: usize, yy: usize| w.u[yy * nx + xx];
                        let l = g(x.saturating_sub(1), y) + g((x + 1).min(nx - 1), y) + g(x, y.saturating_sub(1)) + g(x, (y + 1).min(ny - 1)) - 4.0 * w.u[i];
                        acc[i] = c2 * speed_k[i] * speed_k[i] * l - damping * 4.0 * w.v[i];
                    }
                }
                for i in 0..nx * ny {
                    w.v[i] += acc[i] * dt;
                    w.u[i] += w.v[i] * dt;
                }
                // Producers drive the surface.
                for (p, wt) in prods.iter().zip(&weights) {
                    if p.amp == 0.0 {
                        continue;
                    }
                    let target = p.amp * (std::f32::consts::TAU * p.freq * t + p.phase).sin();
                    for i in 0..nx * ny {
                        if wt[i] > 0.0 {
                            w.u[i] += (target - w.u[i]) * wt[i];
                            w.v[i] *= 1.0 - wt[i];
                        }
                    }
                }
                // Dry ground holds the surface flat.
                for i in 0..nx * ny {
                    if speed_k[i] <= 0.0 {
                        w.u[i] = 0.0;
                        w.v[i] = 0.0;
                    }
                }
                // Edges: reflective ones mirror; the others absorb in a thin sponge layer.
                let refl = |side: u32| reflect == 5 || reflect == side;
                let sponge = 4usize.min(nx / 4).max(1);
                for y in 0..ny {
                    for x in 0..nx {
                        let i = y * nx + x;
                        let mut f = 1.0f32;
                        if !refl(1) && x < sponge {
                            f = f.min(x as f32 / sponge as f32);
                        }
                        if !refl(3) && nx - 1 - x < sponge {
                            f = f.min((nx - 1 - x) as f32 / sponge as f32);
                        }
                        if !refl(2) && y < sponge {
                            f = f.min(y as f32 / sponge as f32);
                        }
                        if !refl(4) && ny - 1 - y < sponge {
                            f = f.min((ny - 1 - y) as f32 / sponge as f32);
                        }
                        if f < 1.0 {
                            let k = 0.85 + 0.15 * f;
                            w.u[i] *= k;
                            w.v[i] *= k;
                        }
                    }
                }
            }
        },
    );
    let s = b.scale;
    let (bw, bh) = (b.img.width, b.img.height);
    let sample = |x: f64, y: f64| -> f32 {
        let gx = (x / lw.max(1.0) * (st.nx - 1) as f64).clamp(0.0, (st.nx - 1) as f64);
        let gy = (y / lw.max(1.0) * (st.nx - 1) as f64).clamp(0.0, (st.ny - 1) as f64);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(st.nx - 1), (y0 + 1).min(st.ny - 1));
        let (tx, ty) = ((gx - x0 as f64) as f32, (gy - y0 as f64) as f32);
        let g = |xx: usize, yy: usize| st.u[yy * st.nx + xx];
        let a = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
        let c = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
        a + (c - a) * ty
    };
    if plan_only && pr.e("view") == 1 {
        return Err(WavePlan::Height { st, depth });
    }
    if pr.e("view") == 1 {
        let mut img = Image::new(bw, bh);
        let bright = pr.f("heightMapControls/brightness") as f32;
        let contrast = pr.f("heightMapControls/contrast") as f32;
        let gamma = pr.f("heightMapControls/gamma").max(0.01) as f32;
        let alpha = 1.0 - pr.f("heightMapControls/transparency") as f32;
        let dry_transparent = pr.e("heightMapControls/renderDryAreasAs") == 1;
        let dry_at = |x: f64, y: f64| -> bool {
            let Some(d) = &depth else { return false };
            let gx = (x / lw.max(1.0) * (st.nx - 1) as f64).round().clamp(0.0, (st.nx - 1) as f64) as usize;
            let gy = (y / lw.max(1.0) * (st.nx - 1) as f64).round().clamp(0.0, (st.ny - 1) as f64) as usize;
            d[gy * st.nx + gx] <= 0.0
        };
        img.rows_mut().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let lx = (x as f64 + 0.5 - b.offset[0]) / s;
                let ly = (y as f64 + 0.5 - b.offset[1]) / s;
                let v = (bright + contrast * sample(lx, ly)).clamp(0.0, 1.0).powf(1.0 / gamma);
                let al = if dry_transparent && dry_at(lx, ly) { 0.0 } else { alpha };
                *px = [v * al, v * al, v * al, al];
            }
        });
        Ok(img)
    } else {
        // Wireframe preview: the grid turned by Horizontal Rotation about the vertical axis,
        // tilted by Vertical Rotation, heights scaled by Vertical Scale.
        let hr = pr.f("wireframeControls/horizontalRotation").to_radians();
        let vr = pr.get("wireframeControls/verticalRotation").map(Value::as_f64).unwrap_or(53.0).to_radians();
        let vs = pr.get("wireframeControls/verticalScale").map(Value::as_f64).unwrap_or(0.5);
        let (sh, ch) = hr.sin_cos();
        let (sv, cv) = vr.sin_cos();
        let to = |x: usize, y: usize| -> [f32; 2] {
            // Grid point relative to the layer centre (layer px), height in layer px.
            let gx = x as f64 / (st.nx - 1) as f64 * lw - lw * 0.5;
            let gy = y as f64 / (st.nx - 1) as f64 * lw - (st.ny - 1) as f64 / (st.nx - 1) as f64 * lw * 0.5;
            let hgt = st.u[y * st.nx + x] as f64 * vs * lh * 0.5;
            let (rx, ry) = (gx * ch - gy * sh, gx * sh + gy * ch);
            let (px, py) = b.to_px([lw * 0.5 + rx, lh * 0.5 + ry * cv - hgt * sv]);
            [px as f32, py as f32]
        };
        let mut lines = Vec::new();
        for y in 0..st.ny {
            for x in 0..st.nx {
                let a = to(x, y);
                if x + 1 < st.nx {
                    let c = to(x + 1, y);
                    lines.push(Sprite::new(a[0], a[1], 0.4, [0.4, 0.9, 0.4, 1.0], Shape::Line { dx: c[0] - a[0], dy: c[1] - a[1] }));
                }
                if y + 1 < st.ny {
                    let c = to(x, y + 1);
                    lines.push(Sprite::new(a[0], a[1], 0.4, [0.4, 0.9, 0.4, 1.0], Shape::Line { dx: c[0] - a[0], dy: c[1] - a[1] }));
                }
            }
        }
        let plan = crate::sim::SpritePlan { sprites: lines, tints: vec![], acc: Acc::Over, post: crate::sim::Post::OverBlack };
        if plan_only {
            return Err(WavePlan::Wire(plan));
        }
        Ok(plan.finish(Buf { img: Image::new(bw, bh), offset: b.offset, scale: b.scale }).img)
    }
}

// ------------------------------------------------------------------ Foam

#[derive(Clone, Debug)]
struct FoamBubble {
    p: [f32; 2],
    v: [f32; 2],
    age: f32,
    size: f32,
    /// Spin (radians) for Physical Orientation.
    spin: f32,
    id: u32,
}

#[derive(Clone, Debug, Default)]
struct FoamState {
    b: Vec<FoamBubble>,
    carry: f32,
    next: u32,
}

static FOAM_CACHE: SimCache<FoamState> = SimCache::new(4);

/// Foam steps once per frame at 30 frames per second.
const FOAM_FPS: f64 = 30.0;

/// Foam's Bubble Texture options (the last uses Bubble Texture Layer).
const FOAM_TEXTURES: [&str; 6] = ["Default Bubble", "Spit", "Bubblegum", "Dishwater", "Milky", "User Defined"];

/// Age (in simulation steps) at which bubble `id` pops: its lifespan, shortened at random for
/// frail bubbles (`frailty` 0 = every bubble reaches `lifespan`, 1 = uniformly early).
fn foam_pop_age(id: u32, lifespan: f32, frailty: f32, seed: u32) -> f32 {
    lifespan * (1.0 - frailty * (hs(id, 11, seed) * 0.5 + 0.5))
}

/// Foam: bubbles born in the producer rectangle grow to their size, drift with the wind,
/// turbulence and the Flow Map (bubbles run downhill on its luminance), repel and stick to each
/// other, wobble, and pop at the end of their (Strength-shortened) lifespan, pushing their
/// neighbours away at Pop Velocity. Rendered bubbles use a built-in texture or the Bubble
/// Texture Layer, oriented by Bubble Orientation, and reflect the Environment Map.
fn foam(ctx: &EffectCtx, b: Buf) -> Buf {
    foam_full_plan(ctx, &b).finish(b)
}

/// Foam's bubbles as sprites, `None` when it draws more than sprites (a User Defined bubble
/// texture, an Environment Map reflection or the Draft + Flow Map view's flow map).
pub(crate) fn foam_plan(ctx: &EffectCtx, b: &Buf) -> Option<crate::sim::SpritePlan> {
    let plan = foam_full_plan(ctx, b);
    (plan.user.is_none() && plan.env.is_none() && plan.flow.is_none()).then_some(plan.sprites)
}

/// A bubble disc filled by Foam's User Defined texture (buffer px; `rot` radians).
#[derive(Clone, Copy, Debug)]
pub struct FoamDisc {
    pub x: f64,
    pub y: f64,
    pub r: f32,
    pub rot: f32,
    pub fade: f32,
}

/// Foam's Environment Map reflection: the map fitted to the buffer, Reflection Convergence and
/// Strength, and the bubble discs reflecting it (`rot` and `fade` unused).
pub struct FoamEnv {
    pub img: Image,
    pub conv: f64,
    pub strength: f32,
    pub discs: Vec<FoamDisc>,
}

/// What Foam draws this frame over a transparent frame of the layer's size: the bubble sprites,
/// then the User Defined texture's discs, the Environment Map's reflections and (Draft + Flow
/// Map) the flow map under it all, each [`foam_disc`]-composited in bubble order.
pub struct FoamPlan {
    pub sprites: crate::sim::SpritePlan,
    pub user: Option<(crate::LayerPixels, Vec<FoamDisc>)>,
    pub env: Option<FoamEnv>,
    /// The flow map fitted to the buffer.
    pub flow: Option<Image>,
}

impl FoamPlan {
    pub fn finish(&self, b: Buf) -> Buf {
        let mut out = splat(b.img.width, b.img.height, &self.sprites.sprites, self.sprites.acc);
        // User Defined: the texture layer fills each bubble's disc.
        if let Some((tex, discs)) = &self.user {
            for d in discs {
                let (rot, fade) = (d.rot, d.fade);
                foam_disc(&mut out, d.x, d.y, d.r as f64, |u, v| {
                    // u, v in −1..1 across the bubble, rotated into the texture.
                    let (sr, cr) = rot.sin_cos();
                    let (tu, tv) = (u as f32 * cr + v as f32 * sr, -u as f32 * sr + v as f32 * cr);
                    let (x, y) = ((tu as f64 * 0.5 + 0.5) * tex.size[0], (tv as f64 * 0.5 + 0.5) * tex.size[1]);
                    let px = tex.buf.img.sample_bilinear(x * tex.buf.scale + tex.buf.offset[0], y * tex.buf.scale + tex.buf.offset[1]);
                    px.map(|c| c * fade)
                });
            }
        }
        // Environment Map: each bubble reflects the map along its sphere normal (Reflection
        // Convergence pulls the reflection towards the bubble's own spot of the map).
        if let Some(env) = &self.env {
            let (conv, strength) = (env.conv, env.strength);
            let (w, h) = (b.img.width as f64, b.img.height as f64);
            for d in &env.discs {
                let (bx, by) = (d.x, d.y);
                foam_disc(&mut out, bx, by, d.r as f64, |u, v| {
                    // Sphere normal; the reflection looks across the whole map, or (convergence 1)
                    // just behind the bubble.
                    let (ex, ey) =
                        if conv >= 1.0 { (bx, by) } else { (bx * conv + (u * 0.5 + 0.5) * w * (1.0 - conv), by * conv + (v * 0.5 + 0.5) * h * (1.0 - conv)) };
                    let e = env.img.sample_bilinear(ex, ey);
                    let rim = ((u * u + v * v) as f32).min(1.0);
                    let k = strength * (0.3 + 0.7 * rim);
                    [e[0] * k, e[1] * k, e[2] * k, 0.0]
                });
            }
        }
        // Draft + Flow Map shows the flow map under the bubbles.
        if let Some(fm) = &self.flow {
            out.data.par_iter_mut().zip(fm.data.par_iter()).for_each(|(o, f)| {
                let k = 1.0 - o[3];
                for c in 0..4 {
                    o[c] += f[c] * 0.5 * k;
                }
            });
        }
        Buf { img: out, ..b }
    }
}

/// Foam's [`FoamPlan`] for layer geometry `b` (its size; its pixels are not read).
pub fn foam_full_plan(ctx: &EffectCtx, b: &Buf) -> FoamPlan {
    let pr = ctx.params;
    let (lw, lh) = (ctx.layer_size[0] as f32, ctx.layer_size[1] as f32);
    let unit = lw * 0.01;
    let prod = pr.v2("producer/producerPoint");
    let (psx, psy) = (pr.f("producer/producerXSize") as f32 * lw, pr.f("producer/producerYSize") as f32 * lh);
    let porient = (pr.f("producer/producerOrientation") as f32).to_radians();
    let rate = pr.f("producer/productionRate").max(0.0) as f32;
    let size = pr.f("bubbles/size").max(0.0) as f32;
    let size_var = pr.f("bubbles/sizeVariance") as f32;
    let lifespan = pr.f("bubbles/lifespan").max(1.0) as f32;
    let growth = pr.f("bubbles/growthSpeed").max(0.0001) as f32;
    // Strength: weaker bubbles tend to pop before their lifespan (at 20 or more every bubble
    // lives the full lifespan); each bubble gets its own pop age.
    let frailty = 1.0 - (pr.f("bubbles/strength") as f32 / 20.0).clamp(0.0, 1.0);
    let init_speed = pr.f("physics/initialSpeed") as f32;
    let init_dir = (pr.f("physics/initialDirection") as f32).to_radians();
    let wind = pr.f("physics/windSpeed") as f32;
    let wind_dir = (pr.f("physics/windDirection") as f32).to_radians();
    let turb = pr.f("physics/turbulence") as f32;
    let wobble = pr.f("physics/wobbleAmount").max(0.0) as f32;
    let repulsion = pr.f("physics/repulsion") as f32;
    let pop_velocity = pr.f("physics/popVelocity") as f32;
    let viscosity = pr.f("physics/viscosity").clamp(0.0, 1.0) as f32;
    let sticky = pr.f("physics/stickiness").clamp(0.0, 1.0) as f32;
    let universe = pr.f("universeSize").max(0.01) as f32;
    let seed = (pr.f("randomSeed") as u32).wrapping_mul(0x68e31da4) ^ ctx.seed;
    let wvec = [wind_dir.sin() * wind * unit, -wind_dir.cos() * wind * unit];
    // Flow Map: the map layer's first frame, its luminance a height field (Fits the layer or
    // the whole universe).
    let flow = pr.get("flowMap/flowMap").and_then(Value::as_layer).and_then(|id| {
        let host = ctx.env.host?;
        let me = pr.get(&crate::layer_source_id("flowMap/flowMap")).is_none_or(|v| v.as_enum() != 0);
        host.layer_at(id, ctx.env.comp_time - ctx.time, me)
    });
    let steep = pr.f("flowMap/flowMapSteepness") as f32;
    let fits_universe = pr.e("flowMap/flowMapFits") == 1;
    let substeps = match pr.e("flowMap/simulationQuality") {
        1 => 2,
        2 => 4,
        _ => 1,
    };
    let height = flow.as_ref().map(|lp| {
        let img = &lp.buf.img;
        let pl = Plane::from_image(img, |px| {
            let (c, a) = unpremul(px);
            effectcraft_color::luminance(c[0], c[1], c[2]) * a
        });
        (pl, lp.buf.scale, lp.buf.offset, lp.size)
    });
    let (ux0, uy0) = (-(universe - 1.0) * 0.5 * lw, -(universe - 1.0) * 0.5 * lh);
    let (ux1, uy1) = (lw - ux0, lh - uy0);
    // Height at a layer point (0..1).
    let h_at = |x: f32, y: f32| -> f32 {
        let Some((pl, s, off, size)) = &height else { return 0.0 };
        let (fx, fy) = if fits_universe { ((x - ux0) / (ux1 - ux0), (y - uy0) / (uy1 - uy0)) } else { (x / lw, y / lh) };
        pl.sample(fx as f64 * size[0] * s + off[0], fy as f64 * size[1] * s + off[1])
    };
    let mut key = params_key(ctx, &Buf { img: Image::new(0, 0), offset: [0.0; 2], scale: 1.0 }, 5);
    if let Some((pl, ..)) = &height {
        key ^= pl.data.iter().step_by(5).fold(0u64, |a, v| a.rotate_left(3) ^ v.to_bits() as u64);
    }
    let steps = if ctx.time <= 0.0 { 0 } else { (ctx.time * FOAM_FPS + 1e-6).floor() as u64 };
    let rad = |q: &FoamBubble| q.size * unit * 3.0 * (q.age * growth).min(1.0);
    let st = FOAM_CACHE.run(key, steps, FoamState::default, |st, step| {
        let t = step as f32 / FOAM_FPS as f32;
        // Births.
        st.carry += rate;
        let n = st.carry.floor() as u32;
        st.carry -= n as f32;
        for _ in 0..n {
            if st.b.len() >= 3000 {
                break;
            }
            let id = st.next;
            st.next = st.next.wrapping_add(1);
            let (ux, uy) = (hs(id, 1, seed) * 0.5 * psx, hs(id, 2, seed) * 0.5 * psy);
            let (so, co) = porient.sin_cos();
            let p = [prod[0] as f32 + ux * co - uy * so, prod[1] as f32 + ux * so + uy * co];
            let v = [init_dir.sin() * init_speed * unit, -init_dir.cos() * init_speed * unit];
            let sz = (size * (1.0 + size_var * hs(id, 3, seed))).max(0.01);
            st.b.push(FoamBubble { p, v, age: 0.0, size: sz, spin: hs(id, 12, seed) * std::f32::consts::PI, id });
        }
        for _ in 0..substeps {
            let k = 1.0 / substeps as f32;
            // Forces: drift towards the wind, turbulence, viscosity, the flow map's slope.
            for q in st.b.iter_mut() {
                let n1 = value_noise(q.p[0] / (lw * 0.15), q.p[1] / (lw * 0.15), t * 0.5, seed) - 0.5;
                let n2 = value_noise(q.p[0] / (lw * 0.15) + 19.0, q.p[1] / (lw * 0.15), t * 0.5, seed) - 0.5;
                for c in 0..2 {
                    q.v[c] += (wvec[c] - q.v[c]) * 0.1 * k;
                    q.v[c] *= 1.0 - viscosity * 0.5 * k;
                }
                q.v[0] += n1 * turb * unit * k;
                q.v[1] += n2 * turb * unit * k;
                if height.is_some() && steep != 0.0 {
                    let e = unit.max(1.0);
                    let gx = (h_at(q.p[0] + e, q.p[1]) - h_at(q.p[0] - e, q.p[1])) / (2.0 * e);
                    let gy = (h_at(q.p[0], q.p[1] + e) - h_at(q.p[0], q.p[1] - e)) / (2.0 * e);
                    // Downhill: from light (high) to dark (low).
                    q.v[0] -= gx * steep * lw * 0.5 * k;
                    q.v[1] -= gy * steep * lw * 0.5 * k;
                }
                // Physical Orientation: bubbles roll with their sideways motion.
                q.spin += q.v[0] / (rad(q).max(0.5)) * k;
            }
            // Repulsion between overlapping bubbles (grid hashed, deterministic order).
            if repulsion > 0.0 && st.b.len() > 1 {
                let maxr = st.b.iter().map(rad).fold(0.0f32, f32::max).max(1.0);
                let cs = maxr * 2.0;
                let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
                for (i, q) in st.b.iter().enumerate() {
                    grid.entry(((q.p[0] / cs).floor() as i32, (q.p[1] / cs).floor() as i32)).or_default().push(i);
                }
                let mut push = vec![[0.0f32; 2]; st.b.len()];
                for (i, q) in st.b.iter().enumerate() {
                    let (gx, gy) = ((q.p[0] / cs).floor() as i32, (q.p[1] / cs).floor() as i32);
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let Some(list) = grid.get(&(gx + dx, gy + dy)) else { continue };
                            for &j in list {
                                if j <= i {
                                    continue;
                                }
                                let o = &st.b[j];
                                let (ddx, ddy) = (o.p[0] - q.p[0], o.p[1] - q.p[1]);
                                let d = (ddx * ddx + ddy * ddy).sqrt().max(1e-4);
                                let overlap = rad(q) + rad(o) - d;
                                if overlap > 0.0 {
                                    let f = overlap * 0.5 * repulsion.min(4.0) * (1.0 - sticky * 0.5) / d * k;
                                    push[i][0] -= ddx * f;
                                    push[i][1] -= ddy * f;
                                    push[j][0] += ddx * f;
                                    push[j][1] += ddy * f;
                                }
                            }
                        }
                    }
                }
                for (q, d) in st.b.iter_mut().zip(&push) {
                    q.p[0] += d[0].clamp(-maxr, maxr);
                    q.p[1] += d[1].clamp(-maxr, maxr);
                }
            }
            for q in st.b.iter_mut() {
                q.p[0] += q.v[0] * k;
                q.p[1] += q.v[1] * k;
            }
        }
        for q in st.b.iter_mut() {
            q.age += 1.0;
        }
        // Pops: bubbles past their pop age burst, pushing their neighbours away.
        let popped: Vec<([f32; 2], f32)> = st.b.iter().filter(|q| q.age >= foam_pop_age(q.id, lifespan, frailty, seed)).map(|q| (q.p, rad(q))).collect();
        if pop_velocity != 0.0 {
            for (pp, pr_) in &popped {
                let reach = (pr_ * 3.0).max(unit);
                for q in st.b.iter_mut() {
                    let (dx, dy) = (q.p[0] - pp[0], q.p[1] - pp[1]);
                    let d = (dx * dx + dy * dy).sqrt();
                    if d > 1e-4 && d < reach {
                        let f = pop_velocity * unit * (1.0 - d / reach) / d;
                        q.v[0] += dx * f;
                        q.v[1] += dy * f;
                    }
                }
            }
        }
        st.b.retain(|q| q.age < foam_pop_age(q.id, lifespan, frailty, seed) && q.p[0] > ux0 && q.p[0] < ux1 && q.p[1] > uy0 && q.p[1] < uy1);
    });
    let view = pr.e("view");
    let zoom = pr.f("zoom").max(0.01) as f32;
    let zoom_producer = pr.b("producer/zoomProducerPoint");
    let texture = pr.e("rendering/bubbleTexture");
    let orient = pr.e("rendering/bubbleOrientation");
    let blend = pr.e("rendering/blendMode");
    let s = b.scale as f32;
    // Zoom centres on the layer, or on the producer with Zoom Producer Point.
    let (cx, cy) = if zoom_producer { (prod[0] as f32, prod[1] as f32) } else { (lw * 0.5, lh * 0.5) };
    let mut order: Vec<&FoamBubble> = st.b.iter().collect();
    if blend == 2 {
        // Solid New on Top: oldest first.
        order.sort_by(|a, c| c.age.total_cmp(&a.age).then(a.id.cmp(&c.id)));
    } else if blend == 1 {
        order.sort_by(|a, c| a.age.total_cmp(&c.age).then(a.id.cmp(&c.id)));
    }
    // Where a bubble is drawn: buffer centre, radius, rotation (radians).
    let place = |q: &FoamBubble| {
        let wob = if wobble > 0.0 { 1.0 + wobble * 0.15 * ((q.age * 0.35) + hs(q.id, 13, seed) * 6.0).sin() } else { 1.0 };
        let r = rad(q) * wob * zoom * s;
        let (bx, by) = b.to_px([(cx + (q.p[0] - cx) * zoom) as f64, (cy + (q.p[1] - cy) * zoom) as f64]);
        let rot = match orient {
            1 => q.spin,
            2 => q.v[1].atan2(q.v[0]),
            _ => 0.0,
        };
        (bx, by, r, rot)
    };
    let user_tex = (view == 2 && texture == 5).then(|| ctx.layer_param("rendering/bubbleTextureLayer", true)).flatten();
    let sprites: Vec<Sprite> = order
        .iter()
        .filter(|_| user_tex.is_none())
        .map(|q| {
            let (bx, by, r, rot) = place(q);
            let fade = ((foam_pop_age(q.id, lifespan, frailty, seed) - q.age) / 5.0).clamp(0.0, 1.0);
            // Draft and Draft + Flow Map show the wireframe-style bubbles.
            let mut sp = if view != 2 {
                Sprite::new(bx as f32, by as f32, r, [0.55, 0.75, 1.0, fade], Shape::Bubble)
            } else {
                let (c, shape) = match texture {
                    1 => ([0.95, 0.95, 0.92], Shape::Faded),
                    2 => ([1.0, 0.6, 0.8], Shape::Bubble),
                    3 => ([0.8, 0.85, 0.75], Shape::Bubble),
                    4 => ([0.97, 0.97, 0.95], Shape::Sphere),
                    _ => ([1.0, 1.0, 1.0], Shape::Bubble),
                };
                Sprite::new(bx as f32, by as f32, r, [c[0], c[1], c[2], fade], shape)
            };
            sp.rot = rot;
            sp
        })
        .collect();
    let acc = if blend == 0 && view == 2 { Acc::Add } else { Acc::Over };
    let disc = |q: &&FoamBubble| {
        let (x, y, r, rot) = place(q);
        let fade = ((foam_pop_age(q.id, lifespan, frailty, seed) - q.age) / 5.0).clamp(0.0, 1.0);
        FoamDisc { x, y, r, rot, fade }
    };
    let user = user_tex.map(|tex| (tex, order.iter().map(disc).collect()));
    let strength = pr.f("rendering/reflectionStrength").clamp(0.0, 1.0) as f32;
    let env = if view == 2
        && strength > 0.0
        && let Some(env) = ctx.layer_param("rendering/environmentMap", true)
    {
        Some(FoamEnv {
            img: crate::util::fit_layer(ctx, b, &env, true),
            conv: pr.f("rendering/reflectionConvergence").clamp(0.0, 1.0),
            strength,
            discs: order.iter().map(disc).collect(),
        })
    } else {
        None
    };
    let flow = if view == 1
        && let Some(lp) = &flow
    {
        Some(crate::util::fit_layer(ctx, b, lp, true))
    } else {
        None
    };
    FoamPlan { sprites: crate::sim::SpritePlan { sprites, tints: vec![], acc, post: crate::sim::Post::Replace }, user, env, flow }
}

/// Add `f(u, v)` (premultiplied; u, v ∈ −1..1 across the disc) over the disc of radius `r` at
/// (`x`, `y`) with anti-aliased edges ("over" for opaque texels, additive for alpha 0).
fn foam_disc(out: &mut Image, x: f64, y: f64, r: f64, f: impl Fn(f64, f64) -> Px) {
    if r <= 0.0 {
        return;
    }
    let (x0, x1) = ((x - r - 1.0).floor().max(0.0) as i64, (x + r + 1.0).ceil().min(out.width as f64) as i64);
    let (y0, y1) = ((y - r - 1.0).floor().max(0.0) as i64, (y + r + 1.0).ceil().min(out.height as f64) as i64);
    for py in y0..y1 {
        for px in x0..x1 {
            let (u, v) = ((px as f64 + 0.5 - x) / r, (py as f64 + 0.5 - y) / r);
            let d = (u * u + v * v).sqrt();
            let cov = ((1.0 - d) * r + 0.5).clamp(0.0, 1.0) as f32;
            if cov <= 0.0 {
                continue;
            }
            let s = f(u.clamp(-1.0, 1.0), v.clamp(-1.0, 1.0)).map(|c| c * cov);
            let o = out.get(px, py);
            let k = 1.0 - s[3];
            out.set(px as u32, py as u32, [s[0] + o[0] * k, s[1] + o[1] * k, s[2] + o[2] * k, (s[3] + o[3] * k).min(1.0)]);
        }
    }
}

// ------------------------------------------------------------------ specs

fn producer_params(k: u32, amp: f64, pos: (f64, f64)) -> Vec<crate::ParamSpec> {
    let ids: [&'static str; 8] = match k {
        1 => [
            "producer1/producer1Type",
            "producer1/producer1Position",
            "producer1/producer1Length",
            "producer1/producer1Width",
            "producer1/producer1Angle",
            "producer1/producer1Amplitude",
            "producer1/producer1Frequency",
            "producer1/producer1Phase",
        ],
        _ => [
            "producer2/producer2Type",
            "producer2/producer2Position",
            "producer2/producer2Length",
            "producer2/producer2Width",
            "producer2/producer2Angle",
            "producer2/producer2Amplitude",
            "producer2/producer2Frequency",
            "producer2/producer2Phase",
        ],
    };
    vec![
        p(ids[0], "Type", Value::Enum(0), popup(&["Ring", "Line"])),
        p(ids[1], "Position", pt(pos.0, pos.1), ParamUi::Point),
        p(ids[2], "Height/Length", num(0.1), slider(0.0, 2.0, 0.0, 1.0, 3)),
        p(ids[3], "Width", num(0.1), slider(0.0, 2.0, 0.0, 1.0, 3)),
        p(ids[4], "Angle", num(0.0), ParamUi::Angle),
        p(ids[5], "Amplitude", num(amp), slider(-5.0, 5.0, -1.0, 1.0, 3)),
        p(ids[6], "Frequency", num(1.0), slider(0.0, 30.0, 0.0, 5.0, 3)),
        p(ids[7], "Phase", num(0.0), ParamUi::Angle),
    ]
}

fn card_dance_params() -> Vec<crate::ParamSpec> {
    let mut v = vec![
        p("rowsColumns", "Rows & Columns", Value::Enum(0), popup(&["Independent", "Columns Follows Rows"])),
        p("rows", "Rows", num(10.0), slider(1.0, 1000.0, 1.0, 100.0, 0)),
        p("columns", "Columns", num(10.0), slider(1.0, 1000.0, 1.0, 100.0, 0)),
        p("backLayer", "Back Layer", Value::Layer(None), ParamUi::Layer),
        p("gradientLayer1", "Gradient Layer 1", Value::Layer(None), ParamUi::Layer),
        p("gradientLayer2", "Gradient Layer 2", Value::Layer(None), ParamUi::Layer),
    ];
    let names: [[&'static str; 4]; 8] = [
        ["xPosition/xPosSource", "xPosition/xPosMultiplier", "xPosition/xPosOffset", "X Position"],
        ["yPosition/yPosSource", "yPosition/yPosMultiplier", "yPosition/yPosOffset", "Y Position"],
        ["zPosition/zPosSource", "zPosition/zPosMultiplier", "zPosition/zPosOffset", "Z Position"],
        ["xRotation/xRotSource", "xRotation/xRotMultiplier", "xRotation/xRotOffset", "X Rotation"],
        ["yRotation/yRotSource", "yRotation/yRotMultiplier", "yRotation/yRotOffset", "Y Rotation"],
        ["zRotation/zRotSource", "zRotation/zRotMultiplier", "zRotation/zRotOffset", "Z Rotation"],
        ["xScale/xScaleSource", "xScale/xScaleMultiplier", "xScale/xScaleOffset", "X Scale"],
        ["yScale/yScaleSource", "yScale/yScaleMultiplier", "yScale/yScaleOffset", "Y Scale"],
    ];
    debug_assert_eq!(names.len(), CD_PROPS.len());
    for (i, n) in names.iter().enumerate() {
        let rot = (3..6).contains(&i);
        let scale = i >= 6;
        let (lo, hi) = if rot { (-3600.0, 3600.0) } else { (-100.0, 100.0) };
        v.push(p(n[0], "Source", Value::Enum(0), popup(CD_SOURCES)));
        v.push(p(n[1], "Multiplier", num(0.0), slider(lo, hi, if rot { -360.0 } else { -10.0 }, if rot { 360.0 } else { 10.0 }, 2)));
        v.push(p(n[2], "Offset", num(if scale { 1.0 } else { 0.0 }), slider(lo, hi, if rot { -360.0 } else { -10.0 }, if rot { 360.0 } else { 10.0 }, 2)));
    }
    v.extend(crate::card3d::camera_params(2.0));
    v.extend(crate::card3d::lighting_params(1.0, 0.25));
    v.extend(crate::card3d::material_params(0.75));
    v
}

pub fn specs() -> Vec<EffectSpec> {
    vec![
        spec(
            "ec.sim.ccballaction",
            "CC Ball Action",
            vec![
                p("scatter", "Scatter", num(0.0), slider(0.0, 2000.0, 0.0, 300.0, 1)),
                p("rotationAxis", "Rotation Axis", Value::Enum(0), popup(&["X Axis", "Y Axis", "Z Axis", "XY Axis", "XZ Axis", "YZ Axis", "XYZ Axis"])),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("twistProperty", "Twist Property", Value::Enum(0), popup(&["X Axis", "Y Axis", "Radius", "Brightness"])),
                p("twistAngle", "Twist Angle", num(0.0), ParamUi::Angle),
                p("gridSpacing", "Grid Spacing", num(4.0), slider(1.0, 200.0, 1.0, 50.0, 0)),
                p("ballSize", "Ball Size", num(40.0), slider(0.0, 400.0, 0.0, 200.0, 1)),
                p("instabilityState", "Instability State", num(0.0), ParamUi::Angle),
            ],
            ball_action,
        ),
        spec(
            "ec.sim.ccpixelpolly",
            "CC Pixel Polly",
            vec![
                p("force", "Force", num(100.0), slider(-1000.0, 1000.0, -300.0, 300.0, 1)),
                p("gravity", "Gravity", num(1.0), slider(-20.0, 20.0, -5.0, 5.0, 2)),
                p("spinning", "Spinning", num(0.0), ParamUi::Angle),
                p("forceCenter", "Force Center", pt(0.5, 0.5), ParamUi::Point),
                p("directionRandomness", "Direction Randomness", num(10.0), pct()),
                p("speedRandomness", "Speed Randomness", num(10.0), pct()),
                p("gridSpacing", "Grid Spacing", num(10.0), slider(1.0, 500.0, 2.0, 100.0, 0)),
                p("object", "Object", Value::Enum(0), popup(&["Polygon", "Textured Polygon", "Square", "Textured Square"])),
                p("enableDepthSort", "Enable Depth Sort", Value::Bool(true), ParamUi::Checkbox),
                p("startTime", "Start Time (sec)", num(0.0), slider(-1000.0, 1000.0, 0.0, 10.0, 2)),
            ],
            pixel_polly,
        ),
        spec(
            "ec.sim.ccscatterize",
            "CC Scatterize",
            vec![
                p("scatter", "Scatter", num(0.0), slider(0.0, 1000.0, 0.0, 200.0, 1)),
                p("rightTwist", "Right Twist", num(0.0), ParamUi::Angle),
                p("leftTwist", "Left Twist", num(0.0), ParamUi::Angle),
                p("transferMode", "Transfer Mode", Value::Enum(0), popup(&["Composite", "Add"])),
            ],
            scatterize,
        ),
        spec("ec.sim.carddance", "Card Dance", card_dance_params(), card_dance),
        spec(
            "ec.sim.caustics",
            "Caustics",
            vec![
                p("bottom/bottom", "Bottom", Value::Layer(None), ParamUi::Layer),
                p("bottom/scaling", "Scaling", num(1.0), slider(0.01, 10.0, 0.1, 3.0, 3)),
                p("bottom/repeatMode", "Repeat Mode", Value::Enum(2), popup(&["Once", "Tiled", "Reflected"])),
                p("bottom/bottomSizeDiffers", "If Layer Size Differs", Value::Enum(1), popup(&["Center", "Stretch to Fit"])),
                p("bottom/blur", "Blur", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
                p("water/waterSurface", "Water Surface", Value::Layer(None), ParamUi::Layer),
                p("water/waveHeight", "Wave Height", num(0.2), slider(-1.0, 1.0, -1.0, 1.0, 3)),
                p("water/smoothing", "Smoothing", num(5.0), slider(0.0, 100.0, 0.0, 50.0, 1)),
                p("water/waterDepth", "Water Depth", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("water/refractiveIndex", "Refractive Index", num(1.2), slider(1.0, 3.0, 1.0, 2.0, 3)),
                p("water/surfaceColor", "Surface Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("water/surfaceOpacity", "Surface Opacity", num(0.3), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("water/causticsStrength", "Caustics Strength", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("sky/sky", "Sky", Value::Layer(None), ParamUi::Layer),
                p("sky/scaling", "Scaling", num(1.0), slider(0.01, 10.0, 0.1, 3.0, 3)),
                p("sky/repeatMode", "Repeat Mode", Value::Enum(2), popup(&["Once", "Tiled", "Reflected"])),
                p("sky/skySizeDiffers", "If Layer Size Differs", Value::Enum(1), popup(&["Center", "Stretch to Fit"])),
                p("sky/intensity", "Intensity", num(0.3), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("sky/convergence", "Convergence", num(0.5), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("lighting/lightType", "Light Type", Value::Enum(0), popup(&crate::card3d::LIGHT_TYPES)),
                p("lighting/lightIntensity", "Light Intensity", num(1.0), slider(0.0, 4.0, 0.0, 2.0, 2)),
                p("lighting/lightColor", "Light Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("lighting/lightPosition", "Light Position", pt(0.0, 0.0), ParamUi::Point),
                p("lighting/lightHeight", "Light Height", num(1.0), slider(0.0, 10.0, 0.0, 4.0, 2)),
                p("lighting/ambientLight", "Ambient Light", num(0.35), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("material/diffuse", "Diffuse Reflection", num(0.75), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("material/specular", "Specular Reflection", num(0.2), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("material/highlightSharpness", "Highlight Sharpness", num(15.0), slider(1.0, 100.0, 1.0, 100.0, 1)),
            ],
            caustics,
        ),
        spec(
            "ec.sim.waveworld",
            "Wave World",
            {
                let mut v = vec![
                    p("view", "View", Value::Enum(0), popup(&["Wireframe Preview", "Height Map"])),
                    p("wireframeControls/horizontalRotation", "Horizontal Rotation", num(0.0), ParamUi::Angle),
                    p("wireframeControls/verticalRotation", "Vertical Rotation", num(53.0), slider(0.0, 90.0, 0.0, 90.0, 1)),
                    p("wireframeControls/verticalScale", "Vertical Scale", num(0.5), slider(0.0, 10.0, 0.0, 2.0, 3)),
                    p("heightMapControls/brightness", "Brightness", num(0.5), slider(-1.0, 2.0, 0.0, 1.0, 3)),
                    p("heightMapControls/contrast", "Contrast", num(0.75), slider(0.0, 4.0, 0.0, 2.0, 3)),
                    p("heightMapControls/gamma", "Gamma Adjustment", num(1.0), slider(0.01, 10.0, 0.2, 5.0, 3)),
                    p("heightMapControls/renderDryAreasAs", "Render Dry Areas As", Value::Enum(0), popup(&["Solid", "Transparent"])),
                    p("heightMapControls/transparency", "Transparency", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                    p("simulation/gridResolution", "Grid Resolution", num(60.0), slider(1.0, 400.0, 1.0, 200.0, 0)),
                    p("simulation/gridResDownsamples", "Grid Res Downsamples", Value::Bool(false), ParamUi::Checkbox),
                    p("simulation/waveSpeed", "Wave Speed", num(0.2), slider(0.0, 5.0, 0.0, 1.0, 3)),
                    p("simulation/damping", "Damping", num(0.05), slider(0.0, 5.0, 0.0, 1.0, 3)),
                    p("simulation/reflectEdges", "Reflect Edges", Value::Enum(0), popup(&["None", "Left", "Top", "Right", "Bottom", "All"])),
                    p("simulation/preRoll", "Pre-roll (seconds)", num(0.0), slider(0.0, 30.0, 0.0, 10.0, 2)),
                    p("ground/ground", "Ground", Value::Layer(None), ParamUi::Layer),
                    p("ground/steepness", "Steepness", num(0.25), slider(0.0, 2.0, 0.0, 1.0, 3)),
                    p("ground/height", "Height", num(0.25), slider(0.0, 2.0, 0.0, 1.0, 3)),
                    p("ground/waveStrength", "Wave Strength", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                ];
                v.extend(producer_params(1, 0.5, (0.5, 0.5)));
                v.extend(producer_params(2, 0.0, (0.25, 0.25)));
                v
            },
            wave_world,
        ),
        spec(
            "ec.sim.shatter",
            "Shatter",
            vec![
                p(
                    "view",
                    "View",
                    Value::Enum(1),
                    popup(&["Rendered", "Wireframe Front View", "Wireframe", "Wireframe Front View + Forces", "Wireframe + Forces"]),
                ),
                p("render", "Render", Value::Enum(0), popup(&["All", "Layer", "Pieces"])),
                p("shape/pattern", "Pattern", Value::Enum(0), popup(SHATTER_PATTERNS)),
                p("shape/repetitions", "Repetitions", num(10.0), slider(1.0, 500.0, 1.0, 100.0, 2)),
                p("shape/direction", "Direction", num(0.0), ParamUi::Angle),
                p("shape/origin", "Origin", pt(0.5, 0.5), ParamUi::Point),
                p("shape/customShatterMap", "Custom Shatter Map", Value::Layer(None), ParamUi::Layer),
                p("shape/whiteTilesFixed", "White Tiles Fixed", Value::Bool(false), ParamUi::Checkbox),
                p("shape/extrusionDepth", "Extrusion Depth", num(0.05), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("force1/force1Position", "Position", pt(0.5, 0.5), ParamUi::Point),
                p("force1/force1Depth", "Depth", num(0.1), slider(-1.0, 1.0, -1.0, 1.0, 3)),
                p("force1/force1Radius", "Radius", num(0.4), slider(0.0, 4.0, 0.0, 2.0, 3)),
                p("force1/force1Strength", "Strength", num(5.0), slider(-20.0, 20.0, -10.0, 10.0, 2)),
                p("force2/force2Position", "Position", pt(0.25, 0.25), ParamUi::Point),
                p("force2/force2Depth", "Depth", num(0.1), slider(-1.0, 1.0, -1.0, 1.0, 3)),
                p("force2/force2Radius", "Radius", num(0.0), slider(0.0, 4.0, 0.0, 2.0, 3)),
                p("force2/force2Strength", "Strength", num(5.0), slider(-20.0, 20.0, -10.0, 10.0, 2)),
                p("physics/rotationSpeed", "Rotation Speed", num(0.2), slider(0.0, 5.0, 0.0, 1.0, 3)),
                p("physics/tumbleAxis", "Tumble Axis", Value::Enum(0), popup(&["Free", "None", "X", "Y", "Z", "XY", "XZ", "YZ"])),
                p("physics/randomness", "Randomness", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("physics/viscosity", "Viscosity", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("physics/massVariance", "Mass Variance", num(30.0), pct()),
                p("physics/gravity", "Gravity", num(3.0), slider(0.0, 100.0, 0.0, 10.0, 2)),
                p("physics/gravityDirection", "Gravity Direction", num(180.0), ParamUi::Angle),
                p("physics/gravityInclination", "Gravity Inclination", num(0.0), slider(-90.0, 90.0, -90.0, 90.0, 1)),
                p("gradient/shatterThreshold", "Shatter Threshold", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("gradient/gradientLayer", "Gradient Layer", Value::Layer(None), ParamUi::Layer),
                p("gradient/invertGradient", "Invert Gradient", Value::Bool(false), ParamUi::Checkbox),
                p("textures/color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("textures/opacity", "Opacity", num(1.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("textures/frontMode", "Front Mode", Value::Enum(1), popup(&SHATTER_TEXTURE_MODES)),
                p("textures/sideMode", "Side Mode", Value::Enum(0), popup(&SHATTER_TEXTURE_MODES)),
                p("textures/backMode", "Back Mode", Value::Enum(1), popup(&SHATTER_TEXTURE_MODES)),
                p("textures/frontLayer", "Front Layer", Value::Layer(None), ParamUi::Layer),
                p("textures/sideLayer", "Side Layer", Value::Layer(None), ParamUi::Layer),
                p("textures/backLayer", "Back Layer", Value::Layer(None), ParamUi::Layer),
            ]
            .into_iter()
            .chain(crate::card3d::camera_params(4.0))
            .chain(crate::card3d::lighting_params(1.0, 0.25))
            .chain(crate::card3d::material_params(0.75))
            .chain([p("randomSeed", "Random Seed", num(0.0), slider(0.0, 10_000.0, 0.0, 1000.0, 0))])
            .collect(),
            shatter,
        ),
        spec(
            "ec.sim.foam",
            "Foam",
            vec![
                p("view", "View", Value::Enum(0), popup(&["Draft", "Draft + Flow Map", "Rendered"])),
                p("producer/producerPoint", "Producer Point", pt(0.5, 0.5), ParamUi::Point),
                p("producer/producerXSize", "Producer X Size", num(0.05), slider(0.0, 2.0, 0.0, 1.0, 3)),
                p("producer/producerYSize", "Producer Y Size", num(0.05), slider(0.0, 2.0, 0.0, 1.0, 3)),
                p("producer/producerOrientation", "Producer Orientation", num(0.0), ParamUi::Angle),
                p("producer/zoomProducerPoint", "Zoom Producer Point", Value::Bool(false), ParamUi::Checkbox),
                p("producer/productionRate", "Production Rate", num(1.0), slider(0.0, 50.0, 0.0, 10.0, 3)),
                p("bubbles/size", "Size", num(0.5), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("bubbles/sizeVariance", "Size Variance", num(0.5), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("bubbles/lifespan", "Lifespan", num(300.0), slider(1.0, 30_000.0, 1.0, 1000.0, 1)),
                p("bubbles/growthSpeed", "Bubble Growth Speed", num(0.1), slider(0.0001, 10.0, 0.0001, 1.0, 3)),
                p("bubbles/strength", "Strength", num(10.0), slider(0.0, 100.0, 0.0, 20.0, 2)),
                p("physics/initialSpeed", "Initial Speed", num(0.0), slider(-100.0, 100.0, -10.0, 10.0, 3)),
                p("physics/initialDirection", "Initial Direction", num(0.0), ParamUi::Angle),
                p("physics/windSpeed", "Wind Speed", num(0.5), slider(-100.0, 100.0, -10.0, 10.0, 3)),
                p("physics/windDirection", "Wind Direction", num(90.0), ParamUi::Angle),
                p("physics/turbulence", "Turbulence", num(0.5), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("physics/wobbleAmount", "Wobble Amount", num(0.05), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("physics/repulsion", "Repulsion", num(1.0), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("physics/popVelocity", "Pop Velocity", num(1.0), slider(-100.0, 100.0, -10.0, 10.0, 3)),
                p("physics/viscosity", "Viscosity", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("physics/stickiness", "Stickiness", num(0.75), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("zoom", "Zoom", num(1.0), slider(0.01, 50.0, 0.1, 5.0, 3)),
                p("universeSize", "Universe Size", num(1.0), slider(0.01, 10.0, 0.5, 3.0, 3)),
                p("rendering/blendMode", "Blend Mode", Value::Enum(0), popup(&["Transparent", "Solid Old on Top", "Solid New on Top"])),
                p("rendering/bubbleTexture", "Bubble Texture", Value::Enum(0), popup(&FOAM_TEXTURES)),
                p("rendering/bubbleTextureLayer", "Bubble Texture Layer", Value::Layer(None), ParamUi::Layer),
                p("rendering/bubbleOrientation", "Bubble Orientation", Value::Enum(0), popup(&["Fixed", "Physical Orientation", "Bubble Velocity"])),
                p("rendering/environmentMap", "Environment Map", Value::Layer(None), ParamUi::Layer),
                p("rendering/reflectionStrength", "Reflection Strength", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("rendering/reflectionConvergence", "Reflection Convergence", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("flowMap/flowMap", "Flow Map", Value::Layer(None), ParamUi::Layer),
                p("flowMap/flowMapSteepness", "Flow Map Steepness", num(0.05), slider(-100.0, 100.0, -1.0, 1.0, 3)),
                p("flowMap/flowMapFits", "Flow Map Fits", Value::Enum(0), popup(&["Background", "Universe"])),
                p("flowMap/simulationQuality", "Simulation Quality", Value::Enum(0), popup(&["Normal", "High", "Intense"])),
                p("randomSeed", "Random Seed", num(1.0), slider(0.0, 10_000.0, 0.0, 1000.0, 0)),
            ],
            foam,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, run_fx};

    fn ramp(w: u32, hh: u32) -> Image {
        let mut img = Image::new(w, hh);
        for y in 0..hh {
            for x in 0..w {
                img.set(x, y, [x as f32 / w as f32, y as f32 / hh as f32, 0.4, 1.0]);
            }
        }
        img
    }

    fn run(id: &str, vals: &[(&str, Value)], img: Image, t: f64) -> Image {
        run_fx(id, vals, img, t, EffectEnv::default()).img
    }

    fn max_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
    }

    fn sum_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).sum::<f32>()).sum()
    }

    #[test]
    fn piece_at_rest_is_identity() {
        let src = ramp(16, 16);
        let cam = Cam { c: [8.0, 8.0], d: 64.0 };
        let id = rot_axis([1.0, 0.0, 0.0], 0.0);
        let pc = Piece::new(&[[0.0, 0.0], [16.0, 0.0], [16.0, 16.0], [0.0, 16.0]], [8.0, 8.0], &id, [8.0, 8.0, 0.0], cam).unwrap();
        let mut out = Image::new(16, 16);
        draw_pieces(&mut out, &[pc], &src, None);
        assert!(max_diff(&out, &src) < 1e-4);
    }

    #[test]
    fn ball_action_draws_balls() {
        let v = [("ballSize", num(80.0))];
        let a = run("ec.sim.ccballaction", &v, ramp(32, 32), 0.0);
        assert_eq!(a, run("ec.sim.ccballaction", &v, ramp(32, 32), 0.0));
        assert!(a.data.iter().any(|p| p[3] > 0.9) && a.data.iter().any(|p| p[3] == 0.0), "balls with gaps");
        let none = run("ec.sim.ccballaction", &[("ballSize", num(0.0))], ramp(32, 32), 0.0);
        assert!(none.data.iter().all(|p| p[3] == 0.0));
        let sc = run("ec.sim.ccballaction", &[("ballSize", num(80.0)), ("scatter", num(20.0))], ramp(32, 32), 0.0);
        assert!(sum_diff(&a, &sc) > 1.0);
    }

    #[test]
    fn pixel_polly_breaks_after_start() {
        let v = [("forceCenter", pt(16.0, 16.0)), ("gridSpacing", num(4.0))];
        let before = run("ec.sim.ccpixelpolly", &v, ramp(32, 32), 0.0);
        assert_eq!(before, ramp(32, 32), "intact before the start time");
        let a = run("ec.sim.ccpixelpolly", &v, ramp(32, 32), 0.5);
        assert_eq!(a, run("ec.sim.ccpixelpolly", &v, ramp(32, 32), 0.5));
        assert!(sum_diff(&a, &ramp(32, 32)) > 10.0);
        let tex = run("ec.sim.ccpixelpolly", &[("forceCenter", pt(16.0, 16.0)), ("gridSpacing", num(4.0)), ("object", Value::Enum(3))], ramp(32, 32), 0.5);
        assert!(sum_diff(&a, &tex) > 0.1);
    }

    #[test]
    fn scatterize_identity_and_scatter() {
        assert_eq!(run("ec.sim.ccscatterize", &[], ramp(24, 24), 0.0), ramp(24, 24));
        let a = run("ec.sim.ccscatterize", &[("scatter", num(5.0))], ramp(24, 24), 0.0);
        assert_eq!(a, run("ec.sim.ccscatterize", &[("scatter", num(5.0))], ramp(24, 24), 0.0));
        assert!(sum_diff(&a, &ramp(24, 24)) > 1.0);
        let tw = run("ec.sim.ccscatterize", &[("leftTwist", num(80.0)), ("rightTwist", num(80.0))], ramp(24, 24), 0.0);
        assert!(sum_diff(&tw, &ramp(24, 24)) > 1.0);
    }

    #[test]
    fn card_dance_identity_and_rotation() {
        let src = ramp(30, 20);
        let a = run("ec.sim.carddance", &[], src.clone(), 0.0);
        assert!(max_diff(&a, &src) < 0.02, "flat cards reproduce the layer: {}", max_diff(&a, &src));
        let r = run("ec.sim.carddance", &[("yRotation/yRotSource", Value::Enum(1)), ("yRotation/yRotMultiplier", num(90.0))], src.clone(), 0.0);
        assert_eq!(r, run("ec.sim.carddance", &[("yRotation/yRotSource", Value::Enum(1)), ("yRotation/yRotMultiplier", num(90.0))], src.clone(), 0.0));
        assert!(sum_diff(&r, &src) > 5.0);
        let z = run("ec.sim.carddance", &[("zPosition/zPosOffset", num(-1.0))], src.clone(), 0.0);
        assert!(sum_diff(&z, &src) > 5.0, "cards pulled towards the camera grow");
    }

    #[test]
    fn shatter_at_rest_then_breaks() {
        let v = [("view", Value::Enum(0)), ("force1/force1Position", pt(16.0, 16.0)), ("shape/origin", pt(16.0, 16.0))];
        let src = ramp(32, 32);
        let t0 = run("ec.sim.shatter", &v, src.clone(), 0.0);
        assert!(max_diff(&t0, &src) < 0.03, "{}", max_diff(&t0, &src));
        let t1 = run("ec.sim.shatter", &v, src.clone(), 0.5);
        assert_eq!(t1, run("ec.sim.shatter", &v, src.clone(), 0.5));
        assert!(sum_diff(&t1, &src) > 10.0);
        for pat in 0..SHATTER_PATTERNS.len() as u32 {
            let vv =
                [("view", Value::Enum(0)), ("shape/pattern", Value::Enum(pat)), ("force1/force1Position", pt(16.0, 16.0)), ("shape/origin", pt(16.0, 16.0))];
            let r = run("ec.sim.shatter", &vv, src.clone(), 0.0);
            assert!(max_diff(&r, &src) < 0.05, "pattern {pat} tiles the layer: {}", max_diff(&r, &src));
        }
        let wire = run("ec.sim.shatter", &[("shape/repetitions", num(3.0))], src.clone(), 0.0);
        assert!(wire.data.iter().any(|p| p[3] > 0.5) && wire.data.iter().any(|p| p[3] == 0.0));
        let layer_only =
            run("ec.sim.shatter", &[("view", Value::Enum(0)), ("render", Value::Enum(1)), ("force1/force1Position", pt(16.0, 16.0))], src.clone(), 0.5);
        assert!(layer_only.get(16, 16)[3] == 0.0, "broken centre removed");
    }

    #[test]
    fn caustics_flat_water() {
        let src = ramp(24, 24);
        let a = run(
            "ec.sim.caustics",
            &[
                ("water/surfaceOpacity", num(0.0)),
                ("lighting/ambientLight", num(0.0)),
                ("material/diffuse", num(1.0)),
                ("lighting/lightHeight", num(1000.0)),
                ("material/specular", num(0.0)),
            ],
            src.clone(),
            0.0,
        );
        assert!(max_diff(&a, &src) < 0.01, "flat water, light overhead = bottom: {}", max_diff(&a, &src));
        let b = run("ec.sim.caustics", &[], src.clone(), 0.0);
        assert_eq!(b, run("ec.sim.caustics", &[], src.clone(), 0.0));
        assert!(sum_diff(&b, &src) > 0.1);
    }

    #[test]
    fn caustics_sky_and_light_types() {
        let src = Image::filled(32, 32, [0.5, 0.5, 0.5, 1.0]);
        let env = EffectEnv { host: Some(&FoamHost), ..Default::default() };
        let run_h = |vals: &[(&str, Value)]| run_fx("ec.sim.caustics", vals, src.clone(), 0.0, env).img;
        // A mirror-like surface shows the (green) sky.
        let mirror = [("water/surfaceOpacity", num(1.0)), ("sky/sky", Value::Layer(Some(2))), ("sky/intensity", num(1.0))];
        let p = run_h(&mirror).get(16, 16);
        assert!(p[1] > p[0] + 0.3 && p[1] > p[2] + 0.3, "{p:?}");
        // Point Source lights pixels by their own direction to the light: the image is no
        // longer uniform; Distant Source lights the flat water evenly.
        let lit = |t: u32| {
            run_h(&[
                ("lighting/lightType", Value::Enum(t)),
                ("lighting/lightPosition", pt(0.0, 0.0)),
                ("lighting/lightHeight", num(0.2)),
                ("water/surfaceOpacity", num(0.0)),
            ])
        };
        let (distant, point) = (lit(0), lit(1));
        assert!((distant.get(2, 2)[0] - distant.get(30, 30)[0]).abs() < 1e-5);
        assert!(point.get(2, 2)[0] > point.get(30, 30)[0] + 0.02, "{:?} {:?}", point.get(2, 2), point.get(30, 30));
        // First Comp Light without a comp light: ambient only.
        let none = run_h(&[("lighting/lightType", Value::Enum(2)), ("water/surfaceOpacity", num(0.0))]);
        assert!(none.get(16, 16)[0] < distant.get(16, 16)[0]);
    }

    #[test]
    fn wave_world_wireframe_controls_ground_and_dry_areas() {
        let base = [("producer1/producer1Position", pt(8.0, 16.0)), ("simulation/gridResolution", num(24.0))];
        let wire = |extra: &[(&str, Value)]| run("ec.sim.waveworld", &[base.as_slice(), extra].concat(), Image::new(32, 32), 0.5);
        assert_ne!(wire(&[]), wire(&[("wireframeControls/horizontalRotation", num(40.0))]));
        assert_ne!(wire(&[]), wire(&[("wireframeControls/verticalScale", num(3.0))]));
        // Ground (left dark, right light): the light side rises above the water and is dry.
        let env = EffectEnv { host: Some(&FoamHost), ..Default::default() };
        let hm = |extra: &[(&str, Value)]| {
            let mut v: Vec<(&str, Value)> = base.to_vec();
            v.extend([("view", Value::Enum(1)), ("ground/ground", Value::Layer(Some(1))), ("ground/steepness", num(0.5)), ("ground/height", num(0.25))]);
            v.extend_from_slice(extra);
            run_fx("ec.sim.waveworld", &v, Image::new(32, 32), 1.0, env).img
        };
        let solid = hm(&[]);
        let clear = hm(&[("heightMapControls/renderDryAreasAs", Value::Enum(1))]);
        assert!(solid.get(30, 16)[3] > 0.99 && clear.get(30, 16)[3] == 0.0);
        assert!(clear.get(4, 16)[3] > 0.99, "wet side stays");
        // Dry ground stays flat: mid grey (brightness 0.5) there.
        assert!((solid.get(30, 16)[0] - 0.5).abs() < 1e-4);
        // Wave Strength slows waves over shallow water: the surface differs.
        assert_ne!(hm(&[]), hm(&[("ground/waveStrength", num(1.0))]));
    }

    #[test]
    fn wave_world_seek_and_waves() {
        let v = vec![("view", Value::Enum(1)), ("producer1/producer1Position", pt(16.0, 16.0)), ("simulation/gridResolution", num(24.0))];
        let _ = run("ec.sim.waveworld", &v, Image::new(32, 32), 0.5);
        let a = run("ec.sim.waveworld", &v, Image::new(32, 32), 1.0);
        let mut v2 = v.clone();
        v2.push(("heightMapControls/transparency", num(0.0)));
        assert_eq!(a, run("ec.sim.waveworld", &v2, Image::new(32, 32), 1.0));
        // Waves spread: some pixels differ from the flat-water grey.
        assert!(a.data.iter().any(|p| (p[0] - 0.5).abs() > 0.05), "waves");
        let flat = run("ec.sim.waveworld", &[("view", Value::Enum(1)), ("producer1/producer1Amplitude", num(0.0))], Image::new(32, 32), 1.0);
        assert!(flat.data.iter().all(|p| (p[0] - 0.5).abs() < 1e-5));
        let wire =
            run("ec.sim.waveworld", &[("producer1/producer1Position", pt(16.0, 16.0)), ("simulation/gridResolution", num(10.0))], Image::new(32, 32), 0.3);
        assert!(wire.data.iter().any(|p| p[1] > 0.3));
    }

    #[test]
    fn wave_world_steps_match_direct() {
        // Fresh caches: resumed simulation equals simulation from scratch.
        let v = vec![
            ("view", Value::Enum(1)),
            ("producer1/producer1Position", pt(16.0, 16.0)),
            ("simulation/gridResolution", num(20.0)),
            ("simulation/damping", num(0.07)),
        ];
        let a = run("ec.sim.waveworld", &v, Image::new(32, 32), 1.3);
        let mut v2 = v.clone();
        v2.push(("simulation/damping", num(0.07000001)));
        // Different key: computed from scratch at 1.3 directly vs via 0.4 (both fresh keys).
        let _ = run("ec.sim.waveworld", &v2, Image::new(32, 32), 0.4);
        let b = run("ec.sim.waveworld", &v2, Image::new(32, 32), 1.3);
        assert!(max_diff(&a, &b) < 1e-3, "{}", max_diff(&a, &b));
    }

    #[test]
    fn foam_bubbles_drift_and_seek() {
        let v = vec![("producer/producerPoint", pt(16.0, 16.0)), ("view", Value::Enum(2)), ("producer/productionRate", num(2.0))];
        let none = run("ec.sim.foam", &v, Image::new(32, 32), 0.0);
        assert!(none.data.iter().all(|p| p[3] == 0.0));
        let _ = run("ec.sim.foam", &v, Image::new(32, 32), 0.6);
        let a = run("ec.sim.foam", &v, Image::new(32, 32), 1.2);
        let mut v2 = v.clone();
        v2.push(("randomSeed", num(1.0)));
        assert_eq!(a, run("ec.sim.foam", &v2, Image::new(32, 32), 1.2));
        assert!(a.data.iter().any(|p| p[3] > 0.05), "bubbles");
        let draft = run("ec.sim.foam", &[("producer/producerPoint", pt(16.0, 16.0)), ("producer/productionRate", num(2.0))], Image::new(32, 32), 1.0);
        assert!(draft.data.iter().any(|p| p[3] > 0.05));
    }

    #[test]
    fn foam_strength_controls_early_popping() {
        // Strong bubbles all live their full lifespan; weak ones pop at random earlier.
        assert_eq!(foam_pop_age(7, 300.0, 0.0, 1), 300.0);
        let early = (0..200).filter(|&id| foam_pop_age(id, 300.0, 1.0, 1) < 150.0).count();
        assert!(early > 60 && early < 140, "{early}");
        let count = |strength: f64| {
            let v = vec![
                ("producer/producerPoint", pt(16.0, 16.0)),
                ("producer/productionRate", num(3.0)),
                ("bubbles/lifespan", num(40.0)),
                ("bubbles/strength", num(strength)),
                ("physics/windSpeed", num(0.0)),
            ];
            run("ec.sim.foam", &v, Image::new(32, 32), 1.5).data.iter().map(|p| p[3]).sum::<f32>()
        };
        assert!(count(0.0) < count(20.0) * 0.9, "{} vs {}", count(0.0), count(20.0));
    }

    /// A host with a comp camera (identity, or tilted) and a first comp light.
    struct Scene {
        camera: Option<[[f64; 4]; 3]>,
        light: Option<crate::CompLight>,
        layer: Option<Image>,
    }
    impl crate::EffectHost for Scene {
        fn layer(&self, _: u64, _: bool) -> Option<crate::LayerPixels> {
            let img = self.layer.clone()?;
            let size = [img.width as f64, img.height as f64];
            Some(crate::LayerPixels { buf: Buf { img, offset: [0.0; 2], scale: 1.0 }, size })
        }
        fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
            None
        }
        fn comp_scene(&self) -> Option<crate::CompScene> {
            Some(crate::CompScene { camera: self.camera, camera_layer: true, light: self.light })
        }
    }

    fn run_env(id: &str, vals: &[(&str, Value)], img: Image, t: f64, host: &Scene) -> Image {
        run_fx(id, vals, img, t, EffectEnv { host: Some(host), ..Default::default() }).img
    }

    #[test]
    fn card_dance_camera_systems() {
        let src = ramp(40, 30);
        // Camera Position: rotating the camera changes the view; zero rotation is the layer.
        let r = run("ec.sim.carddance", &[("cameraPosition/yRotation", num(30.0))], src.clone(), 0.0);
        assert!(sum_diff(&r, &src) > 10.0);
        let order = run("ec.sim.carddance", &[("cameraPosition/yRotation", num(30.0)), ("cameraPosition/xRotation", num(30.0))], src.clone(), 0.0);
        let order2 = run(
            "ec.sim.carddance",
            &[("cameraPosition/yRotation", num(30.0)), ("cameraPosition/xRotation", num(30.0)), ("cameraPosition/transformOrder", Value::Enum(5))],
            src.clone(),
            0.0,
        );
        assert!(sum_diff(&order, &order2) > 1.0, "Transform Order matters");
        // X, Y Position moves the view: the layer shifts by the offset.
        let moved = run("ec.sim.carddance", &[("cameraPosition/xyPosition", pt(30.0, 15.0))], src.clone(), 0.0);
        assert!((moved.get(25, 10)[0] - src.get(15, 10)[0]).abs() < 0.03, "{:?} {:?}", moved.get(25, 10), src.get(15, 10));
        // Corner Pins: the layer's corners land on the pins.
        let pins = [
            ("cameraSystem", Value::Enum(1)),
            ("cornerPins/upperLeftCorner", pt(10.0, 5.0)),
            ("cornerPins/upperRightCorner", pt(30.0, 5.0)),
            ("cornerPins/lowerLeftCorner", pt(10.0, 25.0)),
            ("cornerPins/lowerRightCorner", pt(30.0, 25.0)),
        ];
        let cp = run("ec.sim.carddance", &pins, src.clone(), 0.0);
        assert_eq!(cp.get(3, 3)[3], 0.0, "outside the pinned quad");
        assert!(cp.get(20, 15)[3] > 0.99);
        assert!((cp.get(20, 15)[0] - src.get(20, 15)[0]).abs() < 0.05, "centre maps to centre");
        // Comp Camera: the identity camera reproduces the layer; a camera that doubles x moves it.
        let ident = Scene { camera: Some([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.002, 1.0]]), light: None, layer: None };
        let cc = run_env("ec.sim.carddance", &[("cameraSystem", Value::Enum(2))], src.clone(), 0.0, &ident);
        assert!(max_diff(&cc, &src) < 0.02, "{}", max_diff(&cc, &src));
        let half = Scene { camera: Some([[0.5, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, 0.002, 1.0]]), ..ident };
        let cc = run_env("ec.sim.carddance", &[("cameraSystem", Value::Enum(2))], src.clone(), 0.0, &half);
        assert_eq!(cc.get(35, 25)[3], 0.0, "half-size view leaves the far corner empty");
    }

    #[test]
    fn card_dance_lighting_and_material() {
        let src = Image::filled(40, 30, [0.5, 0.5, 0.5, 1.0]);
        let tilt = [("rows", num(1.0)), ("columns", num(1.0)), ("yRotation/yRotSource", Value::Enum(0)), ("yRotation/yRotOffset", num(60.0))];
        let lit = |extra: &[(&str, Value)]| {
            let mut v: Vec<(&str, Value)> = tilt.to_vec();
            v.extend_from_slice(extra);
            run("ec.sim.carddance", &v, src.clone(), 0.0).get(20, 15)
        };
        let base = lit(&[]);
        // Tilted cards facing away from a frontal light get darker than flat ones.
        assert!(base[0] < 0.45, "{base:?}");
        // Light colour tints; intensity brightens; ambient alone lights evenly.
        let red = lit(&[("lighting/lightColor", col(1.0, 0.0, 0.0))]);
        assert!(red[0] > red[1] + 0.05);
        assert!(lit(&[("lighting/lightIntensity", num(2.0))])[0] > base[0]);
        let amb = lit(&[("lighting/ambientLight", num(1.0)), ("material/diffuse", num(0.0))]);
        assert!((amb[0] - 0.5).abs() < 1e-3);
        // A light placed to the side (Point Source) changes the shading of the tilted card.
        let side = lit(&[("lighting/lightType", Value::Enum(1)), ("lighting/lightPosition", pt(200.0, 15.0)), ("lighting/lightDepth", num(0.2))]);
        assert!((side[0] - base[0]).abs() > 0.02, "{side:?} vs {base:?}");
        // Specular adds a highlight on flat, frontally lit cards.
        let flat = run("ec.sim.carddance", &[("material/specular", num(0.5))], src.clone(), 0.0).get(20, 15);
        assert!(flat[0] > 0.9, "{flat:?}");
        // First Comp Light: no light in the comp → ambient only; a light from the host is used.
        let none = Scene { camera: None, light: None, layer: None };
        let a = run_env("ec.sim.carddance", &[("lighting/lightType", Value::Enum(2))], src.clone(), 0.0, &none).get(20, 15);
        assert!((a[0] - 0.5 * (0.25 + 0.75)).abs() < 1e-3, "{a:?}");
        let blue = Scene { light: Some(crate::CompLight { pos: [20.0, 15.0, -100.0], dir: [0.0, 0.0, 1.0], color: [0.0, 0.0, 1.0], kind: 0 }), ..none };
        let a = run_env("ec.sim.carddance", &[("lighting/lightType", Value::Enum(2))], src.clone(), 0.0, &blue).get(20, 15);
        assert!(a[2] > a[0] + 0.2, "{a:?}");
    }

    #[test]
    fn shatter_custom_map_gradient_textures_and_extrusion() {
        let src = ramp(32, 32);
        let base = [("view", Value::Enum(0)), ("force1/force1Position", pt(16.0, 16.0)), ("force1/force1Radius", num(2.0)), ("shape/origin", pt(16.0, 16.0))];
        let with = |extra: &[(&str, Value)], t: f64, host: Option<&Scene>| {
            let mut v: Vec<(&str, Value)> = base.to_vec();
            v.extend_from_slice(extra);
            match host {
                Some(h) => run_env("ec.sim.shatter", &v, src.clone(), t, h),
                None => run("ec.sim.shatter", &v, src.clone(), t),
            }
        };
        // Custom Shatter Map: a map white on the left and black on the right; with White Tiles
        // Fixed the left half stays put while the right half breaks away.
        let mut map = Image::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                map.set(x, y, if x < 16 { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] });
            }
        }
        let host = Scene { camera: None, light: None, layer: Some(map) };
        let custom = [("shape/pattern", Value::Enum(5)), ("shape/customShatterMap", Value::Layer(Some(1))), ("shape/whiteTilesFixed", Value::Bool(true))];
        let at_rest = with(&custom, 0.0, Some(&host));
        assert!(max_diff(&at_rest, &src) < 0.05, "custom pieces tile the layer: {}", max_diff(&at_rest, &src));
        let broke = with(&custom, 1.0, Some(&host));
        assert!(max_diff_region(&broke, &src, 0..12) < 0.05, "white tiles fixed");
        assert!(sum_diff_region(&broke, &src, 20..32) > 5.0, "black group moved");
        // Gradient: a black gradient layer with threshold 50 % keeps everything together.
        let black = Scene { camera: None, light: None, layer: Some(Image::filled(32, 32, [0.0, 0.0, 0.0, 1.0])) };
        let g = with(&[("gradient/gradientLayer", Value::Layer(Some(1))), ("gradient/shatterThreshold", num(50.0))], 1.0, Some(&black));
        assert!(max_diff(&g, &src) < 0.05);
        let gi = with(
            &[("gradient/gradientLayer", Value::Layer(Some(1))), ("gradient/shatterThreshold", num(50.0)), ("gradient/invertGradient", Value::Bool(true))],
            1.0,
            Some(&black),
        );
        assert!(sum_diff(&gi, &src) > 10.0);
        // Textures: Front Mode Color paints the pieces flat; Opacity fades them.
        let flat = with(&[("textures/frontMode", Value::Enum(0)), ("textures/color", col(0.0, 1.0, 0.0))], 0.0, None);
        let p = flat.get(16, 16);
        assert!(p[1] > 0.95 && p[0] < 0.05, "{p:?}");
        let half = with(&[("textures/opacity", num(0.5)), ("shape/repetitions", num(2.0))], 0.0, None);
        assert!((half.get(8, 4)[3] - 0.5).abs() < 0.02, "{:?}", half.get(8, 4));
        // Extrusion: thicker pieces cover more once they tumble.
        let cover = |d: f64| {
            with(&[("shape/extrusionDepth", num(d)), ("textures/sideMode", Value::Enum(0)), ("textures/color", col(1.0, 1.0, 1.0))], 0.6, None)
                .data
                .iter()
                .map(|p| p[3])
                .sum::<f32>()
        };
        assert!(cover(1.0) > cover(0.0) + 1.0, "{} vs {}", cover(1.0), cover(0.0));
        // Deterministic and seek-consistent.
        assert_eq!(with(&custom, 0.7, Some(&host)), with(&custom, 0.7, Some(&host)));
    }

    fn max_diff_region(a: &Image, b: &Image, xs: std::ops::Range<u32>) -> f32 {
        let mut m = 0.0f32;
        for y in 0..a.height {
            for x in xs.clone() {
                let (p, q) = (a.get(x as i64, y as i64), b.get(x as i64, y as i64));
                m = (0..4).map(|k| (p[k] - q[k]).abs()).fold(m, f32::max);
            }
        }
        m
    }

    fn sum_diff_region(a: &Image, b: &Image, xs: std::ops::Range<u32>) -> f32 {
        let mut s = 0.0;
        for y in 0..a.height {
            for x in xs.clone() {
                let (p, q) = (a.get(x as i64, y as i64), b.get(x as i64, y as i64));
                s += (0..4).map(|k| (p[k] - q[k]).abs()).sum::<f32>();
            }
        }
        s
    }

    /// Layer 1 = a left-dark, right-light ramp; layer 2 = solid green.
    struct FoamHost;
    impl crate::EffectHost for FoamHost {
        fn layer(&self, id: u64, _: bool) -> Option<crate::LayerPixels> {
            let img = if id == 1 { ramp(32, 32) } else { Image::filled(32, 32, [0.0, 1.0, 0.0, 1.0]) };
            Some(crate::LayerPixels { buf: Buf { img, offset: [0.0; 2], scale: 1.0 }, size: [32.0, 32.0] })
        }
        fn layer_at(&self, id: u64, _: f64, me: bool) -> Option<crate::LayerPixels> {
            self.layer(id, me)
        }
        fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
            None
        }
    }

    #[test]
    fn foam_flow_map_pop_velocity_wobble_textures_and_reflection() {
        let base = vec![
            ("producer/producerPoint", pt(16.0, 16.0)),
            ("producer/productionRate", num(2.0)),
            ("physics/windSpeed", num(0.0)),
            ("physics/turbulence", num(0.0)),
            ("view", Value::Enum(2)),
        ];
        let with = |extra: &[(&str, Value)], t: f64| {
            let mut v = base.clone();
            v.extend_from_slice(extra);
            run_fx("ec.sim.foam", &v, Image::new(32, 32), t, EffectEnv { host: Some(&FoamHost), ..Default::default() }).img
        };
        let cx = |img: &Image| {
            let (mut s, mut n) = (0.0, 0.0);
            for y in 0..32 {
                for x in 0..32 {
                    let a = img.get(x, y)[3];
                    s += a * x as f32;
                    n += a;
                }
            }
            s / n.max(1e-6)
        };
        // Flow map: bubbles run downhill (towards the dark left side).
        let still = with(&[], 1.0);
        let flowing = with(&[("flowMap/flowMap", Value::Layer(Some(1))), ("flowMap/flowMapSteepness", num(1.0))], 1.0);
        assert!(cx(&flowing) < cx(&still) - 1.0, "{} vs {}", cx(&flowing), cx(&still));
        let hq =
            with(&[("flowMap/flowMap", Value::Layer(Some(1))), ("flowMap/flowMapSteepness", num(1.0)), ("flowMap/simulationQuality", Value::Enum(2))], 1.0);
        assert_eq!(
            hq,
            with(&[("flowMap/flowMap", Value::Layer(Some(1))), ("flowMap/flowMapSteepness", num(1.0)), ("flowMap/simulationQuality", Value::Enum(2))], 1.0)
        );
        // Pop velocity scatters the survivors when short-lived bubbles pop.
        let short = [("bubbles/lifespan", num(8.0)), ("bubbles/strength", num(0.0))];
        let a = with(&[short.as_slice(), &[("physics/popVelocity", num(0.0))]].concat(), 1.0);
        let b2 = with(&[short.as_slice(), &[("physics/popVelocity", num(20.0))]].concat(), 1.0);
        assert_ne!(a, b2);
        // Wobble changes the drawn bubbles.
        assert_ne!(with(&[("physics/wobbleAmount", num(0.0))], 1.0), with(&[("physics/wobbleAmount", num(3.0))], 1.0));
        // User Defined texture: bubbles show the texture layer (green).
        let tex = with(&[("rendering/bubbleTexture", Value::Enum(5)), ("rendering/bubbleTextureLayer", Value::Layer(Some(2)))], 1.0);
        assert!(tex.data.iter().any(|p| p[1] > 0.5 && p[0] < 0.1));
        // Environment map: reflections tint the bubbles.
        let refl = with(&[("rendering/environmentMap", Value::Layer(Some(2))), ("rendering/reflectionStrength", num(1.0))], 1.0);
        let g = |i: &Image| i.data.iter().map(|p| p[1] - p[0]).sum::<f32>();
        assert!(g(&refl) > g(&still) + 1.0);
        // Zoom Producer Point zooms about the producer instead of the layer centre.
        let off = [("producer/producerPoint", pt(8.0, 8.0)), ("zoom", num(2.0))];
        let z1 = with(&off, 1.0);
        let z2 = with(&[off.as_slice(), &[("producer/zoomProducerPoint", Value::Bool(true))]].concat(), 1.0);
        assert!((cx(&z2) - 8.0).abs() < 2.0 && cx(&z1) < 4.0, "{} vs {}", cx(&z2), cx(&z1));
    }
}

//! Parametric primitives (Layer ▸ New ▸ Cube/Sphere/Plane/Torus/Cone/Cylinder).
//!
//! Meshes are built in model space (+Y up, counter-clockwise front faces, like glTF), centred
//! on the origin, in pixels, with normals, UVs (0..1 across each face/around each surface) and
//! tangents.

use std::f32::consts::{PI, TAU};

use serde::{Deserialize, Serialize};

use crate::{Material, Mesh, Model, Primitive};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveKind {
    #[default]
    Cube,
    Sphere,
    Plane,
    Torus,
    Cone,
    Cylinder,
}

impl PrimitiveKind {
    pub const ALL: [PrimitiveKind; 6] =
        [PrimitiveKind::Cube, PrimitiveKind::Sphere, PrimitiveKind::Plane, PrimitiveKind::Torus, PrimitiveKind::Cone, PrimitiveKind::Cylinder];
    pub fn label(self) -> &'static str {
        match self {
            PrimitiveKind::Cube => "Cube",
            PrimitiveKind::Sphere => "Sphere",
            PrimitiveKind::Plane => "Plane",
            PrimitiveKind::Torus => "Torus",
            PrimitiveKind::Cone => "Cone",
            PrimitiveKind::Cylinder => "Cylinder",
        }
    }
    pub fn parse(s: &str) -> Option<PrimitiveKind> {
        PrimitiveKind::ALL.into_iter().find(|k| k.label().eq_ignore_ascii_case(s.trim()))
    }
}

/// Primitive dimensions (pixels) and tessellation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimParams {
    /// Width, height, depth (cube, plane: width × height).
    pub size: [f32; 3],
    /// Sphere/cone/cylinder radius; torus ring radius.
    pub radius: f32,
    /// Torus tube radius.
    pub tube: f32,
    /// Cone/cylinder height.
    pub height: f32,
    /// Segments around.
    pub segments: u32,
    /// Sphere rings / torus tube sides.
    pub rings: u32,
}

impl Default for PrimParams {
    fn default() -> Self {
        PrimParams { size: [300.0; 3], radius: 150.0, tube: 50.0, height: 300.0, segments: 48, rings: 24 }
    }
}

struct B {
    p: Primitive,
}

impl B {
    fn new() -> B {
        B { p: Primitive { material: Some(0), ..Default::default() } }
    }
    fn v(&mut self, pos: [f32; 3], n: [f32; 3], uv: [f32; 2]) -> u32 {
        self.p.positions.push(pos);
        self.p.normals.push(n);
        self.p.uvs.push(uv);
        (self.p.positions.len() - 1) as u32
    }
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.p.indices.extend([a, b, c]);
    }
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        // a-b-c-d counter-clockwise.
        self.tri(a, b, c);
        self.tri(a, c, d);
    }
}

/// The mesh of a primitive.
pub fn mesh(kind: PrimitiveKind, pr: &PrimParams) -> Primitive {
    let seg = pr.segments.clamp(3, 512);
    let rings = pr.rings.clamp(2, 512);
    let mut b = B::new();
    match kind {
        PrimitiveKind::Cube => {
            let [w, h, d] = pr.size.map(|v| v.abs() * 0.5);
            // (normal, u axis, v axis) per face; v runs down the texture.
            let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
                ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
                ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
                ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0]),
                ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
                ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ];
            let half = [w, h, d];
            for (n, u, v) in faces {
                let c = |su: f32, sv: f32| -> [f32; 3] { [0, 1, 2].map(|k| (n[k] + u[k] * su + v[k] * sv) * half[k]) };
                let i0 = b.v(c(-1.0, -1.0), n, [0.0, 0.0]);
                let i1 = b.v(c(1.0, -1.0), n, [1.0, 0.0]);
                let i2 = b.v(c(1.0, 1.0), n, [1.0, 1.0]);
                let i3 = b.v(c(-1.0, 1.0), n, [0.0, 1.0]);
                // u × v points along -n for these axes (v down): wind so the face is CCW from outside.
                b.quad(i0, i3, i2, i1);
            }
        }
        PrimitiveKind::Plane => {
            let (w, h) = (pr.size[0].abs() * 0.5, pr.size[1].abs() * 0.5);
            let n = [0.0, 0.0, 1.0];
            let i0 = b.v([-w, h, 0.0], n, [0.0, 0.0]);
            let i1 = b.v([w, h, 0.0], n, [1.0, 0.0]);
            let i2 = b.v([w, -h, 0.0], n, [1.0, 1.0]);
            let i3 = b.v([-w, -h, 0.0], n, [0.0, 1.0]);
            b.quad(i0, i3, i2, i1);
        }
        PrimitiveKind::Sphere => {
            let r = pr.radius.abs();
            for j in 0..=rings {
                let th = PI * j as f32 / rings as f32;
                // Exact poles and seam (welded positions).
                let (st, ct) = if j == 0 {
                    (0.0, 1.0)
                } else if j == rings {
                    (0.0, -1.0)
                } else {
                    th.sin_cos()
                };
                for i in 0..=seg {
                    let ph = TAU * (i % seg) as f32 / seg as f32;
                    let n = [st * ph.sin(), ct, st * ph.cos()];
                    b.v([n[0] * r, n[1] * r, n[2] * r], n, [i as f32 / seg as f32, j as f32 / rings as f32]);
                }
            }
            let row = seg + 1;
            for j in 0..rings {
                for i in 0..seg {
                    let a = j * row + i;
                    let (b0, c, d) = (a + row, a + row + 1, a + 1);
                    if j != 0 {
                        b.tri(a, b0, d);
                    }
                    if j != rings - 1 {
                        b.tri(d, b0, c);
                    }
                }
            }
        }
        PrimitiveKind::Torus => {
            let (rr, tr) = (pr.radius.abs(), pr.tube.abs());
            for j in 0..=rings {
                let v = TAU * (j % rings) as f32 / rings as f32;
                for i in 0..=seg {
                    let u = TAU * (i % seg) as f32 / seg as f32;
                    // Ring in the XZ plane.
                    let (cu, su, cv, sv) = (u.cos(), u.sin(), v.cos(), v.sin());
                    let n = [cv * su, sv, cv * cu];
                    let p = [(rr + tr * cv) * su, tr * sv, (rr + tr * cv) * cu];
                    b.v(p, n, [i as f32 / seg as f32, j as f32 / rings as f32]);
                }
            }
            let row = seg + 1;
            for j in 0..rings {
                for i in 0..seg {
                    let a = j * row + i;
                    b.quad(a, a + 1, a + row + 1, a + row);
                }
            }
        }
        PrimitiveKind::Cylinder | PrimitiveKind::Cone => {
            let r = pr.radius.abs();
            let h = pr.height.abs() * 0.5;
            let top_r = if kind == PrimitiveKind::Cone { 0.0 } else { r };
            // Side normal tilt for the cone (slope).
            let (ny, nk) = {
                let s = (r - top_r) / (2.0 * h).max(1e-6);
                let l = (1.0 + s * s).sqrt();
                (s / l, 1.0 / l)
            };
            let base = b.p.positions.len() as u32;
            for i in 0..=seg {
                let a = TAU * (i % seg) as f32 / seg as f32;
                let (s, c) = a.sin_cos();
                let n = [s * nk, ny, c * nk];
                let u = i as f32 / seg as f32;
                b.v([s * top_r, h, c * top_r], n, [u, 0.0]);
                b.v([s * r, -h, c * r], n, [u, 1.0]);
            }
            for i in 0..seg {
                let t0 = base + 2 * i;
                let (b0, t1, b1) = (t0 + 1, t0 + 2, t0 + 3);
                if kind == PrimitiveKind::Cone {
                    b.tri(t0, b0, b1);
                } else {
                    b.quad(t0, b0, b1, t1);
                }
            }
            let caps: &[(f32, f32)] = if kind == PrimitiveKind::Cone { &[(-1.0, r)] } else { &[(1.0, r), (-1.0, r)] };
            for &(side, rad) in caps {
                let n = [0.0, side, 0.0];
                let c = b.v([0.0, h * side, 0.0], n, [0.5, 0.5]);
                let first = b.p.positions.len() as u32;
                for i in 0..=seg {
                    let a = TAU * (i % seg) as f32 / seg as f32;
                    let (s, co) = a.sin_cos();
                    b.v([s * rad, h * side, co * rad], n, [0.5 + 0.5 * s, 0.5 - 0.5 * co * side]);
                }
                for i in 0..seg {
                    let (p0, p1) = (first + i, first + i + 1);
                    if side > 0.0 { b.tri(c, p0, p1) } else { b.tri(c, p1, p0) }
                }
            }
        }
    }
    b.p.compute_tangents();
    b.p
}

/// A primitive as a one-mesh model with one material.
pub fn model(kind: PrimitiveKind, pr: &PrimParams, material: Material) -> Model {
    let mut material = material;
    if kind == PrimitiveKind::Plane {
        material.double_sided = true;
    }
    Model::single(Mesh { name: kind.label().to_string(), primitives: vec![mesh(kind, pr)] }, vec![material])
}

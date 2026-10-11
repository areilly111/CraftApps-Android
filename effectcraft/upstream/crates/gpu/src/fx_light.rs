//! GPU effects, the CC light family (kernels in `shaders/fx_light.wgsl`, every entry point
//! prefixed `flt_`): CC Light Rays, CC Light Burst 2.5, CC Light Sweep and CC Light Wipe, with
//! the CPU effects' exact steps. Centres, radii, the wipe's corner distance and ramp are
//! computed on the CPU in f64 as the CPU effect does; the kernels work relative to the centre.
//! Light Sweep's softened alpha plane is the CPU's `gauss_plane` (box passes, edges repeated).

use effectcraft_effects::EffectCtx;

use crate::context::{Enc, GpuImage, Params};
use crate::effects::{GBuf, gaussian_blur};

/// Compute entry points in `fx_light.wgsl`.
pub(crate) const KERNELS: &[&str] = &["flt_rays", "flt_burst", "flt_sweep", "flt_wipe"];

/// Effect ids implemented here.
pub(crate) const IDS: &[&str] = &["ec.generate.cclightrays", "ec.generate.cclightburst", "ec.generate.cclightsweep", "ec.transition.cclightwipe"];

/// Run effect `id` (one of [`IDS`]); `None` = this parameter combination runs on the CPU.
pub(crate) fn apply(e: &mut Enc, id: &str, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    match id {
        "ec.generate.cclightrays" => light_rays(e, ctx, b),
        "ec.generate.cclightburst" => light_burst(e, ctx, b),
        "ec.generate.cclightsweep" => light_sweep(e, ctx, b),
        "ec.transition.cclightwipe" => light_wipe(e, ctx, b),
        _ => None,
    }
}

/// A same-size per-pixel kernel over the buffer.
fn run(e: &mut Enc, entry: &str, p: &Params, mut b: GBuf, aux: Option<&GpuImage>) -> Option<GBuf> {
    let out = e.scratch(b.img.width, b.img.height);
    e.pixels(entry, p, &b.img, aux, &out, None);
    b.img = out;
    Some(b)
}

fn rgb(c: [f32; 4]) -> [f32; 4] {
    [c[0], c[1], c[2], 0.0]
}

// ---------------------------------------------------------------- CC Light Rays / Burst (generate2.rs)

fn light_rays(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let pr = ctx.params;
    let c = b.to_px(pr.v2("center"));
    let k = (pr.f("intensity") / 100.0 * 0.5).clamp(0.0, 0.95);
    let gain = (pr.f("intensity") / 100.0 * 2.0) as f32;
    let rad = (pr.f("radius") * b.scale).max(0.5);
    let ws = (pr.f("warpSoftness") / 50.0).max(0.0);
    let mut p = Params::default();
    p.u[0] = [(pr.e("shape") == 1) as u32, pr.b("colorFromSource") as u32, pr.e("transferMode"), 0];
    p.f[0] = [c.0 as f32, c.1 as f32, k as f32, gain];
    p.f[1] = [rad as f32, (rad * (1.0 + ws)) as f32 + 1e-3, 0.0, 0.0];
    p.f[2] = rgb(pr.color("color"));
    run(e, "flt_rays", &p, b, None)
}

fn light_burst(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let pr = ctx.params;
    let c = b.to_px(pr.v2("center"));
    let inten = (pr.f("intensity") / 100.0) as f32;
    let k = (pr.f("rayLength") / 100.0).clamp(0.0, 1.0);
    let set_color = pr.b("setColor");
    if k <= 0.0 && (inten - 1.0).abs() < 1e-6 && !set_color {
        return Some(b);
    }
    let mut p = Params::default();
    p.u[0] = [pr.e("burst"), set_color as u32, 0, 0];
    p.f[0] = [c.0 as f32, c.1 as f32, k as f32, inten];
    p.f[1] = rgb(pr.color("color"));
    run(e, "flt_burst", &p, b, None)
}

// ---------------------------------------------------------------- CC Light Sweep (generate2.rs)

fn light_sweep(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let pr = ctx.params;
    let c = b.to_px(pr.v2("center"));
    let dir = pr.f("direction").to_radians();
    let half = (pr.f("width") * b.scale * 0.5).max(0.5);
    let edge_t = pr.f("edgeThickness") * b.scale;
    // The alpha plane (x), softened by util::gauss_plane (edges repeated) when thick enough.
    let alpha = e.scratch(b.img.width, b.img.height);
    e.pixels("fxe_alpha", &Params::default(), &b.img, None, &alpha, None);
    let soft = if edge_t > 0.05 { gaussian_blur(e, &alpha, edge_t * 0.5, edge_t * 0.5, true) } else { alpha };
    let mut p = Params::default();
    p.u[0] = [pr.e("shape"), pr.e("lightReceptionMode"), 0, 0];
    p.f[0] = [c.0 as f32, c.1 as f32, dir.cos() as f32, dir.sin() as f32];
    p.f[1] = [half as f32, (pr.f("sweepIntensity") / 100.0) as f32, (pr.f("edgeIntensity") / 100.0) as f32, 0.0];
    p.f[2] = rgb(pr.color("lightColor"));
    run(e, "flt_sweep", &p, b, Some(&soft))
}

// ---------------------------------------------------------------- CC Light Wipe (transition.rs)

fn light_wipe(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let pr = ctx.params;
    let done = (pr.f("completion") / 100.0).clamp(0.0, 1.0);
    if done <= 0.0 {
        return Some(b);
    }
    let c = b.to_px(pr.v2("center"));
    let shape = pr.e("shape");
    let (s, co) = pr.f("direction").to_radians().sin_cos();
    let reverse = pr.b("reverse");
    let metric = |dx: f64, dy: f64| {
        let (qx, qy) = (dx * co + dy * s, -dx * s + dy * co);
        match shape {
            0 => qx.abs(),
            2 => qx.abs().max(qy.abs()),
            _ => qx.hypot(qy),
        }
    };
    // transition::max_corner_dist.
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    let maxd = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().map(|(x, y)| metric(x - c.0, y - c.1)).fold(1e-6, f64::max);
    let edge = if reverse { (1.0 - done) * maxd } else { done * maxd };
    let band = (maxd * 0.06).max(2.0);
    let ramp = (done * 20.0).min(1.0) as f32 * ((1.0 - done) * 20.0).min(1.0) as f32;
    let mut p = Params::default();
    p.u[0] = [shape, pr.b("colorFromSource") as u32, reverse as u32, 0];
    p.f[0] = [c.0 as f32, c.1 as f32, s as f32, co as f32];
    p.f[1] = [edge as f32, band as f32, pr.f("intensity") as f32 / 100.0 * ramp, 0.0];
    p.f[2] = rgb(pr.color("color"));
    run(e, "flt_wipe", &p, b, None)
}

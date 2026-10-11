//! Utility effects, batch 2: HDR Compander.
//!
//! HDR Compander squeezes high-dynamic-range values into 0–1 (Compress Range) so 8/16 bpc
//! effects can process them, and restores them afterwards (Expand Range) with the same Gain and
//! Gamma; the two modes are exact inverses.

use effectcraft_keyframe::Value;
use rayon::prelude::*;

use crate::util::{premul, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

#[inline]
fn spow(v: f32, e: f32) -> f32 {
    v.signum() * v.abs().powf(e)
}

fn hdr_compander(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let expand = ctx.params.e("mode") == 1;
    let gain = ctx.params.f("gain").max(1e-6) as f32;
    let gamma = ctx.params.f("gamma").max(1e-3) as f32;
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 {
            return;
        }
        let c = c.map(|v| if expand { spow(v, gamma) * gain } else { spow(v / gain, 1.0 / gamma) });
        *px = premul(c, a);
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    vec![EffectSpec {
        id: "ec.utility.hdrcompander",
        name: "HDR Compander",
        category: "Utility",
        params: vec![
            p("mode", "Mode", Value::Enum(0), popup(&["Compress Range", "Expand Range"])),
            p("gain", "Gain", num(1.0), slider(1.0, 100.0, 1.0, 100.0, 2)),
            p("gamma", "Gamma", num(1.0), slider(0.1, 10.0, 0.1, 3.0, 2)),
        ],
        render: hdr_compander,
        gpu: false,
        float: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, run_fx};
    use effectcraft_raster::Image;

    fn hdr() -> Image {
        crate::util::gen_image(8, 4, |x, y| {
            let v = x as f32 * 0.6 + y as f32 * 0.1;
            [v * 0.8, v * 0.4, v * 0.2, 0.8]
        })
    }

    #[test]
    fn defaults_are_identity_and_deterministic() {
        let im = hdr();
        let a = run_fx("ec.utility.hdrcompander", &[], im.clone(), 0.0, EffectEnv::default());
        let b = run_fx("ec.utility.hdrcompander", &[], im.clone(), 0.0, EffectEnv::default());
        assert_eq!(a.img, b.img);
        assert!(a.img.data.iter().zip(&im.data).all(|(p, q)| (0..4).all(|i| (p[i] - q[i]).abs() < 1e-5)));
    }

    #[test]
    fn compress_then_expand_round_trips() {
        let im = hdr();
        let set = [("gain", num(5.0)), ("gamma", num(2.2))];
        let c = run_fx("ec.utility.hdrcompander", &set, im.clone(), 0.0, EffectEnv::default());
        // Compressed values fit in 0–1.
        assert!(c.img.data.iter().all(|p| p[0] <= p[3] + 1e-5));
        let e = run_fx("ec.utility.hdrcompander", &[set[0].clone(), set[1].clone(), ("mode", Value::Enum(1))], c.img, 0.0, EffectEnv::default());
        for (p, q) in e.img.data.iter().zip(&im.data) {
            for i in 0..4 {
                assert!((p[i] - q[i]).abs() < 1e-3, "{p:?} vs {q:?}");
            }
        }
    }
}

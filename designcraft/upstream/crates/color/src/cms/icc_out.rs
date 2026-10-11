//! An ICC output profile of our Generic CMYK press model (for PDF/X output intents): lut16
//! tables sampled from the model — CMYK → Lab from the forward model, Lab → CMYK from its
//! inverse (separation with black generation and the ink limit). The profile is ours, built in
//! code; it describes our model, not a measured press.

use std::sync::OnceLock;

use moxcms::{
    ColorProfile, DataColorSpace, LocalizableString, LutDataType, LutStore, LutType, LutWarehouse, Matrix3d, ProfileClass, ProfileText,
    RenderingIntent,
};

use super::Intent;
use super::lab::Lab;

const A2B_GRID: usize = 9;
const B2A_GRID: usize = 17;

/// ICC v2 legacy 16-bit Lab encoding (lut16Type).
fn enc_lab(l: Lab) -> [u16; 3] {
    let q = |v: f32| v.round().clamp(0.0, 65535.0) as u16;
    [q(l.l * 652.8), q((l.a + 128.0) * 256.0), q((l.b + 128.0) * 256.0)]
}

fn dec_lab(v: [u16; 3]) -> Lab {
    Lab::new(v[0] as f32 / 652.8, v[1] as f32 / 256.0 - 128.0, v[2] as f32 / 256.0 - 128.0)
}

fn lut(inputs: u8, outputs: u8, grid: usize, clut: Vec<u16>) -> LutWarehouse {
    let ident = |n: u8| LutStore::Store16((0..n).flat_map(|_| [0u16, 65535]).collect());
    LutWarehouse::Lut(LutDataType {
        num_input_channels: inputs,
        num_output_channels: outputs,
        num_clut_grid_points: grid as u8,
        matrix: Matrix3d::IDENTITY,
        num_input_table_entries: 2,
        num_output_table_entries: 2,
        input_table: ident(inputs),
        clut_table: LutStore::Store16(clut),
        output_table: ident(outputs),
        lut_type: LutType::Lut16,
    })
}

fn text(s: &str) -> Option<ProfileText> {
    Some(ProfileText::Localizable(vec![LocalizableString::new("en".into(), "US".into(), s.into())]))
}

/// The Generic CMYK model as an ICC output profile (built once).
pub fn generic_cmyk_icc() -> &'static [u8] {
    static P: OnceLock<Vec<u8>> = OnceLock::new();
    P.get_or_init(|| {
        let g = super::generic();
        let step = |i: usize, n: usize| i as f32 / (n - 1) as f32;
        // CMYK → Lab (media-relative), first input channel varying slowest.
        let mut a2b = Vec::with_capacity(A2B_GRID.pow(4) * 3);
        for c in 0..A2B_GRID {
            for m in 0..A2B_GRID {
                for y in 0..A2B_GRID {
                    for k in 0..A2B_GRID {
                        let lab = g.to_lab_rel([step(c, A2B_GRID), step(m, A2B_GRID), step(y, A2B_GRID), step(k, A2B_GRID)]);
                        a2b.extend(enc_lab(lab));
                    }
                }
            }
        }
        // Lab → CMYK through the model's separation.
        let mut b2a = Vec::with_capacity(B2A_GRID.pow(3) * 4);
        let e = |i: usize| (i as f32 * 65535.0 / (B2A_GRID - 1) as f32).round() as u16;
        for l in 0..B2A_GRID {
            for a in 0..B2A_GRID {
                for b in 0..B2A_GRID {
                    let lab = dec_lab([e(l), e(a), e(b)]);
                    let cmyk = g.from_lab(lab, Intent::RelativeColorimetric, true);
                    b2a.extend(cmyk.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16));
                }
            }
        }
        let a2b = lut(4, 3, A2B_GRID, a2b);
        let b2a = lut(3, 4, B2A_GRID, b2a);
        let mut p = ColorProfile::default();
        p.pcs = DataColorSpace::Lab;
        p.color_space = DataColorSpace::Cmyk;
        p.profile_class = ProfileClass::OutputDevice;
        p.rendering_intent = RenderingIntent::RelativeColorimetric;
        let d50 = moxcms::Xyzd { x: 0.9642, y: 1.0, z: 0.8249 };
        p.white_point = d50;
        p.media_white_point = Some(d50);
        p.lut_a_to_b_perceptual = Some(a2b.clone());
        p.lut_a_to_b_colorimetric = Some(a2b.clone());
        p.lut_a_to_b_saturation = Some(a2b);
        p.lut_b_to_a_perceptual = Some(b2a.clone());
        p.lut_b_to_a_colorimetric = Some(b2a.clone());
        p.lut_b_to_a_saturation = Some(b2a);
        p.description = text(super::GENERIC_CMYK);
        p.copyright = text("No copyright; made by DesignCraft from its parametric press model. Use freely.");
        p.encode().unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_cmyk_profile_round_trips() {
        let bytes = generic_cmyk_icc();
        assert!(bytes.len() > 1000);
        assert_eq!(&bytes[36..40], b"acsp");
        assert_eq!(&bytes[16..20], b"CMYK");
        assert_eq!(&bytes[12..16], b"prtr");
        // Read back: CMYK → sRGB through the profile is close to the model.
        let p = super::super::icc::IccProfile::from_bytes(None, bytes).expect("parses");
        let g = super::super::generic();
        for cmyk in [[0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 1.0, 0.0], [0.2, 0.3, 0.1, 0.5]] {
            let via = p.to_srgb(&cmyk, Intent::RelativeColorimetric).expect("converts");
            let model = g.to_srgb(cmyk, false);
            for i in 0..3 {
                assert!((via[i] - model[i]).abs() < 0.04, "{cmyk:?}: {via:?} vs {model:?}");
            }
        }
    }
}

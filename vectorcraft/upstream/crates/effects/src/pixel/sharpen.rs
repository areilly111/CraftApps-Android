//! Sharpen › Unsharp Mask.

use super::{gaussian, to16};

/// Unsharp Mask: each colour channel pushed away from a Gaussian blur of the pixels (σ = `sigma`
/// pixels) by `amount` times their difference, where that difference reaches `threshold` levels.
/// The blur is premultiplied, so edges against transparency don't darken; coverage stays as it is.
pub(super) fn unsharp(px: &mut [[u8; 4]], w: usize, h: usize, amount: f64, sigma: f64, threshold: f64) {
    if sigma.is_nan() || sigma < 0.2 || amount.is_nan() || amount <= 0.0 || h == 0 {
        return;
    }
    let mut blurred = to16(px);
    gaussian(&mut blurred, w, sigma);
    let (amount, threshold) = (amount as f32, threshold as f32);
    for (p, b) in px.iter_mut().zip(&blurred) {
        let a = p[3] as f32;
        if a == 0.0 {
            continue;
        }
        for i in 0..3 {
            let own = p[i] as f32 * 255.0 / a;
            let around = if b[3] > 0 { b[i] as f32 * 255.0 / b[3] as f32 } else { own };
            let diff = own - around;
            if diff.abs() < threshold || diff == 0.0 {
                continue;
            }
            let v = (own + amount * diff).clamp(0.0, 255.0);
            p[i] = (v * a / 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

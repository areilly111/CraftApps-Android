//! Transform panel values: an affine split into scale, rotation and shear the way InDesign shows
//! them (rotation counter-clockwise on screen, shear as an angle along x).

use kurbo::Affine;

/// `m`'s linear part as rotation · shear · scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decomposed {
    /// Scale factors (1 = 100%); `scale_y` is negative for a flipped transform.
    pub scale_x: f64,
    pub scale_y: f64,
    /// Degrees, counter-clockwise on screen, in (-180, 180].
    pub rotation: f64,
    /// Degrees (as `transform.shear` takes them).
    pub shear: f64,
}

pub fn decompose(m: Affine) -> Decomposed {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let sx = a.hypot(b);
    if sx < 1e-12 {
        return Decomposed { scale_x: 0.0, scale_y: 0.0, rotation: 0.0, shear: 0.0 };
    }
    let det = a * d - b * c;
    let rotation = (-b).atan2(a).to_degrees();
    let rotation = if rotation <= -180.0 + 1e-9 { 180.0 } else { rotation };
    Decomposed { scale_x: sx, scale_y: det / sx, rotation, shear: ((a * c + b * d) / det).atan().to_degrees() }
}

/// The linear transform with these values (no translation); `decompose` inverts it.
pub fn compose(v: &Decomposed) -> Affine {
    Affine::rotate(-v.rotation.to_radians())
        * Affine::new([1.0, 0.0, v.shear.to_radians().tan(), 1.0, 0.0, 0.0])
        * Affine::scale_non_uniform(v.scale_x, v.scale_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for (sx, sy, r, k) in [(1.0, 1.0, 0.0, 0.0), (2.0, 0.5, 30.0, 0.0), (1.5, -1.0, -120.0, 20.0), (0.25, 3.0, 180.0, -45.0)] {
            let v = Decomposed { scale_x: sx, scale_y: sy, rotation: r, shear: k };
            let back = decompose(compose(&v));
            for (x, y) in [(back.scale_x, sx), (back.scale_y, sy), (back.rotation, r), (back.shear, k)] {
                assert!((x - y).abs() < 1e-9, "{v:?} → {back:?}");
            }
        }
        // transform.rotate's convention: rotate(-θ) is θ counter-clockwise.
        assert!((decompose(Affine::rotate(-0.5)).rotation - 0.5f64.to_degrees()).abs() < 1e-9);
    }
}

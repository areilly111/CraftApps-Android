//! The 256-colour index palette CAD apps share, generated rather than tabulated: indices 1–9 are
//! the standard colours, 10–249 run through 24 hues at five brightnesses each in a full and a
//! half-saturated tone, and 250–255 are greys.

use crate::ColorDepth;

/// The nine standard colours (index 7 shows white on a dark background and black on paper).
const STANDARD: [[u8; 3]; 9] =
    [[255, 0, 0], [255, 255, 0], [0, 255, 0], [0, 255, 255], [0, 0, 255], [255, 0, 255], [255, 255, 255], [128, 128, 128], [192, 192, 192]];

/// The brightness of the five shades of each hue.
const SHADES: [f64; 5] = [1.0, 0.65, 0.5, 0.3, 0.15];

/// The RGB of colour index `i` (1–255; anything else is index 7's white).
pub fn aci_rgb(i: u8) -> [u8; 3] {
    match i {
        1..=9 => STANDARD[usize::from(i - 1)],
        10..=249 => {
            let hue = f64::from(i / 10 - 1) * 15.0;
            let k = i % 10;
            let v = SHADES[usize::from(k / 2)];
            let pastel = k % 2 == 1;
            hue_rgb(hue).map(|f| {
                let f = if pastel { f + (1.0 - f) / 2.0 } else { f };
                (255.0 * v * f).floor() as u8
            })
        }
        250..=255 => {
            let g = (51.0 + 204.0 * f64::from(i - 250) / 5.0).floor() as u8;
            [g; 3]
        }
        _ => STANDARD[6],
    }
}

/// The fully saturated, full-brightness colour of `hue` degrees as 0–1 channels.
fn hue_rgb(hue: f64) -> [f64; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u8 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// The indices a colour depth may use (true colour: all of them, for the fallback index).
fn candidates(depth: ColorDepth) -> Box<dyn Iterator<Item = u8>> {
    match depth {
        ColorDepth::Aci8 => Box::new(1..=8),
        ColorDepth::Aci16 => Box::new((1..=9).chain([30]).chain(250..=255)),
        ColorDepth::Aci256 | ColorDepth::True => Box::new(1..=255),
    }
}

/// The index of `depth` nearest to `rgb` (squared RGB distance; ties go to the lower index).
/// Index 7 stands for both white and black, as CAD apps draw it in the background's contrast.
pub fn nearest_aci(rgb: [u8; 3], depth: ColorDepth) -> u8 {
    let dist = |c: [u8; 3]| rgb.iter().zip(c).map(|(a, b)| (i32::from(*a) - i32::from(b)).pow(2)).sum::<i32>();
    candidates(depth).min_by_key(|i| if *i == 7 { dist([255; 3]).min(dist([0; 3])) } else { dist(aci_rgb(*i)) }).unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_matches_its_known_entries() {
        let known: [(u8, [u8; 3]); 14] = [
            (1, [255, 0, 0]),
            (10, [255, 0, 0]),
            (11, [255, 127, 127]),
            (12, [165, 0, 0]),
            (13, [165, 82, 82]),
            (17, [76, 38, 38]),
            (19, [38, 19, 19]),
            (20, [255, 63, 0]),
            (21, [255, 159, 127]),
            (30, [255, 127, 0]),
            (140, [0, 191, 255]),
            (250, [51, 51, 51]),
            (252, [132, 132, 132]),
            (255, [255, 255, 255]),
        ];
        for (i, rgb) in known {
            assert_eq!(aci_rgb(i), rgb, "index {i}");
        }
    }

    #[test]
    fn nearest_index_by_depth() {
        assert_eq!(nearest_aci([255, 0, 0], ColorDepth::Aci8), 1);
        assert_eq!(nearest_aci([0, 0, 0], ColorDepth::Aci256), 7, "black is index 7");
        assert_eq!(nearest_aci([255, 255, 255], ColorDepth::Aci16), 7);
        assert_eq!(nearest_aci([255, 127, 0], ColorDepth::Aci256), 30);
        assert_eq!(nearest_aci([255, 127, 0], ColorDepth::Aci16), 30);
        assert_eq!(nearest_aci([255, 127, 0], ColorDepth::Aci8), 1, "8 colours: red is nearest");
        assert_eq!(nearest_aci([60, 60, 60], ColorDepth::Aci16), 250);
        assert_eq!(nearest_aci([100, 100, 100], ColorDepth::Aci8), 8);
        assert_eq!(nearest_aci([60, 60, 60], ColorDepth::Aci8), 7, "dark greys go to black");
    }
}

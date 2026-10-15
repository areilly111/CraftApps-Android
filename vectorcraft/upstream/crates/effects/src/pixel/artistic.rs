//! Effect › Artistic. Original filters built from public behaviour documentation:
//! <https://helpx.adobe.com/illustrator/using/summary-effects.html> and
//! <https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html>.
//! These are visual approximations, not reproductions of a proprietary algorithm.

use super::PixelSpace;
use super::pixelate::{Place, cell, rand};
use super::texture::{luma, premultiply, straight};
use vectorcraft_geom::Point;

const SALT_FILM: u64 = 256;

/// Keep the last cell's four corner values: magnified adjacent pixels often share a cell.
/// The cache changes only cost, never the field, and needs constant storage per raster.
#[derive(Default)]
struct CellNoise {
    last: Option<(i64, i64, [f64; 4])>,
}

impl CellNoise {
    fn at(&mut self, salt: u64, q: Point) -> f64 {
        let (Some(i), Some(j)) = (cell(q.x), cell(q.y)) else { return 0.0 };
        let corners = match self.last {
            Some((x, y, values)) if x == i && y == j => values,
            _ => {
                let at = |di: i64, dj: i64| 2.0 * rand(salt, i.wrapping_add(di), j.wrapping_add(dj), 0) - 1.0;
                let values = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                self.last = Some((i, j, values));
                values
            }
        };
        let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
        let (fx, fy) = (smooth(q.x - i as f64), smooth(q.y - j as f64));
        let [a, b, c, d] = corners;
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        // Divide by the root sum of squared interpolation weights so corners and cell
        // interiors have the same variance, rather than imprinting a contrast grid.
        let norm = ((fx * fx + (1.0 - fx).powi(2)) * (fy * fy + (1.0 - fy).powi(2))).sqrt();
        (top + (bottom - top) * fy) / norm
    }
}

#[derive(Default)]
struct FilmNoise {
    low: CellNoise,
    high: CellNoise,
}

impl FilmNoise {
    fn at(&mut self, q: Point) -> f32 {
        // Fixed independent rotations (0.6 and -0.35 radians) and a second-octave offset
        // keep the two lattices from lining up with pixel rows or with each other.
        let low = Point::new(0.8253356149096783 * q.x - 0.5646424733950354 * q.y, 0.5646424733950354 * q.x + 0.8253356149096783 * q.y);
        let high = Point::new(
            2.0 * (0.9393727128473789 * q.x + 0.34289780745545134 * q.y) + 0.37,
            2.0 * (-0.34289780745545134 * q.x + 0.9393727128473789 * q.y) - 0.37,
        );
        // Normalising the blend can exceed the corner range, so bound the final field.
        (0.8 * (0.65 * self.low.at(SALT_FILM, low) + 0.35 * self.high.at(SALT_FILM + 1, high))).clamp(-1.0, 1.0) as f32
    }
}

/// Bounded footprint sampling during minification, with attenuation for very large pixels.
/// Up to four samples approximate area filtering, not exact integration of every grain.
fn grain_at(place: &Place, field: &mut FilmNoise, x: usize, y: usize) -> f32 {
    if place.px <= 0.5 {
        return field.at(place.at(x, y));
    }
    let mut sample = |dx, dy| field.at(place.to_rel * Point::new(x as f64 + dx, y as f64 + dy));
    let diagonal = 0.5 * (sample(0.25, 0.25) + sample(0.75, 0.75));
    // Blend sampling modes rather than changing the texture's strength suddenly at a zoom.
    if place.px < 0.75 {
        let t = ((place.px - 0.5) / 0.25) as f32;
        return field.at(place.at(x, y)) * (1.0 - t) + diagonal * t;
    }
    if place.px <= 1.0 {
        return diagonal;
    }
    let other = 0.5 * (sample(0.75, 0.25) + sample(0.25, 0.75));
    let t = (0.5 * ((place.px - 1.0) / 0.5).min(1.0)) as f32;
    // The four samples already average about a two-point footprint. Only wider pixels need
    // extra attenuation; /px alone suppressed coarse grain twice (checked against dense sampling).
    (diagonal * (1.0 - t) + other * t) / (place.px / 2.0).max(1.0) as f32
}

/// Film Grain: monochrome grain in shadows and midtones, with brighter highlights as
/// `highlight` (0..20) widens their area. `intensity` (0..10) increases the highlight lift and
/// quiets highlight grain, including when the Highlight Area is zero;
/// `grain` (0..20) sets the noise strength. The smooth grain field lies relative to the object's
/// bounds' centre, following translations and redraws; changing the bounds shifts the pattern.
/// Its scale is in document units; minification uses bounded approximate footprint filtering.
/// Coverage is preserved, including fully transparent pixels. Constant work and no allocations.
pub(super) fn film_grain(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, grain: f64, highlight: f64, intensity: f64) {
    if ![grain, highlight, intensity].iter().all(|v| v.is_finite()) {
        return;
    }
    let Some(place) = Place::new(space, w, h) else { return };
    let grain = (grain / 20.0).clamp(0.0, 1.0) as f32;
    let highlight = (highlight / 20.0).clamp(0.0, 1.0) as f32;
    let intensity = (intensity / 10.0).clamp(0.0, 1.0) as f32;
    if grain == 0.0 && (intensity == 0.0 || highlight == 0.0) {
        return;
    }
    let mut field = FilmNoise::default();
    place.each(px, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        let c = straight(*p);
        let brightness = luma(c);
        // Wider highlight areas include more midtones, without lifting black or growing a halo.
        let threshold = 0.85 - 0.6 * highlight;
        let t = ((brightness - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
        let light = t * t * (3.0 - 2.0 * t) * highlight * intensity * 0.65;
        // Keep the grain strength even across shadows and midtones, then quiet it smoothly
        // above mid-grey to a quarter-strength floor at full intensity, before highlight
        // headroom. This keeps a softer grain on cream. Black takes light speckles after
        // clipping; white stays clean.
        // This tonal curve is an interpretation of the public description, not a measured
        // proprietary transfer function. Shared offsets stay neutral until a channel clips.
        let upper = ((brightness - 0.5) * 2.0).clamp(0.0, 1.0);
        let upper = upper * upper * (3.0 - 2.0 * upper);
        let amplitude = (0.4 * (1.0 - upper * (0.5 + 0.5 * intensity))).max(0.1).min(1.0 - brightness);
        let strength = amplitude * grain * (1.0 - light);
        // With no highlight lift the starting premultiplied channels are whole bytes. A
        // bounded offset below half a byte cannot change rounding, so avoid hashing it.
        // Leave a small margin for straight/premultiplied floating-point roundoff.
        if light == 0.0 && strength * f32::from(p[3]) < 0.45 {
            return;
        }
        let noise = if strength > 0.0 { grain_at(&place, &mut field, x, y) * strength } else { 0.0 };
        let c = c.map(|v| v + (1.0 - v) * light + noise);
        *p = premultiply(c, f32::from(p[3]) / 255.0);
    });
}

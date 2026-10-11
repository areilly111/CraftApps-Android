//! Colours and gradients of the editing data.
//!
//! - Colours: `g`/`G` grey (1 is white), `k`/`K` CMYK, `x`/`X` a named CMYK colour at a tint
//!   (`c m y k (name) tint`; tint 0 is the full colour), `Xa`/`XA` RGB (`c m y k r g b`, the CMYK
//!   values its equivalent), `Xx`/`XX` a named colour of either model
//!   (`c m y k [r g b] (name) tint type`). Lower case paints fills, upper case strokes.
//! - Gradient definitions: `(name) type stops Bd`, its stops as `colour style [opacity 6]
//!   midpoint ramp Bs` (style 0 grey, 1 CMYK, 2 RGB with its CMYK, 3 and 4 named colours), `BD`.
//! - A gradient on an object: `[flag] Bb` (1: the stroke), `flag (name) x y angle length a b c d
//!   tx ty Bg`, the gradient matrix `a b c d tx ty Bm`, the focal point `x y angle length Bh`,
//!   `flag BB`.

use std::collections::HashMap;

use kurbo::{Affine, Point, Vec2};
use vectorcraft_color::gradient::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
use vectorcraft_color::{Color, Paint};

use super::obj::V;

/// Largest number of stops read in one gradient.
const MAX_STOPS: usize = 256;

/// A colour read with the swatch it names (spot and global colours).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Named {
    pub name: String,
    /// The full colour.
    pub color: Color,
    pub tint: f32,
}

/// The paint of a colour operator's operands (`op`: its name, lower case or not), with the named
/// colour it uses. `None` when the operands don't fit.
pub(super) fn color_op(op: &str, vals: &[V]) -> Option<(Paint, Option<Named>)> {
    let nums: Vec<f32> = vals.iter().filter_map(V::num).map(|v| v as f32).collect();
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let last = |n: usize| nums.get(nums.len().checked_sub(n)?..);
    match op.to_ascii_lowercase().as_str() {
        "g" => Some((Paint::solid(Color::gray(1.0 - unit(*nums.last()?))), None)),
        "k" => {
            let [c, m, y, k] = last(4)? else { return None };
            Some((Paint::solid(Color::cmyk(unit(*c), unit(*m), unit(*y), unit(*k))), None))
        }
        "xa" => {
            let [r, g, b] = last(3)? else { return None };
            Some((Paint::solid(Color::rgb(unit(*r), unit(*g), unit(*b))), None))
        }
        "x" => named(vals, Some(false)),
        "xx" => named(vals, None),
        _ => None,
    }
}

/// `c m y k (name) tint x` or `c m y k [r g b] (name) tint type Xx` (type 1: RGB); `rgb` forces
/// the model.
fn named(vals: &[V], rgb: Option<bool>) -> Option<(Paint, Option<Named>)> {
    let at = vals.iter().rposition(|v| matches!(v, V::Str(_)))?;
    let name = vals.get(at)?.text()?;
    let before: Vec<f32> = vals.get(..at)?.iter().filter_map(V::num).map(|v| v as f32).collect();
    let after: Vec<f32> = vals.get(at + 1..)?.iter().filter_map(V::num).map(|v| v as f32).collect();
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let tint = 1.0 - unit(*after.first()?);
    let n = before.len();
    let color = if rgb.unwrap_or(after.get(1).is_some_and(|t| *t == 1.0)) {
        let [r, g, b] = before.get(n.checked_sub(3)?..)? else { return None };
        Color::rgb(unit(*r), unit(*g), unit(*b))
    } else {
        // The CMYK values come first (an RGB equivalent may follow them).
        let from = if n >= 7 { n - 7 } else { n.checked_sub(4)? };
        let [c, m, y, k] = before.get(from..from + 4)? else { return None };
        Color::cmyk(unit(*c), unit(*m), unit(*y), unit(*k))
    };
    let paint = Paint::Solid { color: color.tinted(tint), swatch: Some(name.clone()), tint };
    Some((paint, Some(Named { name, color, tint })))
}

/// The colour of a gradient stop's colour values and style.
fn stop_color(style: f64, comps: &[V]) -> Option<(Color, Option<Named>)> {
    let nums: Vec<f32> = comps.iter().filter_map(V::num).map(|v| v as f32).collect();
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let tail = |n: usize| nums.get(nums.len().checked_sub(n)?..);
    Some(match style as i64 {
        0 => (Color::gray(1.0 - unit(*nums.last()?)), None),
        1 => {
            let [c, m, y, k] = tail(4)? else { return None };
            (Color::cmyk(unit(*c), unit(*m), unit(*y), unit(*k)), None)
        }
        2 => {
            let [r, g, b] = tail(3)? else { return None };
            (Color::rgb(unit(*r), unit(*g), unit(*b)), None)
        }
        _ => {
            let (paint, named) = named(comps, Some(style as i64 == 4))?;
            (paint.color()?, named)
        }
    })
}

/// A gradient definition being read.
#[derive(Debug)]
pub(super) struct GradientDef {
    pub name: String,
    pub gradient: Gradient,
}

impl GradientDef {
    /// `(name) type stops Bd`.
    pub fn begin(vals: &[V]) -> Option<Self> {
        let name = vals.iter().find_map(|v| if let V::Str(_) = v { v.text() } else { None })?;
        let nums: Vec<f64> = vals.iter().filter_map(V::num).collect();
        let kind = if nums.first().copied() == Some(1.0) { GradientKind::Radial } else { GradientKind::Linear };
        Some(Self { name, gradient: Gradient { kind, stops: vec![] } })
    }

    /// A stop (`Bs`).
    pub fn stop(&mut self, vals: &[V], spots: &mut Vec<Named>) {
        if self.gradient.stops.len() >= MAX_STOPS {
            return;
        }
        let n = vals.len();
        let num = |i: usize| vals.get(i).and_then(V::num);
        let (Some(ramp), Some(mid)) = (n.checked_sub(1).and_then(num), n.checked_sub(2).and_then(num)) else { return };
        // Newer stops have an opacity and the marker 6 before the midpoint.
        let newer = n >= 5 && n.checked_sub(3).and_then(num) == Some(6.0);
        let (style_at, opacity) = if newer { (n.checked_sub(5), n.checked_sub(4).and_then(num).unwrap_or(1.0)) } else { (n.checked_sub(3), 1.0) };
        let (Some(style_at), Some(style)) = (style_at, style_at.and_then(num)) else { return };
        let Some((color, named)) = stop_color(style, vals.get(..style_at).unwrap_or_default()) else { return };
        let mut stop = GradientStop::new((ramp / 100.0).clamp(0.0, 1.0) as f32, color);
        stop.opacity = opacity.clamp(0.0, 1.0) as f32;
        stop.midpoint = (mid / 100.0).clamp(0.13, 0.87) as f32;
        if let Some(n) = named {
            stop.set_color(n.color.tinted(n.tint), Some((n.name.clone(), n.tint)));
            spots.push(n);
        }
        self.gradient.stops.push(stop);
    }

    /// The finished gradient (`BD`): stops in order of their ramp points.
    pub fn finish(mut self) -> Option<(String, Gradient)> {
        if self.gradient.stops.is_empty() {
            return None;
        }
        self.gradient.stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        Some((self.name, self.gradient))
    }
}

/// A gradient instance on an object, between `Bb` and `BB`.
#[derive(Clone, Debug, Default)]
pub(super) struct Instance {
    pub stroke: bool,
    pub name: Option<String>,
    origin: Point,
    angle: f64,
    length: f64,
    /// `Bg`'s own matrix.
    bg: Affine,
    /// The gradient matrix (`Bm`), when there is one.
    bm: Option<Affine>,
    /// The focal point's offset from the origin (`Bh`).
    hilight: Vec2,
}

fn affine(n: &[f64]) -> Option<Affine> {
    let [a, b, c, d, e, f] = n else { return None };
    let m = Affine::new([*a, *b, *c, *d, *e, *f]);
    (m.as_coeffs().iter().all(|v| v.is_finite())).then_some(m)
}

impl Instance {
    pub fn begin(vals: &[V]) -> Self {
        let stroke = vals.iter().filter_map(V::num).next_back() == Some(1.0);
        Self { stroke, length: 1.0, ..Self::default() }
    }

    /// `flag (name) x y angle length a b c d tx ty Bg`.
    pub fn bg(&mut self, vals: &[V]) {
        let Some(at) = vals.iter().rposition(|v| matches!(v, V::Str(_))) else { return };
        self.name = vals.get(at).and_then(V::text);
        let n: Vec<f64> = vals.get(at + 1..).unwrap_or_default().iter().filter_map(V::num).collect();
        if let [x, y, angle, length, m @ ..] = n.as_slice() {
            self.origin = Point::new(*x, *y);
            self.angle = *angle;
            self.length = *length;
            self.bg = affine(m).unwrap_or(Affine::IDENTITY);
        }
    }

    /// `a b c d tx ty Bm`.
    pub fn bm(&mut self, vals: &[V]) {
        let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
        self.bm = n.get(n.len().saturating_sub(6)..).and_then(affine).filter(|m| m.determinant().abs() > 1e-12);
    }

    /// `x y angle length Bh`.
    pub fn bh(&mut self, vals: &[V]) {
        let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
        if let [x, y, ..] = n.as_slice() {
            self.hilight = Vec2::new(*x, *y);
        }
    }

    /// The paint, through `to_doc` (art space → the document), when the gradient is defined.
    pub fn paint(&self, defs: &HashMap<String, Gradient>, to_doc: Affine) -> Option<Paint> {
        let name = self.name.as_ref()?;
        let gradient = defs.get(name)?.clone();
        let m = to_doc * self.bm.unwrap_or(Affine::IDENTITY) * self.bg;
        let a = self.angle.to_radians();
        let dir = Vec2::new(a.cos(), a.sin()) * self.length;
        let start = m * self.origin;
        let end = m * (self.origin + dir);
        if !(start.is_finite() && end.is_finite()) || start.distance(end) < 1e-9 {
            return None;
        }
        // Across the vector, for an elliptical radial gradient.
        let across = m * (self.origin + Vec2::new(-dir.y, dir.x));
        let aspect = (across.distance(start) / start.distance(end)).clamp(0.01, 100.0);
        let focal = (self.hilight.hypot() > 1e-9).then(|| m * (self.origin + self.hilight)).filter(|p| p.is_finite());
        let radial = gradient.kind == GradientKind::Radial;
        let geom = GradientGeom { start, end, aspect: if radial { aspect } else { 1.0 }, focal: if radial { focal } else { None } };
        let angle = -(end - start).atan2().to_degrees();
        Some(Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), angle, swatch: Some(name.clone()), ..GradientPaint::new(gradient) })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(v: &[f64]) -> Vec<V> {
        v.iter().map(|n| V::Num(*n)).collect()
    }

    #[test]
    fn colours() {
        let (p, _) = color_op("g", &nums(&[0.7])).unwrap();
        assert!(matches!(p.color(), Some(Color::Gray { k }) if (k - 0.3).abs() < 1e-6));
        let (p, _) = color_op("XA", &nums(&[0.0, 0.99, 1.0, 0.0, 1.0, 0.0, 0.0])).unwrap();
        assert_eq!(p.color(), Some(Color::rgb(1.0, 0.0, 0.0)));
        let mut v = nums(&[0.0, 0.5, 1.0, 0.0]);
        v.push(V::Str(b"MySpot".to_vec()));
        v.push(V::Num(0.4));
        let (p, named) = color_op("x", &v).unwrap();
        let named = named.unwrap();
        assert_eq!(named.name, "MySpot");
        assert!((named.tint - 0.6).abs() < 1e-6);
        assert!(matches!(p, Paint::Solid { swatch: Some(_), .. }));
        assert!(color_op("k", &nums(&[1.0])).is_none());
    }

    #[test]
    fn gradient_definition_and_instance() {
        let mut d = GradientDef::begin(&[V::Str(b"G".to_vec()), V::Num(1.0), V::Num(2.0)]).unwrap();
        let mut spots = vec![];
        d.stop(&nums(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.5, 6.0, 30.0, 100.0]), &mut spots);
        d.stop(&nums(&[0.81, 0.0, 1.0, 6.0, 50.0, 0.0]), &mut spots);
        let (name, g) = d.finish().unwrap();
        assert_eq!(name, "G");
        assert_eq!(g.kind, GradientKind::Radial);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[1].opacity, 0.5);
        assert!(matches!(g.stops[0].color, Color::Gray { k } if (k - 0.19).abs() < 1e-6));
        let defs = HashMap::from([(name, g)]);
        let mut i = Instance::begin(&[]);
        let mut bg = vec![V::Num(1.0), V::Str(b"G".to_vec())];
        bg.extend(nums(&[0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
        i.bg(&bg);
        i.bm(&nums(&[50.0, 0.0, 0.0, -50.0, 100.0, 100.0]));
        let Some(Paint::Gradient(gp)) = i.paint(&defs, Affine::IDENTITY) else { panic!() };
        let geom = gp.geom.unwrap();
        assert_eq!(geom.start, Point::new(100.0, 100.0));
        assert_eq!(geom.end, Point::new(150.0, 100.0));
        assert!((geom.aspect - 1.0).abs() < 1e-9);
        assert!(i.paint(&HashMap::new(), Affine::IDENTITY).is_none());
    }
}

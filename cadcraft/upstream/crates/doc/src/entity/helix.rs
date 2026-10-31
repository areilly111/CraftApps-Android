//! Helices (HELIX): a spiral round a vertical axis from a base radius to a top radius, drawn as
//! the cubic spline through points on it (DXF Reference: HELIX, a SPLINE with AcDbHelix data).

use cadcraft_geom::{FitOptions, KnotParam, Spline as GSpline, TAU, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Most turns a helix has (as the reference program allows).
pub const MAX_HELIX_TURNS: f64 = 500.0;
/// Points on the helix per turn that its spline runs through.
const POINTS_PER_TURN: f64 = 8.0;

/// What stays fixed when the helix is edited (DXF group 280).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelixConstrain {
    TurnHeight,
    Turns,
    #[default]
    Height,
}

impl HelixConstrain {
    pub fn to_dxf(self) -> i64 {
        match self {
            HelixConstrain::TurnHeight => 0,
            HelixConstrain::Turns => 1,
            HelixConstrain::Height => 2,
        }
    }
    pub fn from_dxf(v: i64) -> HelixConstrain {
        match v {
            0 => HelixConstrain::TurnHeight,
            1 => HelixConstrain::Turns,
            _ => HelixConstrain::Height,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Helix {
    /// Centre of the base (the axis base point).
    pub center: Vec3,
    pub base_radius: f64,
    pub top_radius: f64,
    /// Along the axis (+Z); negative goes down.
    pub height: f64,
    pub turns: f64,
    /// Angle of the start point from the centre, radians.
    #[serde(default)]
    pub start_angle: f64,
    /// Twist: counter-clockwise seen from above (right-handed).
    #[serde(default = "yes")]
    pub ccw: bool,
    #[serde(default)]
    pub constrain: HelixConstrain,
}

fn yes() -> bool {
    true
}

impl Helix {
    /// The values made safe: finite, radii >= 0, turns within (0, 500].
    pub fn sanitized(mut self) -> Helix {
        let fin = |v: f64, d: f64| if v.is_finite() { v } else { d };
        self.center = Vec3::new(fin(self.center.x, 0.0), fin(self.center.y, 0.0), fin(self.center.z, 0.0));
        self.base_radius = fin(self.base_radius, 1.0).abs().min(1e12);
        self.top_radius = fin(self.top_radius, self.base_radius).abs().min(1e12);
        self.height = fin(self.height, 0.0).clamp(-1e12, 1e12);
        self.turns = fin(self.turns, 3.0).clamp(1e-3, MAX_HELIX_TURNS);
        self.start_angle = fin(self.start_angle, 0.0);
        self
    }

    /// Distance between turns.
    pub fn turn_height(&self) -> f64 {
        if self.turns > 0.0 { self.height / self.turns } else { 0.0 }
    }

    /// The point at `t` (0 = start, 1 = end) along the helix.
    pub fn point_at(&self, t: f64) -> Vec3 {
        let sign = if self.ccw { 1.0 } else { -1.0 };
        let a = self.start_angle + sign * TAU * self.turns * t;
        let r = self.base_radius + (self.top_radius - self.base_radius) * t;
        let p = Vec2::polar(self.center.xy(), r, a);
        Vec3::new(p.x, p.y, self.center.z + self.height * t)
    }

    pub fn start_point(&self) -> Vec3 {
        self.point_at(0.0)
    }
    pub fn end_point(&self) -> Vec3 {
        self.point_at(1.0)
    }

    /// The points the spline runs through, evenly spaced in angle.
    pub fn fit_points(&self) -> Vec<Vec3> {
        let turns = if self.turns.is_finite() { self.turns.clamp(1e-3, MAX_HELIX_TURNS) } else { 1.0 };
        let n = ((turns * POINTS_PER_TURN).ceil() as usize).max(4);
        (0..=n).map(|i| self.point_at(i as f64 / n as f64)).collect()
    }

    /// The helix as a cubic spline seen from above, and the height of each control point: the
    /// spline through [`Helix::fit_points`] with evenly spaced knots, so the height, linear in
    /// the parameter, is exact at each control point's Greville abscissa.
    pub fn spline3(&self) -> (GSpline, Vec<f64>) {
        let fit = self.fit_points();
        let xy: Vec<Vec2> = fit.iter().map(|p| p.xy()).collect();
        let sp = GSpline::fit_with(&xy, false, FitOptions { knots: KnotParam::Uniform, ..FitOptions::default() });
        let (lo, hi) = sp.domain();
        let span = if hi - lo > 1e-12 { hi - lo } else { 1.0 };
        let p = sp.degree.max(1);
        let z = (0..sp.control.len())
            .map(|j| {
                let g = (1..=p).map(|k| sp.knots.get(j + k).copied().unwrap_or(hi)).sum::<f64>() / p as f64;
                self.center.z + self.height * ((g - lo) / span).clamp(0.0, 1.0)
            })
            .collect();
        (sp, z)
    }

    /// The helix seen from above.
    pub fn spline(&self) -> GSpline {
        self.spline3().0
    }
}

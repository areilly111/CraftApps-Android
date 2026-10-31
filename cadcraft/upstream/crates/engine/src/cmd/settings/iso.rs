//! Isometric drafting commands: ISODRAFT (isometric snap style on with a plane, or back to
//! orthographic), ISOPLANE (the current plane; F5 cycles it) and ELLIPSE's Isocircle option.

use cadcraft_geom::{Ellipse, Vec2};
use serde_json::{Value, json};

use super::super::{bad, f64_req, point_req, str_param};
use crate::snap::iso::{self, IsoPlane};
use crate::{Accept, EngineError, Input, Interactive, Prompt, Result, Session, Step};

fn report(s: &Session) -> Value {
    let st = &s.settings;
    json!({
        "on": st.isodraft,
        "plane": st.isoplane.name(),
        "snapstyl": i32::from(st.isodraft),
        "snapisopair": st.isoplane.index(),
        "message": if st.isodraft { format!("<Isodraft {}>", st.isoplane.name()) } else { "<Isodraft off>".to_string() },
    })
}

/// The `plane` parameter: a plane, `orthographic` (`None` inside), or absent.
fn plane_param(cmd: &str, p: &Value) -> Result<Option<Option<IsoPlane>>> {
    let Some(t) = str_param(p, "plane") else { return Ok(None) };
    if !t.trim().is_empty() && "orthographic".starts_with(&t.trim().to_ascii_lowercase()) {
        return Ok(Some(None));
    }
    IsoPlane::parse(t).map(|pl| Some(Some(pl))).ok_or_else(|| bad(cmd, "`plane` is left, top, right or orthographic"))
}

/// ISODRAFT `{on?: bool, plane?}`: a plane turns isometric drafting on in that plane,
/// `orthographic` turns it off; `on` sets it; neither toggles it (the status-bar button).
pub(super) fn run_isodraft(s: &mut Session, p: &Value) -> Result<Value> {
    match plane_param("isodraft", p)? {
        Some(Some(pl)) => {
            s.settings.isodraft = true;
            s.settings.isoplane = pl;
        }
        Some(None) => s.settings.isodraft = false,
        None => s.settings.isodraft = p.get("on").and_then(Value::as_bool).unwrap_or(!s.settings.isodraft),
    }
    s.touch();
    Ok(report(s))
}

/// ISOPLANE `{plane?}`: set the current isometric plane, or switch to the next one (F5).
pub(super) fn run_isoplane(s: &mut Session, p: &Value) -> Result<Value> {
    let pl = match plane_param("isoplane", p)? {
        Some(Some(pl)) => pl,
        Some(None) => return Err(bad("isoplane", "`plane` is left, top or right")),
        None => s.settings.isoplane.next(),
    };
    s.settings.isoplane = pl;
    s.touch();
    let mut v = report(s);
    if let Some(m) = v.get_mut("message") {
        *m = json!(format!("<Isoplane {}>", pl.name()));
    }
    Ok(v)
}

/// ISODRAFT at the command line: Orthographic or a plane; Enter toggles.
#[derive(Default)]
pub(super) struct IsodraftM;

impl Interactive for IsodraftM {
    fn name(&self) -> &'static str {
        "ISODRAFT"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let st = &s.settings;
        let default = if st.isodraft { "Orthographic" } else { st.isoplane.name() };
        Prompt::new("Drafting mode?", Accept { enter: true, ..Accept::default() }).kw(&["Orthographic", "Left", "Top", "Right"]).default(default)
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let p = match i {
            Input::Keyword(k) | Input::Text(k) => json!({ "plane": k }),
            Input::Enter => json!({}),
            Input::Cancel => return Ok(Step::Cancel),
            _ => return Ok(Step::Continue),
        };
        let v = run_isodraft(s, &p)?;
        if let Some(m) = v.get("message").and_then(Value::as_str) {
            s.echo(m.to_string());
        }
        Ok(Step::Done)
    }
}

/// ISOPLANE at the command line: a plane; Enter switches to the next one.
#[derive(Default)]
pub(super) struct IsoplaneM;

impl Interactive for IsoplaneM {
    fn name(&self) -> &'static str {
        "ISOPLANE"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        Prompt::new("Isometric plane?", Accept { enter: true, ..Accept::default() })
            .kw(&["Left", "Top", "Right"])
            .default(s.settings.isoplane.next().name())
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        let p = match i {
            Input::Keyword(k) | Input::Text(k) => json!({ "plane": k }),
            Input::Enter => json!({}),
            Input::Cancel => return Ok(Step::Cancel),
            _ => return Ok(Step::Continue),
        };
        let v = run_isoplane(s, &p)?;
        if let Some(m) = v.get("message").and_then(Value::as_str) {
            s.echo(m.to_string());
        }
        Ok(Step::Done)
    }
}

/// ELLIPSE `{isocircle: {center, radius | diameter, plane?}}`: an isometric circle in `plane`
/// (default: the current isometric plane).
pub(crate) fn run_isocircle(s: &mut Session, p: &Value) -> Result<Value> {
    let c = p.get("isocircle").ok_or_else(|| bad("ellipse", "`isocircle` is required"))?;
    let center = point_req("ellipse", c, "center")?;
    let r = match c.get("diameter") {
        Some(_) => f64_req("ellipse", c, "diameter")? / 2.0,
        None => f64_req("ellipse", c, "radius")?,
    };
    let plane = match plane_param("ellipse", c)? {
        Some(Some(pl)) => pl,
        Some(None) => return Err(bad("ellipse", "an isocircle's `plane` is left, top or right")),
        None => s.settings.isoplane,
    };
    let e = iso::isocircle(plane, center, r).ok_or_else(|| bad("ellipse", "the isocircle needs a finite center and a positive radius"))?;
    let h = s.add_entity(ellipse_entity(&e))?;
    Ok(json!({ "handle": h.hex(), "plane": plane.name() }))
}

pub(crate) fn ellipse_entity(e: &Ellipse) -> cadcraft_doc::EntityKind {
    cadcraft_doc::EntityKind::Ellipse(cadcraft_doc::Ellipse {
        center: e.center.to3(0.0),
        major: e.major.to3(0.0),
        ratio: e.ratio,
        start: 0.0,
        end: cadcraft_geom::TAU,
    })
}

/// ELLIPSE's Isocircle option: the center, then the radius (or diameter) in the current
/// isometric plane. [`IsoCircle::input`] gives the finished ellipse.
#[derive(Default)]
pub(crate) struct IsoCircle {
    center: Option<Vec2>,
    diameter: bool,
}

impl IsoCircle {
    pub(crate) fn prompt(&self) -> Prompt {
        match self.center {
            None => Prompt::new("Specify center of isocircle", Accept::POINT),
            Some(c) if self.diameter => Prompt::new("Specify diameter of isocircle", Accept::POINT_OR_NUMBER).base(c),
            Some(c) => Prompt::new("Specify radius of isocircle", Accept::POINT_OR_NUMBER).kw(&["Diameter"]).base(c),
        }
    }

    /// The ellipse once the input completes it.
    pub(crate) fn input(&mut self, s: &Session, i: Input) -> Result<Option<Ellipse>> {
        let plane = s.settings.isoplane;
        let Some(c) = self.center else {
            if let Input::Point(p) = i {
                self.center = Some(p);
            }
            return Ok(None);
        };
        let r = match i {
            Input::Keyword(k) if k == "Diameter" => {
                self.diameter = true;
                return Ok(None);
            }
            Input::Point(p) => iso::iso_radius(plane, c, p).map(|r| if self.diameter { r / 2.0 } else { r }),
            Input::Text(t) => {
                let d = crate::units::parse_distance(&t).ok_or_else(|| EngineError::Other("Requires a distance.".into()))?;
                Some(if self.diameter { d / 2.0 } else { d })
            }
            _ => return Ok(None),
        };
        Ok(r.and_then(|r| iso::isocircle(plane, c, r)))
    }

    pub(crate) fn preview(&self, s: &Session, cursor: Vec2) -> Option<Ellipse> {
        let c = self.center?;
        let plane = s.settings.isoplane;
        let r = iso::iso_radius(plane, c, cursor)?;
        iso::isocircle(plane, c, if self.diameter { r / 2.0 } else { r })
    }
}

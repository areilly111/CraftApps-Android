//! DIMTEDIT at the command line: "Select dimension", then a new text location or
//! Left/Right/Center/Home/Angle. Each answer runs the JSON form on the picked dimension.

use cadcraft_doc::{EntityKind, Handle};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{dims_of, run_dimtedit, v3};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

#[derive(Default)]
pub(super) struct TEditM {
    dim: Option<Handle>,
    asking_angle: bool,
}

impl TEditM {
    /// Align Text > Angle: select the dimension (or use the preselected one), then ask for the angle.
    pub(super) fn angle() -> Self {
        Self { dim: None, asking_angle: true }
    }

    fn run(&self, s: &mut Session, mut p: Value) -> Result<Step> {
        let Some(h) = self.dim else { return Ok(Step::Cancel) };
        if let Some(o) = p.as_object_mut() {
            o.insert("handles".into(), json!([h.hex()]));
        }
        run_dimtedit(s, &p)?;
        Ok(Step::Done)
    }
}

impl Interactive for TEditM {
    fn name(&self) -> &'static str {
        "DIMTEDIT"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        // A single preselected dimension is taken at once.
        if s.settings.pickfirst
            && let [h] = dims_of(s, &s.selection())[..]
        {
            self.dim = Some(h);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        if self.dim.is_none() {
            Prompt::new("Select dimension", Accept::SELECT)
        } else if self.asking_angle {
            Prompt::new("Specify angle for dimension text", Accept::NUMBER)
        } else {
            Prompt::new("Specify new location for dimension text", Accept::POINT).kw(&["Left", "Right", "Center", "Home", "Angle"])
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if self.dim.is_none() {
            return Ok(match i {
                Input::Pick(hs) => {
                    match dims_of(s, &hs).first() {
                        Some(h) => self.dim = Some(*h),
                        None if hs.is_empty() => {}
                        None => s.echo("Object selected is not a dimension."),
                    }
                    Step::Continue
                }
                Input::Enter | Input::Cancel => Step::Cancel,
                _ => Step::Continue,
            });
        }
        if self.asking_angle {
            return match i {
                Input::Text(t) => match s.angle_settings().direction(&t) {
                    Some(a) => self.run(s, json!({ "mode": "angle", "angle": a.to_degrees() })),
                    None => {
                        s.echo("Requires a valid angle.");
                        Ok(Step::Continue)
                    }
                },
                Input::Enter | Input::Cancel => Ok(Step::Cancel),
                _ => Ok(Step::Continue),
            };
        }
        match i {
            Input::Point(p) => self.run(s, json!({ "at": [p.x, p.y] })),
            Input::Keyword(k) if k == "Angle" => {
                self.asking_angle = true;
                Ok(Step::Continue)
            }
            Input::Keyword(k) => self.run(s, json!({ "mode": k.to_ascii_lowercase() })),
            Input::Enter | Input::Cancel => Ok(Step::Cancel),
            _ => Ok(Step::Continue),
        }
    }
    /// The text follows the cursor.
    fn preview(&self, s: &Session, c: Vec2) -> Vec<EntityKind> {
        if self.asking_angle {
            return Vec::new();
        }
        let Some(EntityKind::Dimension(mut d)) = self.dim.and_then(|h| s.doc().ok()?.entity(h).map(|e| e.kind.clone())) else { return Vec::new() };
        d.text_mid = v3(c);
        d.user_text_pos = true;
        if let Ok(doc) = s.doc() {
            let st = doc.dim_style(&d.style).cloned().unwrap_or_default().with_overrides(&d.overrides);
            st.follow_text(&mut d, doc.header.f64("DIMSCALE", 1.0));
        }
        d.block = None;
        vec![EntityKind::Dimension(d)]
    }
}

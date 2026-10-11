//! Shape size dialogs (a click with a shape tool): Rectangle, Rounded Rectangle, Ellipse, Polygon,
//! Star and Line Segment Tool Options.

use serde_json::{Value, json};

use super::{DialogSpec, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| title(&d.kind).into(), confirm, ..DialogSpec::FORM };

fn title(kind: &str) -> &'static str {
    match kind {
        "rectangle" => tl!("Rectangle"),
        "roundedRectangle" => tl!("Rounded Rectangle"),
        "ellipse" => tl!("Ellipse"),
        "polygon" => tl!("Polygon"),
        "star" => tl!("Star"),
        _ => tl!("Line Segment Tool Options"),
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    let (id, params) = match d.kind.as_str() {
        "ellipse" => ("shape.ellipse", json!({"x": x, "y": y, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0)})),
        "polygon" => ("shape.polygon", json!({"cx": x, "cy": y, "radius": d.f64("radius", 50.0), "sides": d.f64("sides", 6.0) as u64})),
        "star" => (
            "shape.star",
            json!({"cx": x, "cy": y, "radius1": d.f64("radius1", 50.0), "radius2": d.f64("radius2", 25.0), "points": d.f64("points", 5.0) as u64}),
        ),
        "lineSegment" => {
            let (l, a) = (d.f64("length", 100.0), d.f64("angle", 0.0).to_radians());
            ("shape.line", json!({"x1": x, "y1": y, "x2": x + l * a.cos(), "y2": y - l * a.sin()}))
        }
        _ => (
            "shape.rectangle",
            json!({"x": x, "y": y, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0), "radius": d.f64("radius", 0.0)}),
        ),
    };
    // On the active plane while the perspective grid shows, sized in plane units from the click.
    match vectorcraft_engine::perspective_click(&app.session, id, &params, vectorcraft_geom::Point::new(x, y)) {
        Some((id, params)) => run_and_close(app, &id, params),
        None => run_and_close(app, id, params),
    }
}

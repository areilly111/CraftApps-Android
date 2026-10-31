//! Unit, angle and drafting settings in the drawing header survive saving as DXF or DWG and
//! reopening. The DWG bridge converts with acadrust, whose DXF writer leaves some header
//! variables out; the snap grid's origin and rotation live in the `*ACTIVE` VPORT record.

use cadcraft_doc::{Drawing, HVal};
use cadcraft_geom::Vec3;

fn drawing() -> Drawing {
    let mut d = Drawing::new_metric();
    let h = &mut d.header;
    h.set_i64("AUNITS", 3);
    h.set_i64("AUPREC", 5);
    h.set_i64("LUNITS", 4);
    h.set_i64("LUPREC", 6);
    h.set_f64("ANGBASE", 0.5);
    h.set_i64("ANGDIR", 1);
    h.set_i64("INSUNITS", 6);
    h.set_i64("MEASUREMENT", 0);
    h.set_f64("LTSCALE", 2.5);
    h.set_f64("DIMSCALE", 4.0);
    h.set_i64("PDMODE", 34);
    h.set_f64("PDSIZE", 1.5);
    h.set_f64("FILLETRAD", 2.0);
    h.set_f64("CHAMFERA", 1.25);
    h.set_f64("CHAMFERB", 0.75);
    h.set_f64("ELEVATION", 3.0);
    h.set_f64("THICKNESS", 0.25);
    h.set("SNAPBASE", HVal::Point(Vec3::new(3.0, 4.0, 0.0)));
    h.set_f64("SNAPANG", 30.0);
    d
}

fn check(name: &str, r: &Drawing) {
    let d = drawing();
    let ints = ["AUNITS", "AUPREC", "LUNITS", "LUPREC", "ANGDIR", "INSUNITS", "MEASUREMENT", "PDMODE"];
    for v in ints {
        assert_eq!(r.header.i64(v, -99), d.header.i64(v, -99), "{name}: ${v}");
    }
    let reals = ["ANGBASE", "LTSCALE", "DIMSCALE", "PDSIZE", "FILLETRAD", "CHAMFERA", "CHAMFERB", "ELEVATION", "THICKNESS", "SNAPANG"];
    for v in reals {
        let (got, want) = (r.header.f64(v, -99.0), d.header.f64(v, -99.0));
        assert!((got - want).abs() < 1e-9, "{name}: ${v} read back as {got}, not {want}");
    }
    let base = r.header.point("SNAPBASE").unwrap_or(Vec3::new(-99.0, -99.0, 0.0));
    assert!((base.x - 3.0).abs() < 1e-9 && (base.y - 4.0).abs() < 1e-9, "{name}: $SNAPBASE read back as {base:?}");
}

#[test]
fn header_settings_round_trip_through_dxf() {
    let d = drawing();
    let r = cadcraft_io::read_dxf(cadcraft_io::write_dxf(&d).as_bytes()).unwrap();
    check("DXF", &r);
}

#[test]
fn header_settings_round_trip_through_dwg() {
    let d = drawing();
    let dwg = cadcraft_io::write(&d, "x.dwg").unwrap();
    let r = cadcraft_io::read(&dwg, "x.dwg").unwrap();
    check("DWG", &r);
}

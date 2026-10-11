//! Units: format/parse round-trips in every unit, typed units winning over the field's unit, names
//! and canvas readouts.

use super::Unit;

#[test]
fn every_unit_round_trips_through_its_field_text() {
    for u in Unit::ALL {
        // Field text keeps three decimals of the unit: half a step is the most a round trip loses.
        let tol = 0.0005 * u.points() + 1e-9;
        for pt in [0.0, 1.0, 12.5, 72.0, 100.0, 595.28, 1234.5678, -36.0] {
            let shown = u.format(pt);
            let back = u.parse(&shown).unwrap_or_else(|| panic!("{u:?} can't read back `{shown}`"));
            assert!((back - pt).abs() <= tol, "{u:?}: {pt} → `{shown}` → {back}");
            // The bare number reads in the field's unit, and in any unit with its suffix.
            let bare = u.parse(&u.number(pt)).unwrap();
            assert!((bare - pt).abs() <= tol, "{u:?}: bare `{}` → {bare}", u.number(pt));
            let pts = Unit::Points.parse(&shown).unwrap();
            assert!((pts - pt).abs() <= tol, "{u:?}: `{shown}` in a points field → {pts}");
        }
    }
}

#[test]
fn typed_units_win_over_the_fields_unit() {
    let mm = 72.0 / 25.4;
    assert_eq!(Unit::Millimeters.parse("10"), Some(10.0 * mm));
    assert_eq!(Unit::Millimeters.parse("12 pt"), Some(12.0));
    assert_eq!(Unit::Millimeters.parse("1in"), Some(72.0));
    assert_eq!(Unit::Points.parse("5 mm"), Some(5.0 * mm));
    assert_eq!(Unit::Centimeters.parse("2p6"), Some(30.0));
    assert_eq!(Unit::Millimeters.parse("10+5"), Some(15.0 * mm));
    assert_eq!(Unit::Inches.parse("1 + 36 pt"), Some(108.0));
    assert_eq!(Unit::Pixels.parse("-4"), Some(-4.0));
}

#[test]
fn odd_text_reads_as_nothing_without_panicking() {
    for s in ["", "é", "é5", "°+1", "×2", "5 ×", "—3", "mm", "1/0", "--", "+"] {
        assert_eq!(Unit::Millimeters.parse(s), None, "{s}");
    }
}

#[test]
fn units_are_named_by_label_key_or_suffix() {
    for u in Unit::ALL {
        assert_eq!(Unit::named(u.label()), Some(u));
        assert_eq!(Unit::named(u.key()), Some(u));
        assert_eq!(Unit::named(&u.label().to_uppercase()), Some(u));
    }
    assert_eq!(Unit::named("mm"), Some(Unit::Millimeters));
    assert_eq!(Unit::named("Feet & Inches"), Some(Unit::FeetInches));
    assert_eq!(Unit::named("feetInches"), Some(Unit::FeetInches));
    assert_eq!(Unit::named("furlongs"), None);
    assert_eq!(Unit::named(""), None);
}

#[test]
fn readouts_show_two_decimals() {
    assert_eq!(Unit::Points.readout(15.0), "15.00 pt");
    assert_eq!(Unit::Millimeters.readout(72.0), "25.40 mm");
    assert_eq!(Unit::Inches.readout(-36.0), "-0.50 in");
    assert_eq!(Unit::Millimeters.number(1.0), "0.3528");
    // Four decimals for the large units, three for the small ones.
    assert_eq!(Unit::Inches.number(0.5616), "0.0078");
    assert_eq!((Unit::Inches.number(595.2756), Unit::Points.number(595.2756)), ("8.2677".into(), "595.276".into()));
    assert_eq!(Unit::Points.number(-0.00001), "0");
}

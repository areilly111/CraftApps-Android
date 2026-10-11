//! `scopes.read` on generated sequences with known colours (a colour matte, colour bars).

use serde_json::json;

use crate::Session;

fn matte_sequence(hex: &str) -> Session {
    let mut s = Session::default();
    let id = s.execute("file.newColorMatte", json!({"color": hex, "seconds": 2})).unwrap()["item"].as_u64().unwrap();
    s.execute("file.newSequenceFromClip", json!({"items": [id]})).unwrap();
    assert!(s.active_sequence().is_some());
    s
}

#[test]
fn scopes_read_reports_exact_levels_of_a_colour_matte() {
    let mut s = matte_sequence("#c86432");
    let r = s.execute("scopes.read", json!({"columns": 4})).unwrap();
    assert_eq!(r["colorSpace"], "Rec. 709");
    let h = &r["histogram"];
    assert_eq!(h["peakBin"], json!({"r": 200, "g": 100, "b": 50, "y": h["peakBin"]["y"].clone()}));
    let n = h["samples"].as_u64().unwrap();
    assert_eq!(h["r"][200].as_u64(), Some(n), "every sample in bin 200");
    // levels in percent: 200 / 255 = 78.43 %
    assert_eq!(r["stats"]["r"]["mean"], json!(78.43));
    assert_eq!(r["stats"]["b"]["max"], json!(19.61));
    for col in r["waveform"]["traces"]["R"].as_array().unwrap() {
        assert!((col["mean"].as_f64().unwrap() - 78.43).abs() < 0.01, "{col}");
    }
    assert_eq!(r["parade"]["traces"]["G"].as_array().unwrap().len(), 4);
    // the vectorscope: one spot at the matte's chroma
    let [_, cb, cr] = filmcraft_color::rgb_to_ycbcr(200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, filmcraft_color::Matrix::Bt709);
    let pk = &r["vectorscopeYuv"]["peaks"][0];
    assert_eq!(pk["share"], json!(1.0));
    assert!((pk["cb"].as_f64().unwrap() - cb as f64 * 100.0).abs() < 0.3 && (pk["cr"].as_f64().unwrap() - cr as f64 * 100.0).abs() < 0.3, "{pk}");
    // Rec. 601 moves luma; the matrix is reported
    let r601 = s.execute("scopes.read", json!({"colorSpace": "601", "scopes": ["histogram"]})).unwrap();
    assert_eq!(r601["matrix"], "Bt601");
    assert_ne!(r601["histogram"]["peakBin"]["y"], h["peakBin"]["y"]);
    assert!(r601.get("waveform").is_none());
    // options and errors
    let r = s.execute("scopes.read", json!({"scopes": ["waveform"], "waveformType": "yc", "bins": false})).unwrap();
    assert_eq!(r["waveform"]["type"], "yc");
    assert!(r["waveform"]["traces"]["C"].is_array());
    assert!(s.execute("scopes.read", json!({"scopes": ["oscilloscope"]})).is_err());
    assert!(s.execute("scopes.read", json!({"colorSpace": "P3"})).is_err());
    assert!(Session::default().execute("scopes.read", json!({})).is_err());
}

#[test]
fn scopes_read_finds_the_colour_bar_targets() {
    let mut s = Session::default();
    let id = s.execute("file.newBarsAndTone", json!({"seconds": 2})).unwrap()["item"].as_u64().unwrap();
    s.execute("file.newSequenceFromClip", json!({"items": [id]})).unwrap();
    let r = s.execute("scopes.read", json!({"scopes": ["vectorscopeYuv"], "scale": 0.25})).unwrap();
    let peaks = r["vectorscopeYuv"]["peaks"].as_array().unwrap();
    let cells: Vec<(u64, u64)> = peaks.iter().take(7).map(|p| (p["x"].as_u64().unwrap(), p["y"].as_u64().unwrap())).collect();
    let p = filmcraft_scopes::Params::default();
    for t in filmcraft_scopes::targets(filmcraft_color::Matrix::Bt709, 191.0 / 255.0) {
        let (x, y) = filmcraft_scopes::cell_of(t.cb, t.cr, p.vector_size, filmcraft_scopes::VECTOR_EXTENT).unwrap();
        assert!(cells.contains(&(x as u64, y as u64)), "{} target ({x}, {y}) not among {cells:?}", t.name);
    }
}

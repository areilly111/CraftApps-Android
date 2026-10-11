//! Edit → Color Settings and Assign Profile with non-default profiles. The Color Settings are
//! process-wide and every colour shown or rendered goes through them, so these tests run in their
//! own test binary (process) instead of next to the engine's unit tests, which would see the
//! switched working spaces mid-test; here they take turns.

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_color::cms;
use vectorcraft_engine::cmd::colormgmt::{PROFILES_KEY, doc_profiles};
use vectorcraft_testkit::fixtures::session;

/// The active Color Settings: one test at a time.
fn settings() -> MutexGuard<'static, ()> {
    static SETTINGS: Mutex<()> = Mutex::new(());
    SETTINGS.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn color_settings_lists_profiles_and_validates() {
    let _g = settings();
    let mut s = session();
    let r = s.execute("edit.colorSettings", &json!({})).unwrap();
    let names: Vec<&str> = r["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&cms::GENERIC_CMYK) && names.contains(&cms::SRGB) && names.contains(&cms::DEVICE_CMYK));
    assert!(s.execute("edit.colorSettings", &json!({"cmyk": "Missing.icc"})).is_err());
    assert_eq!(cms::active_settings().cmyk, r["cmyk"].as_str().unwrap(), "failed change leaves settings alone");
    let before = r["bpc"].as_bool().unwrap();
    let r2 = s.execute("edit.colorSettings", &json!({"bpc": !before})).unwrap();
    assert_eq!(r2["bpc"], !before);
    s.execute("edit.colorSettings", &json!({"bpc": before})).unwrap();
    assert!(s.execute("color.loadProfile", &json!({"path": "/nonexistent/x.icc"})).is_err());
}

#[test]
fn legacy_profile_name_resolves_in_commands_and_files() {
    let _g = settings();
    let (old, _) = cms::LEGACY_NAMES[0];
    let mut s = session();
    let before = cms::active_settings();
    let r = s.execute("edit.colorSettings", &json!({"rgb": old})).unwrap();
    assert_eq!(r["rgb"], cms::WIDE_GAMUT_RGB);
    assert_eq!(cms::active_settings().rgb, cms::WIDE_GAMUT_RGB);
    let names: Vec<&str> = r["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&cms::WIDE_GAMUT_RGB) && !names.contains(&old));
    let r = s.execute("edit.assignProfile", &json!({"rgb": old})).unwrap();
    assert_eq!(r["rgb"], cms::WIDE_GAMUT_RGB, "stored under the current name");
    // A document saved by an older version names the profile the old way.
    let mut d = (*s.doc().unwrap().doc).clone();
    d.unknown.insert(PROFILES_KEY.into(), json!({"rgb": old, "cmyk": null}));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(doc_profiles(&back), (Some(cms::WIDE_GAMUT_RGB.to_string()), None));
    cms::set_active(&before).unwrap();
}

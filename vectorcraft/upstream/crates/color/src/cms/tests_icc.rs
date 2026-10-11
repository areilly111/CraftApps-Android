use super::*;

fn parse(bytes: &[u8]) -> moxcms::ColorProfile {
    moxcms::ColorProfile::new_from_slice(bytes).unwrap()
}

#[test]
fn builtin_rgb_spaces_encode_under_their_names() {
    for name in BUILTIN_RGB {
        let p = IccProfile::from_bytes(None, &icc_bytes(name).unwrap()).unwrap();
        assert_eq!((p.name.as_str(), p.kind), (name, ProfileKind::Rgb));
    }
    // Legacy names find today's profile.
    assert_eq!(icc_bytes(LEGACY_NAMES[0].0).unwrap(), icc_bytes(WIDE_GAMUT_RGB).unwrap());
    assert!(matches!(icc_bytes("No Such Profile"), Err(CmsError::UnknownProfile(_))));
}

#[test]
fn cmyk_profiles_convert_like_the_cms() {
    for name in BUILTIN_CMYK {
        let bytes = icc_bytes(name).unwrap();
        assert_eq!(parse(&bytes).color_space, moxcms::DataColorSpace::Cmyk);
        let p = IccProfile::from_bytes(None, &bytes).unwrap();
        assert_eq!((p.name.as_str(), p.kind), (name, ProfileKind::Cmyk));
        let cms = Cms::new(&ColorSettings { cmyk: name.into(), ..Default::default() }).unwrap();
        for cmyk in [[0.0; 4], [1.0, 0.0, 0.0, 0.0], [0.2, 0.7, 0.1, 0.3], [0.0, 0.0, 0.0, 1.0], [0.5, 0.5, 0.5, 0.0]] {
            let want = cms.cmyk_to_srgb(cmyk, false);
            let got = p.to_srgb(&cmyk, Intent::RelativeColorimetric).unwrap();
            let de = delta_e2000(lab::srgb_to_lab(want), lab::srgb_to_lab(got));
            assert!(de < 2.5, "{name} {cmyk:?}: {want:?} vs {got:?} (ΔE {de})");
        }
        // And back: an in-gamut colour separates to about the same inks.
        let srgb = cms.cmyk_to_srgb([0.1, 0.6, 0.8, 0.0], false);
        let back = p.from_srgb(srgb, Intent::RelativeColorimetric).unwrap();
        let again = cms.cmyk_to_srgb([back[0], back[1], back[2], back[3]], false);
        assert!(delta_e2000(lab::srgb_to_lab(srgb), lab::srgb_to_lab(again)) < 3.0, "{back:?}");
    }
}

#[test]
fn gray_is_srgb_toned() {
    let p = IccProfile::from_bytes(None, &icc_bytes(GRAY).unwrap()).unwrap();
    assert_eq!((p.name.as_str(), p.kind), (GRAY, ProfileKind::Gray));
    for v in [0.0f32, 0.25, 0.5, 1.0] {
        let got = p.to_srgb(&[v], Intent::RelativeColorimetric).unwrap();
        assert!(got.iter().all(|c| (c - v).abs() < 0.01), "{v} → {got:?}");
    }
}

#[test]
fn encoded_profiles_are_reproducible() {
    assert_eq!(icc::builtin_rgb_bytes(DISPLAY_P3).unwrap(), icc::builtin_rgb_bytes(DISPLAY_P3).unwrap());
}

#[test]
fn a_loaded_profile_is_embedded_as_loaded() {
    let mut bytes = icc::builtin_rgb_bytes(PROPHOTO_RGB).unwrap();
    // A byte the parser ignores (in the reserved header area) marks the file as this one.
    bytes[100] = 7;
    register_icc(&bytes, Some("Embedded As Loaded RGB".into())).unwrap();
    assert_eq!(&*icc_bytes("Embedded As Loaded RGB").unwrap(), &bytes[..]);
}

/// Settings with an ICC CMYK profile (Generic CMYK's, loaded as a file is) and Wide Gamut RGB.
fn icc_cmyk(bpc: bool) -> Cms {
    let name = register_icc(&icc_bytes(GENERIC_CMYK).unwrap(), Some("Test ICC CMYK".into())).unwrap().name;
    Cms::new(&ColorSettings { rgb: WIDE_GAMUT_RGB.into(), cmyk: name, bpc, ..ColorSettings::default() }).unwrap()
}

#[test]
fn icc_cmyk_converts_into_a_wide_rgb_space_directly_without_clipping_to_srgb() {
    let wide = icc_cmyk(true);
    let cyan = [1.0, 0.0, 0.0, 0.0];
    let direct = wide.cmyk_to_rgb(cyan, Intent::RelativeColorimetric);
    // Through sRGB, cyan clips to sRGB's gamut first: in the wider space that reads as a red
    // channel well above zero.
    let hub = wide.srgb_to_rgb(wide.cmyk_to_srgb(cyan, false));
    assert!(direct[0] + 0.15 < hub[0], "direct {direct:?}, through sRGB {hub:?}");
    // Same colour either way where sRGB holds it: a mid grey and paper white.
    for inks in [[0.0, 0.0, 0.0, 0.5], [0.0; 4]] {
        let (d, h) = (wide.cmyk_to_rgb(inks, Intent::RelativeColorimetric), wide.srgb_to_rgb(wide.cmyk_to_srgb(inks, false)));
        assert!(d.iter().zip(h).all(|(a, b)| (a - b).abs() < 0.02), "{inks:?}: direct {d:?}, through sRGB {h:?}");
    }
    // Cms::convert takes the direct route for CMYK.
    let Color::Rgb { r, .. } = wide.convert(&Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 }, Model::Rgb, Intent::RelativeColorimetric) else {
        panic!("RGB")
    };
    assert!((r - direct[0]).abs() < 1e-6);
}

#[test]
fn black_point_compensation_maps_the_darkest_ink_to_rgb_black() {
    let rich = [0.75, 0.68, 0.67, 0.9];
    let (on, off) = (icc_cmyk(true).cmyk_to_rgb(rich, Intent::RelativeColorimetric), icc_cmyk(false).cmyk_to_rgb(rich, Intent::RelativeColorimetric));
    // Without compensation the profile's black is a dark grey; with it, rich black goes most of
    // the way to RGB black (not all: it isn't quite the darkest ink the profile prints).
    assert!(off.iter().all(|v| *v > 0.1), "without: {off:?}");
    assert!(on.iter().zip(off).all(|(a, b)| *a < 0.08 && *a < b * 0.5), "with: {on:?}, without: {off:?}");
    // The screen shows what an sRGB conversion writes, compensated alike.
    let srgb = |bpc| {
        let c = icc_cmyk(bpc);
        Cms::new(&ColorSettings { rgb: SRGB.into(), ..c.settings().clone() }).unwrap()
    };
    for bpc in [true, false] {
        let c = srgb(bpc);
        assert_eq!(c.cmyk_to_srgb(rich, false), c.cmyk_to_rgb(rich, Intent::RelativeColorimetric), "bpc {bpc}");
    }
    // White is untouched, and absolute colorimetric never compensates.
    let white = icc_cmyk(true).cmyk_to_rgb([0.0; 4], Intent::RelativeColorimetric);
    assert!(white.iter().all(|v| *v > 0.99), "{white:?}");
    let abs = |bpc| icc_cmyk(bpc).cmyk_to_rgb(rich, Intent::AbsoluteColorimetric);
    assert_eq!(abs(true), abs(false));
}

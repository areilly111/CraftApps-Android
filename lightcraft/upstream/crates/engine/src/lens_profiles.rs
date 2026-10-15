//! Lens profile corrections from a lens database the user supplies ([`lightcraft_lensdb`]).
//!
//! When a raw carries no lens correction of its own (DNG opcodes, or a camera correction a raw reader converts,
//! [`crate::files::embedded_lens`]), the lens named in its EXIF is looked up in a lensfun-format database: the
//! directory in `LIGHTCRAFT_LENSFUN_DIR` (e.g. lensfun's `data/db`, CC-BY-SA, which is never bundled with the app).
//! The profile is converted to the same [`lightcraft_develop::EmbeddedLens`] form, so "Enable Profile Corrections", its
//! amount sliders, the GPU path and DNG export treat both alike. Without the variable nothing changes.
use std::path::PathBuf;
use std::sync::OnceLock;

use lightcraft_develop::EmbeddedLens;
use lightcraft_lensdb::{LensDb, Shot};

/// The environment variable naming the lens database directory.
pub const ENV: &str = "LIGHTCRAFT_LENSFUN_DIR";

/// The configured database, read once per process; `None` when none is configured or it holds no lenses.
pub fn database() -> Option<&'static LensDb> {
    static DB: OnceLock<Option<LensDb>> = OnceLock::new();
    DB.get_or_init(|| {
        let dir = PathBuf::from(std::env::var_os(ENV)?);
        match LensDb::load_dir(&dir) {
            Ok((db, errors)) => {
                for (file, e) in errors {
                    log::warn!("lens database: skipped {file}: {e}");
                }
                (!db.lenses.is_empty()).then_some(db)
            }
            Err(e) => {
                log::warn!("{e}");
                None
            }
        }
    })
    .as_ref()
}

/// The database profile for a photo with metadata `m`, for its `w × h` (unoriented, default-cropped) image,
/// re-expressed after `orientation`. `None` without a database, lens name, focal length or matching profile.
pub fn lookup(db: &LensDb, m: &lightcraft_meta::Metadata, w: f64, h: f64, orientation: lightcraft_geom::Orientation) -> Option<EmbeddedLens> {
    let focal = m.focal_length.filter(|f| f.is_finite() && *f > 0.0)?;
    let camera = db.find_camera(m.make.as_deref().unwrap_or_default(), m.model.as_deref().unwrap_or_default());
    let lens = db.find_lens(m.lens_make.as_deref(), m.lens_model.as_deref()?, camera)?;
    // the crop factor: the camera's, else what the 35 mm equivalent focal length implies
    let crop =
        camera.map(|c| c.crop_factor).or_else(|| m.focal_length_35mm.map(|f35| f35 / focal).filter(|c| c.is_finite() && (0.1..=20.0).contains(c)));
    let shot = Shot { focal, aperture: m.f_number, distance: None, crop_factor: crop, width: w, height: h };
    let corr = lens.correction(&shot)?;
    Some(lightcraft_pipeline::optics::reorient_lens(&corr, orientation, w, h))
}

/// [`crate::files::embedded_lens`], else the database profile ([`lookup`]) for a raw with metadata `meta`.
pub fn for_raw(raw: &lightcraft_raw::RawInfo, meta: impl FnOnce() -> lightcraft_meta::Metadata) -> Option<EmbeddedLens> {
    crate::files::embedded_lens(raw).or_else(|| {
        let db = database()?;
        let c = raw.crop.clipped(raw.active_area.width, raw.active_area.height);
        let (w, h) = if c.width == 0 || c.height == 0 { (raw.active_area.width, raw.active_area.height) } else { (c.width, c.height) };
        lookup(db, &meta(), w as f64, h as f64, raw.orientation)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_geom::Orientation;

    const DB: &str = r#"<lensdatabase>
      <camera><maker>Testcam</maker><model>TC-1</model><mount>T</mount><cropfactor>1.5</cropfactor></camera>
      <lens><maker>Testcam</maker><model>Testcam 18-55mm f/3.5-5.6</model><mount>T</mount><cropfactor>1.5</cropfactor>
        <calibration>
          <distortion model="poly3" focal="18" k1="-0.03" />
          <vignetting model="pa" focal="18" aperture="3.5" distance="1000" k1="-0.4" k2="0" k3="0" />
        </calibration>
      </lens></lensdatabase>"#;

    fn meta() -> lightcraft_meta::Metadata {
        lightcraft_meta::Metadata {
            make: Some("Testcam".into()),
            model: Some("TC-1".into()),
            lens_model: Some("18-55mm F3.5-5.6".into()),
            focal_length: Some(18.0),
            f_number: Some(3.5),
            ..Default::default()
        }
    }

    #[test]
    fn exif_lens_is_found_and_converted() {
        let db = LensDb::parse(DB).unwrap();
        let lens = lookup(&db, &meta(), 6000.0, 4000.0, Orientation::Normal).unwrap();
        assert!(lens.warp.is_some() && lens.vignette.is_some());
        let rotated = lookup(&db, &meta(), 6000.0, 4000.0, Orientation::Rotate90).unwrap();
        assert_eq!(rotated.warp.unwrap().center, lightcraft_geom::Point::new(0.5, 0.5));
        // no lens name, no focal length, an unknown lens: nothing
        assert!(lookup(&db, &lightcraft_meta::Metadata { lens_model: None, ..meta() }, 6000.0, 4000.0, Orientation::Normal).is_none());
        assert!(lookup(&db, &lightcraft_meta::Metadata { focal_length: None, ..meta() }, 6000.0, 4000.0, Orientation::Normal).is_none());
        let other = lightcraft_meta::Metadata { lens_model: Some("70-200mm F4".into()), ..meta() };
        assert!(lookup(&db, &other, 6000.0, 4000.0, Orientation::Normal).is_none());
    }

    /// A synthetic grid recorded through the profile's barrel distortion comes out of the pipeline's geometry
    /// stage with straight lines.
    #[test]
    fn pipeline_straightens_a_distorted_grid() {
        let db = LensDb::parse(DB).unwrap();
        let profile = &db.lenses[0];
        let (w, h) = (600usize, 400usize);
        let lens = lookup(&db, &meta(), w as f64, h as f64, Orientation::Normal).unwrap();
        // record the grid: each sensor pixel sees the scene point whose distorted position it is
        let n = h as f64 / 2.0;
        let undistort = |x: f64, y: f64| {
            let (dx, dy) = ((x - w as f64 / 2.0) / n, (y - h as f64 / 2.0) / n);
            let rd = dx.hypot(dy);
            let (mut lo, mut hi) = (0.0, 3.0);
            for _ in 0..60 {
                let mid = (lo + hi) / 2.0;
                if mid * profile.distortion_ratio(18.0, mid) < rd {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let s = if rd > 0.0 { lo / rd } else { 1.0 };
            (w as f64 / 2.0 + dx * s * n, h as f64 / 2.0 + dy * s * n)
        };
        // horizontal scene lines every 50 px (2 px wide, dark on light)
        let line = |v: f64| if v.rem_euclid(50.0) < 2.0 { 0.02f32 } else { 0.8 };
        let src = lightcraft_raster::Rgb32f::from_fn(w, h, |x, y| {
            let (_, sy) = undistort(x as f64 + 0.5, y as f64 + 0.5);
            [line(sy); 3]
        });
        // the dark row nearest y = 51 in each column, sub-pixel (centroid of darkness), as rendered
        let line_rows = |img: &lightcraft_raster::Rgba8| -> Vec<f64> {
            (20..w - 20)
                .step_by(10)
                .map(|x| {
                    let (mut sw, mut swy) = (0.0, 0.0);
                    for y in 35..70 {
                        let d = 255.0 - img.get(x, y)[1] as f64;
                        if d > 60.0 {
                            sw += d;
                            swy += d * (y as f64 + 0.5);
                        }
                    }
                    swy / sw.max(1e-9)
                })
                .collect()
        };
        let spread = |rows: &[f64]| rows.iter().cloned().fold(f64::MIN, f64::max) - rows.iter().cloned().fold(f64::MAX, f64::min);
        let req = lightcraft_pipeline::RenderRequest::fit(w, h);
        let mut s = lightcraft_develop::DevelopSettings::default();
        let info = lightcraft_pipeline::SourceInfo { lens: Some(lens), ..Default::default() };
        let before = spread(&line_rows(&lightcraft_pipeline::render(&src, &info, &s, &req).image));
        s.optics.lens_profile = true;
        let after = spread(&line_rows(&lightcraft_pipeline::render(&src, &info, &s, &req).image));
        assert!(before > 2.0, "the recorded line is curved: {before} px");
        // 8-bit output and a 2 px line: the centroid measurement itself is good to about a pixel
        assert!(after < 1.0 && after < before / 8.0, "corrected line bent by {after} px (uncorrected {before})");
    }
}

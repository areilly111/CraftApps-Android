//! Effect → Document Raster Effects Settings, besides the resolution
//! ([`crate::Document::raster_effects_ppi`]): how raster effects (shadows, glows, blurs, feathers)
//! become pixels when they are flattened for output or expanded, and the defaults of Rasterize.
//! Every field is defaulted, so documents saved before these settings existed load unchanged.

use serde::{Deserialize, Serialize};

use crate::setup::Background;

/// The colour model raster effect images are made in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RasterColorModel {
    /// The document's colour mode (RGB or CMYK).
    #[default]
    Document,
    Grayscale,
    /// Black and white pixels only.
    Bitmap,
}

impl RasterColorModel {
    pub const ALL: [RasterColorModel; 3] = [RasterColorModel::Document, RasterColorModel::Grayscale, RasterColorModel::Bitmap];
    pub fn id(self) -> &'static str {
        match self {
            RasterColorModel::Document => "document",
            RasterColorModel::Grayscale => "grayscale",
            RasterColorModel::Bitmap => "bitmap",
        }
    }
}

/// Largest Add … Around Object, in points.
pub const MAX_ADD_AROUND: f64 = 1000.0;

/// Document Raster Effects Settings (resolution aside).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RasterEffectsSettings {
    pub color_model: RasterColorModel,
    /// White: images are opaque, on white. Transparent: they keep their transparency.
    pub background: Background,
    /// Off: the art's edges are hard (raster effects stay smooth).
    pub anti_alias: bool,
    /// With a white background, the white stays only under the art (outside it the image is
    /// transparent); Rasterize clips its image to the art's outline.
    pub clipping_mask: bool,
    /// Extra room around the art, in points (0–[`MAX_ADD_AROUND`]).
    pub add_around: f64,
    /// Keep spot colours in raster effects (stored; effect images are made in process colour).
    pub preserve_spot_colors: bool,
}

impl Default for RasterEffectsSettings {
    fn default() -> Self {
        Self {
            color_model: RasterColorModel::Document,
            background: Background::Transparent,
            anti_alias: true,
            clipping_mask: false,
            add_around: 0.0,
            preserve_spot_colors: true,
        }
    }
}

impl RasterEffectsSettings {
    /// Turn premultiplied RGBA8 pixels into what these settings make of them: the colour model,
    /// then the background. (Anti-aliasing is the renderer's.)
    pub fn finish_pixels(&self, pixels: &mut [u8]) {
        let white = self.background == Background::White;
        if self.color_model == RasterColorModel::Document && !white {
            return;
        }
        for px in pixels.as_chunks_mut::<4>().0 {
            let a = px[3];
            if self.color_model != RasterColorModel::Document && a != 0 {
                // Rec. 709 luma of the premultiplied colour (still premultiplied).
                let y = 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
                let y = match self.color_model {
                    RasterColorModel::Bitmap => {
                        if y >= a as f32 / 2.0 {
                            a
                        } else {
                            0
                        }
                    }
                    _ => y.round().min(a as f32) as u8,
                };
                px[..3].fill(y);
            }
            // Over white; a clipping mask leaves what the art doesn't cover transparent.
            if white && !(self.clipping_mask && a == 0) {
                let under = 255 - a;
                for c in &mut px[..3] {
                    *c = c.saturating_add(under);
                }
                px[3] = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(s: RasterEffectsSettings, px: [u8; 4]) -> [u8; 4] {
        let mut p = px;
        s.finish_pixels(&mut p);
        p
    }

    #[test]
    fn pixels_follow_the_settings() {
        let d = RasterEffectsSettings::default();
        assert_eq!(finish(d.clone(), [100, 0, 0, 100]), [100, 0, 0, 100], "the defaults change nothing");
        let white = RasterEffectsSettings { background: Background::White, ..d.clone() };
        assert_eq!(finish(white.clone(), [0, 0, 0, 0]), [255, 255, 255, 255]);
        assert_eq!(finish(white.clone(), [100, 0, 0, 100]), [255, 155, 155, 255]);
        let masked = RasterEffectsSettings { clipping_mask: true, ..white };
        assert_eq!(finish(masked, [0, 0, 0, 0]), [0, 0, 0, 0]);
        let gray = RasterEffectsSettings { color_model: RasterColorModel::Grayscale, ..d.clone() };
        assert_eq!(finish(gray, [255, 0, 0, 255]), [54, 54, 54, 255]);
        let bitmap = RasterEffectsSettings { color_model: RasterColorModel::Bitmap, ..d };
        assert_eq!(finish(bitmap.clone(), [255, 0, 0, 255]), [0, 0, 0, 255]);
        assert_eq!(finish(bitmap, [0, 255, 0, 255]), [255, 255, 255, 255]);
    }

    #[test]
    fn partial_settings_fill_in_the_rest() {
        let s: RasterEffectsSettings = serde_json::from_str(r#"{"background":"white"}"#).unwrap();
        assert_eq!(s, RasterEffectsSettings { background: Background::White, ..Default::default() });
    }
}

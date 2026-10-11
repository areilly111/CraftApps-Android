//! ICC profiles through `moxcms` (pure Rust): built-in RGB spaces and user-supplied `.icc` files.

use std::sync::{Arc, OnceLock};

use moxcms::{ColorProfile, DataColorSpace, Layout, ProfileText, RenderingIntent, TransformF32Executor, TransformOptions};

use super::{CmsError, Intent, ProfileKind};

fn moxcms_intent(i: Intent) -> RenderingIntent {
    match i {
        Intent::Perceptual => RenderingIntent::Perceptual,
        Intent::RelativeColorimetric => RenderingIntent::RelativeColorimetric,
        Intent::Saturation => RenderingIntent::Saturation,
        Intent::AbsoluteColorimetric => RenderingIntent::AbsoluteColorimetric,
    }
}

const INTENTS: [Intent; 4] = [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation, Intent::AbsoluteColorimetric];

fn intent_index(i: Intent) -> usize {
    INTENTS.iter().position(|x| *x == i).unwrap_or(1)
}

type Xf = Option<Arc<TransformF32Executor>>;

/// A parsed ICC profile plus lazily-built transforms to/from sRGB.
pub struct IccProfile {
    pub name: String,
    pub kind: ProfileKind,
    profile: ColorProfile,
    srgb: ColorProfile,
    to_srgb: [OnceLock<Xf>; 4],
    from_srgb: [OnceLock<Xf>; 4],
}

impl std::fmt::Debug for IccProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IccProfile").field("name", &self.name).field("kind", &self.kind).finish()
    }
}

fn text(t: &Option<ProfileText>) -> Option<String> {
    match t.as_ref()? {
        ProfileText::PlainString(s) => Some(s.clone()),
        ProfileText::Localizable(v) => v.iter().find(|l| l.language == "en").or(v.first()).map(|l| l.value.clone()),
        ProfileText::Description(d) => Some(if d.ascii_string.is_empty() { d.unicode_string.clone() } else { d.ascii_string.clone() }),
    }
    .map(|s| s.trim_matches(char::from(0)).trim().to_string())
    .filter(|s| !s.is_empty())
}

impl IccProfile {
    pub fn from_profile(name: Option<String>, profile: ColorProfile) -> Result<Self, CmsError> {
        let kind = match profile.color_space {
            DataColorSpace::Rgb => ProfileKind::Rgb,
            DataColorSpace::Cmyk => ProfileKind::Cmyk,
            DataColorSpace::Gray => ProfileKind::Gray,
            other => return Err(CmsError::Unsupported(format!("profile colour space {other:?} is not RGB, CMYK or Gray"))),
        };
        let name = name.or_else(|| text(&profile.description)).unwrap_or_else(|| format!("Untitled {kind:?} profile"));
        Ok(Self { name, kind, profile, srgb: ColorProfile::new_srgb(), to_srgb: Default::default(), from_srgb: Default::default() })
    }

    pub fn from_bytes(name: Option<String>, bytes: &[u8]) -> Result<Self, CmsError> {
        let p = ColorProfile::new_from_slice(bytes).map_err(|e| CmsError::BadProfile(format!("{e:?}")))?;
        Self::from_profile(name, p)
    }

    fn layout(&self) -> Layout {
        match self.kind {
            ProfileKind::Cmyk => Layout::Rgba, // 4 channels
            ProfileKind::Gray => Layout::Gray,
            ProfileKind::Rgb => Layout::Rgb,
        }
    }

    fn channels(&self) -> usize {
        match self.kind {
            ProfileKind::Cmyk => 4,
            ProfileKind::Gray => 1,
            ProfileKind::Rgb => 3,
        }
    }

    fn opts(intent: Intent) -> TransformOptions {
        TransformOptions { rendering_intent: moxcms_intent(intent), ..Default::default() }
    }

    fn to_xf(&self, intent: Intent) -> Xf {
        self.to_srgb[intent_index(intent)]
            .get_or_init(|| self.profile.create_transform_f32(self.layout(), &self.srgb, Layout::Rgb, Self::opts(intent)).ok())
            .clone()
    }

    fn inverse_xf(&self, intent: Intent) -> Xf {
        self.from_srgb[intent_index(intent)]
            .get_or_init(|| self.srgb.create_transform_f32(Layout::Rgb, &self.profile, self.layout(), Self::opts(intent)).ok())
            .clone()
    }

    /// Device values (RGB / CMYK / Gray, 0..1) → sRGB.
    pub fn to_srgb(&self, v: &[f32], intent: Intent) -> Option<[f32; 3]> {
        let xf = self.to_xf(intent)?;
        let mut src = v.to_vec();
        src.resize(self.channels(), 0.0);
        let mut dst = [0.0f32; 3];
        xf.transform(&src, &mut dst).ok()?;
        Some(dst.map(|x| x.clamp(0.0, 1.0)))
    }

    /// sRGB → device values.
    pub fn from_srgb(&self, rgb: [f32; 3], intent: Intent) -> Option<Vec<f32>> {
        let xf = self.inverse_xf(intent)?;
        let mut dst = vec![0.0f32; self.channels()];
        xf.transform(&rgb, &mut dst).ok()?;
        Some(dst.into_iter().map(|x| x.clamp(0.0, 1.0)).collect())
    }

    /// Whether transforms can be built for this profile (checked at registration).
    pub fn usable(&self) -> bool {
        self.to_xf(Intent::RelativeColorimetric).is_some() && self.inverse_xf(Intent::RelativeColorimetric).is_some()
    }
}

/// Built-in RGB working spaces from `moxcms`'s standard primaries.
pub fn builtin_rgb(name: &str) -> Option<IccProfile> {
    let p = match name {
        super::ADOBE_RGB => ColorProfile::new_adobe_rgb(),
        super::DISPLAY_P3 => ColorProfile::new_display_p3(),
        super::PROPHOTO_RGB => ColorProfile::new_pro_photo_rgb(),
        super::SRGB => ColorProfile::new_srgb(),
        _ => return None,
    };
    IccProfile::from_profile(Some(name.to_string()), p).ok()
}

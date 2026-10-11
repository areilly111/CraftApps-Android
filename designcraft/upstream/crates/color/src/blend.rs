//! Blend modes (the 16 modes of InDesign's Transparency panel, same as PDF's).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// In Transparency-panel order (grouped as InDesign groups them).
    pub const ALL: [BlendMode; 16] = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::HardLight => "Hard Light",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
        }
    }
    /// Index of the separator group (the panel draws a divider between groups).
    pub fn group(self) -> u8 {
        match self {
            BlendMode::Normal => 0,
            BlendMode::Darken | BlendMode::Multiply | BlendMode::ColorBurn => 1,
            BlendMode::Lighten | BlendMode::Screen | BlendMode::ColorDodge => 2,
            BlendMode::Overlay | BlendMode::SoftLight | BlendMode::HardLight => 3,
            BlendMode::Difference | BlendMode::Exclusion => 4,
            _ => 5,
        }
    }
    /// Parse a label or identifier, case/space-insensitively.
    pub fn parse(s: &str) -> Option<Self> {
        let norm = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-'], "");
        let want = norm(s);
        Self::ALL.into_iter().find(|m| norm(m.label()) == want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_labels() {
        assert_eq!(BlendMode::parse("color burn"), Some(BlendMode::ColorBurn));
        assert_eq!(BlendMode::parse("ColorBurn"), Some(BlendMode::ColorBurn));
        assert_eq!(BlendMode::parse("soft_light"), Some(BlendMode::SoftLight));
        assert_eq!(BlendMode::parse("nope"), None);
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::parse(m.label()), Some(m));
        }
    }
}

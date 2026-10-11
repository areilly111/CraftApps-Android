//! The suite the launcher manages: our seven Android ports, plus the
//! launcher itself (it updates itself with the same machinery).
//!
//! Modeled on `artcraft-launcher-core`'s catalog, trimmed to what the
//! Android launcher needs: display name, tagline, brand color, Android
//! package name and the release tag prefix on `areilly111/CraftApps-Android`.

/// One app in the suite.
pub struct SuiteApp {
    /// Machine name (`photocraft`); also the release tag prefix stem.
    pub id: &'static str,
    /// Display name (`PhotoCraft`).
    pub name: &'static str,
    /// What it's for, in a few words.
    pub tagline: &'static str,
    /// Android applicationId (`app.photocraft.android`).
    pub package: &'static str,
    /// Release tag prefix on areilly111/CraftApps-Android (`photocraft-v`).
    pub tag_prefix: &'static str,
    /// Brand color (sRGB), for the card's accent strip.
    pub color: [u8; 3],
}

const fn app(
    id: &'static str,
    name: &'static str,
    tagline: &'static str,
    package: &'static str,
    color: [u8; 3],
) -> SuiteApp {
    SuiteApp { id, name, tagline, package, tag_prefix: id, color }
}

/// Our seven Android ports, in display order, then the launcher itself.
/// `tag_prefix` is `"<id>-v"`: releases are tagged `<id>-v<upstream>-port<build>`
/// (e.g. `photocraft-v0.6.0-port5`).
pub const SUITE: [SuiteApp; 8] = [
    app(
        "photocraft",
        "PhotoCraft",
        "Image editing",
        "app.photocraft.android",
        [0x2f, 0x7b, 0xf5],
    ),
    app(
        "designcraft",
        "DesignCraft",
        "Page layout and publishing",
        "app.designcraft.android",
        [0x7b, 0xb5, 0x1c],
    ),
    app(
        "vectorcraft",
        "VectorCraft",
        "Vector illustration",
        "app.vectorcraft.android",
        [0xe8, 0x57, 0x3f],
    ),
    app(
        "lightcraft",
        "LightCraft",
        "Photo library and raw development",
        "app.lightcraft.android",
        [0xf2, 0xa5, 0x16],
    ),
    app(
        "pdfcraft",
        "PdfCraft",
        "Reading and organizing PDFs",
        "app.pdfcraft.android",
        [0x12, 0xa5, 0x8a],
    ),
    app(
        "effectcraft",
        "EffectCraft",
        "Motion graphics and visual effects",
        "app.effectcraft.android",
        [0xe0, 0x36, 0x8f],
    ),
    app(
        "filmcraft",
        "FilmCraft",
        "Video editing, color and sound",
        "app.filmcraft.android",
        [0x8b, 0x5c, 0xf6],
    ),
    app(
        "craftlauncher",
        "Craft Launcher",
        "Installs and updates the Craft apps",
        "app.craftlauncher.android",
        [0xf2, 0xf1, 0xee],
    ),
];

impl SuiteApp {
    /// Full tag prefix including the `-v` separator (`"photocraft-v"`).
    pub fn full_tag_prefix(&self) -> String {
        format!("{}-v", self.tag_prefix)
    }
}

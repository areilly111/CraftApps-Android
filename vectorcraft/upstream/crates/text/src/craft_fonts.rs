//! Fonts from the optional craft-fonts build input (<https://github.com/storytold/craft-fonts>).
//!
//! Built with `CRAFT_FONTS_DIR=<craft-fonts checkout>`, [`CRAFT_FONTS`] holds every font in its
//! manifest (Japanese UI and document fonts, and Arabic fonts); built without it, it is empty and Japanese
//! text falls back to the installed system fonts. Release builds always set it. See `build.rs`
//! and craftrules `standards/fonts.md`.

/// A font from the optional craft-fonts build input (empty unless built with `CRAFT_FONTS_DIR`).
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    /// ISO 15924 scripts the font is for, e.g. `"Jpan"`.
    pub scripts: &'static [&'static str],
    pub bytes: &'static [u8],
}

/// A craft-fonts face the web (wasm32) build fetches from beside the wasm instead of embedding,
/// listed in `crates/text/web-fonts.txt` (empty unless built with `CRAFT_FONTS_DIR`). Fetched by
/// `apps/vectorcraft-web/src/fonts.rs`; copied into the site by `cargo xtask web-fonts`.
pub struct WebFont {
    pub family: &'static str,
    pub style: &'static str,
    /// Fetched before the app starts (a fallback the first frame needs), else in the background.
    pub startup: bool,
    /// Site-relative URL, `fonts/<first 16 hex digits of the SHA-256>/<file name>`.
    pub url: &'static str,
    /// Subresource Integrity value, `sha256-<base64>`; the browser rejects other bytes.
    pub integrity: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

impl CraftFont {
    /// Is the font for Japanese text?
    pub fn is_japanese(&self) -> bool {
        self.scripts.contains(&"Jpan")
    }
    /// Is the font for Arabic text?
    pub fn is_arabic(&self) -> bool {
        self.scripts.contains(&"Arab")
    }
    /// A serif (Mincho) family, for document text.
    pub fn is_mincho(&self) -> bool {
        self.family.contains("Mincho")
    }
    /// A sans (Gothic) family, for UI text.
    pub fn is_gothic(&self) -> bool {
        self.family.contains("Gothic")
    }
    /// A bold face.
    pub fn is_bold(&self) -> bool {
        crate::style_weight(self.style) >= 600.0
    }
}

/// The craft-fonts faces for Japanese text, best for document text first: Mincho (serif) families,
/// then the others, regular weights before bold. Empty when built without craft-fonts.
pub fn japanese_document_fonts() -> Vec<&'static CraftFont> {
    let mut v: Vec<&'static CraftFont> = CRAFT_FONTS.iter().filter(|f| f.is_japanese()).collect();
    v.sort_by_key(|f| (!f.is_mincho(), f.is_bold()));
    v
}

/// The craft-fonts faces the document font database loads: the Japanese ones in
/// [`japanese_document_fonts`] order, then the Arabic ones in manifest order (Noto Sans Arabic
/// first: the fallback for Arabic text; the other families are picked by name). Empty when built
/// without craft-fonts.
pub fn document_fonts() -> Vec<&'static CraftFont> {
    let mut v = japanese_document_fonts();
    v.extend(CRAFT_FONTS.iter().filter(|f| f.is_arabic() && !f.is_japanese()));
    v
}

/// The craft-fonts faces for Japanese text, best for UI text first: Gothic (sans) families, then
/// the others; `bold` puts bold faces before regular ones. Empty when built without craft-fonts.
pub fn japanese_ui_fonts(bold: bool) -> Vec<&'static CraftFont> {
    let mut v: Vec<&'static CraftFont> = CRAFT_FONTS.iter().filter(|f| f.is_japanese()).collect();
    v.sort_by_key(|f| (!f.is_gothic(), f.is_bold() != bold));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_fonts_follow_the_list() {
        if CRAFT_FONTS.is_empty() {
            eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
            return;
        }
        // The listed faces this craft-fonts checkout has (an older one lacks some; build.rs leaves
        // those out with a warning).
        let listed: Vec<_> = crate::web_fonts_build::parse_list(include_str!("../web-fonts.txt"))
            .unwrap()
            .into_iter()
            .filter(|l| CRAFT_FONTS.iter().any(|f| f.family == l.family && f.style == l.style))
            .collect();
        assert_eq!(WEB_FONTS.len(), listed.len());
        for (l, w) in listed.iter().zip(WEB_FONTS) {
            assert_eq!((l.family, l.style, l.startup), (w.family, w.style, w.startup));
            let (dir, name) = w.url.strip_prefix("fonts/").and_then(|r| r.split_once('/')).unwrap();
            assert!(dir.len() == 16 && dir.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{}", w.url);
            assert!(name.ends_with(".ttf") || name.ends_with(".otf"), "{}", w.url);
            assert!(w.integrity.starts_with("sha256-") && w.integrity.len() == "sha256-".len() + 44, "{}", w.integrity);
        }
        let startup: Vec<_> = WEB_FONTS.iter().filter(|w| w.startup).map(|w| w.family).collect();
        let noto = CRAFT_FONTS.iter().any(|f| f.family == "Noto Sans Arabic");
        assert_eq!(startup, if noto { vec!["Noto Sans Arabic"] } else { vec![] });
    }
}

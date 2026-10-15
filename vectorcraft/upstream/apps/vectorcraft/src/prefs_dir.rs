//! Where VectorCraft keeps its settings, `ui.json`, and next to it folders of the user's own
//! (Swatches, Graphic Styles, Fonts, Data Recovery, logs). The desktop app and `vectorcraft-cli`
//! (which shares this file) read the same Fonts folder.

use std::path::PathBuf;

#[path = "profile.rs"]
mod profile;

/// The settings file `ui.json` of the app `name`: in a folder named `name` in ~/Library/Application
/// Support (macOS) or %APPDATA% (Windows), or `lower` in $XDG_CONFIG_HOME or ~/.config (Linux and
/// BSD).
pub fn prefs_path_for(name: &str, lower: &str) -> Option<PathBuf> {
    let legacy = name == "DrawCraft" || lower == "drawcraft";
    profile::prefs_path(legacy)
}

/// Have the font scans read VectorCraft's own Fonts folder, next to the settings, which
/// `text.addFontFiles` copies font files into. The folder is set whether or not
/// `VECTORCRAFT_NO_PREFS` is set, as the Swatches and Graphic Styles folders are.
pub fn install_fonts() {
    if let Some(dir) = prefs_path_for("VectorCraft", "vectorcraft").and_then(|p| Some(p.parent()?.join("Fonts"))) {
        vectorcraft_text::set_app_font_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_scans_read_the_fonts_folder_next_to_the_settings() {
        install_fonts();
        let fonts = prefs_path_for("VectorCraft", "vectorcraft").and_then(|p| Some(p.parent()?.join("Fonts")));
        assert!(fonts.is_some(), "a home folder to keep the settings in");
        assert_eq!(vectorcraft_text::app_font_dir().map(std::path::Path::to_path_buf), fonts);
        assert!(vectorcraft_text::system_font_dirs().contains(&fonts.unwrap_or_default()));
    }
}

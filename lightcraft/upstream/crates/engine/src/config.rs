//! Where LightCraft keeps per-user data on this machine: the config folder (app settings, camera
//! profiles, face and denoise models).
//!
//! Hosts that have a file system (the desktop app, the CLI, the MCP server) share these defaults so a model
//! installed from one is there for the others. Nothing here is applied automatically: a [`Session`] has no
//! face-, denoise- or SAM 3 model folder until a host asks for one ([`Session::with_default_face_models`],
//! [`Session::with_default_denoise_models`], [`Session::with_default_sam3_model`]), so tests stay hermetic.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::Session;

#[derive(Clone, Copy)]
enum Platform {
    Mac,
    Windows,
    Other,
}

/// LightCraft's settings folder. A nonempty `$LIGHTCRAFT_CONFIG_DIR` overrides the platform default.
/// The override is the complete directory; no application-name suffix is appended. An explicitly
/// empty value disables the settings directory instead of falling back to a personal profile.
pub fn config_dir() -> Option<PathBuf> {
    let platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Other
    };
    config_dir_with(platform, |name| std::env::var_os(name))
}

fn config_dir_with(platform: Platform, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = env("LIGHTCRAFT_CONFIG_DIR") {
        return (!dir.is_empty()).then(|| PathBuf::from(dir));
    }
    match platform {
        Platform::Mac => env("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/LightCraft")),
        Platform::Windows => env("APPDATA").map(|a| PathBuf::from(a).join("LightCraft")),
        Platform::Other => {
            env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config"))).map(|c| c.join("lightcraft"))
        }
    }
}

/// Where face models are kept: `$LIGHTCRAFT_FACE_MODELS` if set, else `<config>/models`.
pub fn default_face_models_dir() -> Option<PathBuf> {
    std::env::var_os("LIGHTCRAFT_FACE_MODELS").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| config_dir().map(|d| d.join("models")))
}

/// Where opt-in denoise models are kept: `$LIGHTCRAFT_DENOISE_MODELS` if set, else `<config>/denoise-models`.
pub fn default_denoise_models_dir() -> Option<PathBuf> {
    std::env::var_os("LIGHTCRAFT_DENOISE_MODELS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| config_dir().map(|d| d.join("denoise-models")))
}

/// Where the SAM 3 model is kept: `$LIGHTCRAFT_SAM3_DIR` if set, else `<config>/models/sam3`.
pub fn default_sam3_dir() -> Option<PathBuf> {
    sam3_dir(std::env::var_os("LIGHTCRAFT_SAM3_DIR"), config_dir())
}

/// [`default_sam3_dir`] for a given override and config folder: a non-empty override wins.
fn sam3_dir(var: Option<std::ffi::OsString>, config: Option<PathBuf>) -> Option<PathBuf> {
    var.filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| config.map(|d| d.join("models").join("sam3")))
}

impl Session {
    /// Look for the SAM 3 model in the shared default folder ([`default_sam3_dir`]), and for the user's
    /// download mirrors in `<config>/models/sam3-mirrors.txt`.
    pub fn with_default_sam3_model(mut self) -> Self {
        self.segmenter.dir = default_sam3_dir();
        self.segmenter.mirrors_file = config_dir().map(|d| d.join("models").join("sam3-mirrors.txt"));
        self
    }

    /// Keep face models in the shared default folder ([`default_face_models_dir`]).
    pub fn with_default_face_models(mut self) -> Self {
        self.face_models_dir = default_face_models_dir();
        self
    }

    /// Keep denoise models in the shared default folder ([`default_denoise_models_dir`]).
    pub fn with_default_denoise_models(mut self) -> Self {
        self.set_denoise_models_dir(default_denoise_models_dir());
        self
    }

    /// Point AI denoise at `dir` (or at no folder), dropping what was learned about the old one.
    pub fn set_denoise_models_dir(&mut self, dir: Option<PathBuf>) {
        self.denoise.models_dir = dir;
        self.denoise.touch();
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn resolve(platform: Platform, vars: &[(&str, &str)]) -> Option<PathBuf> {
        config_dir_with(platform, |name| vars.iter().find_map(|(key, value)| (*key == name).then(|| OsString::from(value))))
    }

    #[test]
    fn config_override_wins_without_reading_user_profile() {
        for platform in [Platform::Mac, Platform::Windows, Platform::Other] {
            let actual = config_dir_with(platform, |name| {
                assert_eq!(name, "LIGHTCRAFT_CONFIG_DIR");
                Some(OsString::from("isolated/settings"))
            });
            assert_eq!(actual, Some(PathBuf::from("isolated/settings")));
        }
    }

    #[test]
    fn config_empty_override_disables_profile_without_reading_defaults() {
        for platform in [Platform::Mac, Platform::Windows, Platform::Other] {
            assert_eq!(
                config_dir_with(platform, |name| {
                    assert_eq!(name, "LIGHTCRAFT_CONFIG_DIR");
                    Some(OsString::new())
                }),
                None
            );
        }
    }

    #[test]
    fn config_unset_override_keeps_platform_defaults() {
        let vars = [("HOME", "/user"), ("APPDATA", "/roaming"), ("XDG_CONFIG_HOME", "/xdg")];
        assert_eq!(resolve(Platform::Mac, &vars), Some(PathBuf::from("/user/Library/Application Support/LightCraft")));
        assert_eq!(resolve(Platform::Windows, &vars), Some(PathBuf::from("/roaming/LightCraft")));
        assert_eq!(resolve(Platform::Other, &vars), Some(PathBuf::from("/xdg/lightcraft")));
        assert_eq!(resolve(Platform::Other, &[("HOME", "/user")]), Some(PathBuf::from("/user/.config/lightcraft")));
        for platform in [Platform::Mac, Platform::Windows, Platform::Other] {
            assert_eq!(resolve(platform, &[]), None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn config_override_preserves_non_unicode_paths() {
        use std::os::unix::ffi::OsStringExt as _;
        let value = OsString::from_vec(b"isolated/\xff".to_vec());
        assert_eq!(config_dir_with(Platform::Mac, |_| Some(value.clone())), Some(PathBuf::from(value)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A non-empty `LIGHTCRAFT_SAM3_DIR` wins; unset or empty falls back to `<config>/models/sam3` (issue #619).
    #[test]
    fn sam3_dir_prefers_the_override() {
        let config = PathBuf::from("cfg");
        let fallback = Some(config.join("models").join("sam3"));
        assert_eq!(sam3_dir(Some("elsewhere".into()), Some(config.clone())), Some(PathBuf::from("elsewhere")));
        assert_eq!(sam3_dir(None, Some(config.clone())), fallback);
        assert_eq!(sam3_dir(Some(std::ffi::OsString::new()), Some(config)), fallback);
        assert_eq!(sam3_dir(None, None), None);
    }
}

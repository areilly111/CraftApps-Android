//! Native profile paths. Isolated profiles never migrate preferences from a user's old profile.

use std::ffi::OsString;
use std::path::PathBuf;

pub fn prefs_path(legacy: bool) -> Option<PathBuf> {
    prefs_path_with(std::env::consts::OS, legacy, |key| std::env::var_os(key))
}

fn prefs_path_with(os: &str, legacy: bool, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(root) = env("VECTORCRAFT_CONFIG_DIR") {
        // Never fall back to DrawCraft's real profile when the isolated file does not exist.
        return (!legacy && !root.is_empty()).then(|| PathBuf::from(root).join("ui.json"));
    }
    let (name, lower) = if legacy { ("DrawCraft", "drawcraft") } else { ("VectorCraft", "vectorcraft") };
    let base = match os {
        "macos" => env("HOME").map(|p| PathBuf::from(p).join("Library/Application Support").join(name)),
        "windows" => env("APPDATA").map(|p| PathBuf::from(p).join(name)),
        _ => env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| env("HOME").map(|p| PathBuf::from(p).join(".config"))).map(|p| p.join(lower)),
    };
    base.map(|p| p.join("ui.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(key: &str) -> Option<OsString> {
        match key {
            "HOME" => Some("/user".into()),
            "APPDATA" => Some("/roaming".into()),
            "XDG_CONFIG_HOME" => Some("/xdg-config".into()),
            _ => None,
        }
    }

    #[test]
    fn isolated_profile_has_no_legacy_fallback_on_any_platform() {
        for os in ["macos", "windows", "linux"] {
            let env = |key: &str| {
                if key == "VECTORCRAFT_CONFIG_DIR" { Some("/isolated".into()) } else { environment(key) }
            };
            assert_eq!(prefs_path_with(os, false, env), Some("/isolated/ui.json".into()));
            assert_eq!(prefs_path_with(os, true, env), None);
        }
    }

    #[test]
    fn empty_override_does_not_fall_back_to_user_files() {
        let env = |key: &str| if key == "VECTORCRAFT_CONFIG_DIR" { Some(OsString::new()) } else { environment(key) };
        assert_eq!(prefs_path_with("macos", false, env), None);
        assert_eq!(prefs_path_with("macos", true, env), None);
    }

    #[test]
    fn unset_override_keeps_platform_paths_and_legacy_migration() {
        assert_eq!(prefs_path_with("macos", false, environment), Some("/user/Library/Application Support/VectorCraft/ui.json".into()));
        assert_eq!(prefs_path_with("macos", true, environment), Some("/user/Library/Application Support/DrawCraft/ui.json".into()));
        assert_eq!(prefs_path_with("windows", false, environment), Some("/roaming/VectorCraft/ui.json".into()));
        assert_eq!(prefs_path_with("linux", false, environment), Some("/xdg-config/vectorcraft/ui.json".into()));
        assert_eq!(
            prefs_path_with("linux", true, |key| if key == "HOME" { environment(key) } else { None }),
            Some("/user/.config/drawcraft/ui.json".into())
        );
    }
}

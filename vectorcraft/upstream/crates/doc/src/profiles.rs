//! The colour profiles a document is tagged with (Edit → Assign Profile).

use serde::{Deserialize, Serialize};

use crate::Document;

/// `Document::unknown` key under which files before format v3 kept the assigned profiles
/// (`{rgb?, cmyk?}`); still written when saving for those versions.
pub const LEGACY_PROFILES_KEY: &str = "colorProfiles";

/// The profiles a document is tagged with, by name (`None`: the working space).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorProfiles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmyk: Option<String>,
}

impl ColorProfiles {
    pub fn is_empty(&self) -> bool {
        self.rgb.is_none() && self.cmyk.is_none()
    }

    /// The legacy JSON form (`{"rgb": name|null, "cmyk": name|null}`).
    pub fn to_legacy(&self) -> serde_json::Value {
        serde_json::json!({ "rgb": self.rgb, "cmyk": self.cmyk })
    }

    /// Read the legacy JSON form (anything that isn't a name counts as none).
    pub fn from_legacy(v: &serde_json::Value) -> Self {
        let name = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
        Self { rgb: name("rgb"), cmyk: name("cmyk") }
    }
}

impl Document {
    /// Files before format v3 kept the assigned profiles under [`LEGACY_PROFILES_KEY`] in
    /// `unknown`: move them to [`Document::color_profiles`].
    pub fn migrate_color_profiles(&mut self) {
        let Some(v) = self.unknown.remove(LEGACY_PROFILES_KEY) else { return };
        if self.color_profiles.is_empty() {
            self.color_profiles = ColorProfiles::from_legacy(&v);
        }
    }

    /// The reverse of [`Document::migrate_color_profiles`], for saving in a version before v3.
    pub fn store_legacy_color_profiles(&mut self) {
        let profiles = std::mem::take(&mut self.color_profiles);
        if !profiles.is_empty() {
            self.unknown.insert(LEGACY_PROFILES_KEY.into(), profiles.to_legacy());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_profiles_migrate_both_ways() {
        let mut d = Document::new(10.0, 10.0);
        d.unknown.insert(LEGACY_PROFILES_KEY.into(), serde_json::json!({"rgb": "Wide RGB", "cmyk": null}));
        d.migrate_color_profiles();
        assert_eq!(d.color_profiles, ColorProfiles { rgb: Some("Wide RGB".into()), cmyk: None });
        assert!(!d.unknown.contains_key(LEGACY_PROFILES_KEY));
        d.store_legacy_color_profiles();
        assert!(d.color_profiles.is_empty());
        assert_eq!(d.unknown[LEGACY_PROFILES_KEY], serde_json::json!({"rgb": "Wide RGB", "cmyk": null}));
        // Nothing assigned: nothing stored.
        let mut e = Document::new(10.0, 10.0);
        e.store_legacy_color_profiles();
        assert!(e.unknown.is_empty());
    }
}

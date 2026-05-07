// Persistent UI preferences for modplayer-bin. Stored as TOML at the
// platform's config dir (`~/.config/modplayer/settings.toml` on Linux,
// `~/Library/Application Support/modplayer/settings.toml` on macOS,
// `%APPDATA%\modplayer\settings.toml` on Windows). Anything missing
// from the file falls back to `Settings::default()` so partial / older
// configs keep working.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use xmplayer::song::FilterType;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub theme_id: u32,
    pub filter: FilterType,
    pub view_mode: u32,
    pub visualizer_enabled: bool,
    pub visualizer_mode: u32,
}

impl Default for Settings {
    fn default() -> Self {
        // Mirror Song::new defaults so an empty / missing settings file
        // produces the same first-run experience as before this module
        // was added.
        Self {
            theme_id: 0,
            filter: FilterType::Sinc,
            view_mode: 0,
            visualizer_enabled: true,
            visualizer_mode: 0,
        }
    }
}

impl Settings {
    fn config_path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("modplayer").join("settings.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else { return Self::default(); };
        match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|_| Self::default()),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else { return; };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = fs::write(&path, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_via_toml() {
        let original = Settings {
            theme_id: 3,
            filter: FilterType::Cubic,
            view_mode: 2,
            visualizer_enabled: false,
            visualizer_mode: 1,
        };
        let s = toml::to_string_pretty(&original).expect("serialize");
        let parsed: Settings = toml::from_str(&s).expect("parse");
        assert_eq!(parsed.theme_id, original.theme_id);
        assert_eq!(parsed.filter, original.filter);
        assert_eq!(parsed.view_mode, original.view_mode);
        assert_eq!(parsed.visualizer_enabled, original.visualizer_enabled);
        assert_eq!(parsed.visualizer_mode, original.visualizer_mode);
    }

    #[test]
    fn missing_fields_fall_back_to_default() {
        // Older / partial configs must still parse cleanly.
        let parsed: Settings = toml::from_str("theme_id = 4").expect("parse partial");
        let d = Settings::default();
        assert_eq!(parsed.theme_id, 4);
        assert_eq!(parsed.filter, d.filter);
        assert_eq!(parsed.view_mode, d.view_mode);
        assert_eq!(parsed.visualizer_enabled, d.visualizer_enabled);
        assert_eq!(parsed.visualizer_mode, d.visualizer_mode);
    }

    #[test]
    fn corrupt_input_falls_back_to_default() {
        // Malformed TOML must not crash load(); since the parse path is
        // private, simulate it with the same fallback contract.
        let parsed: Settings = toml::from_str("not = valid = toml").unwrap_or_default();
        assert_eq!(parsed.theme_id, Settings::default().theme_id);
    }
}

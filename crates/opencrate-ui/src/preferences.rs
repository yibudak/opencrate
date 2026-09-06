//! User preferences and the last successfully applied lighting configuration.

use opencrate_aura::{
    animation::{MAX_SPEED, MIN_SPEED},
    playback::Settings,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLighting {
    mode: String,
    color: String,
    speed: f64,
    brightness: u8,
}

impl From<Settings> for SavedLighting {
    fn from(settings: Settings) -> Self {
        Self {
            mode: settings.mode.name().into(),
            color: settings.color.to_hex(),
            speed: settings.speed,
            brightness: settings.brightness,
        }
    }
}

impl SavedLighting {
    pub fn settings(&self) -> Result<Settings, String> {
        if !self.speed.is_finite()
            || !(MIN_SPEED..=MAX_SPEED).contains(&self.speed)
            || self.brightness > 100
        {
            return Err("Saved speed or brightness is out of range".into());
        }
        Ok(Settings {
            mode: self
                .mode
                .parse()
                .map_err(|e| format!("Saved effect: {e}"))?,
            color: self
                .color
                .parse()
                .map_err(|e| format!("Saved color: {e}"))?,
            speed: self.speed,
            brightness: self.brightness,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    version: u32,
    pub language: crate::i18n::Language,
    pub restore_lighting: bool,
    pub start_in_tray: bool,
    pub last_lighting: Option<SavedLighting>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            language: crate::i18n::Language::English,
            restore_lighting: true,
            start_in_tray: true,
            last_lighting: None,
        }
    }
}

impl Preferences {
    fn parse(json: &str) -> Result<Self, String> {
        let preferences: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if preferences.version != 1 {
            return Err("Unsupported settings file version".into());
        }
        if let Some(last) = &preferences.last_lighting {
            last.settings()?;
        }
        Ok(preferences)
    }

    pub fn lighting(&self) -> Option<Settings> {
        self.last_lighting
            .as_ref()
            .and_then(|last| last.settings().ok())
    }

    pub fn restore_on_launch(&self) -> Option<Settings> {
        self.restore_lighting.then(|| self.lighting()).flatten()
    }
}

pub struct Store {
    pub preferences: Preferences,
    pub error: Option<String>,
    path: Option<PathBuf>,
    save_at: Option<Instant>,
}

impl Store {
    pub fn load() -> Self {
        let Some(base) = std::env::var_os("APPDATA") else {
            return Self {
                preferences: Preferences::default(),
                error: Some("Cannot locate the settings folder (APPDATA is missing).".into()),
                path: None,
                save_at: None,
            };
        };
        Self::load_path(PathBuf::from(base).join("opencrate").join("settings.json"))
    }

    pub(crate) fn load_path(path: PathBuf) -> Self {
        let result = match fs::read_to_string(&path) {
            Ok(json) => Preferences::parse(&json),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Preferences::default()),
            Err(error) => Err(error.to_string()),
        };
        let (preferences, error) = match result {
            Ok(preferences) => (preferences, None),
            Err(error) => (
                Preferences::default(),
                Some(format!("Could not load settings: {error}")),
            ),
        };
        Self {
            preferences,
            error,
            path: Some(path),
            save_at: None,
        }
    }

    pub fn changed(&mut self) {
        self.save_at = Some(Instant::now() + Duration::from_millis(400));
    }

    pub fn pending_delay(&self) -> Option<Duration> {
        self.save_at
            .map(|due| due.saturating_duration_since(Instant::now()))
    }

    pub fn flush_if_due(&mut self) {
        if self.save_at.is_some_and(|due| Instant::now() >= due) {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        if self.save_at.take().is_none() {
            return;
        }
        let result = self
            .path
            .as_ref()
            .ok_or_else(|| io::Error::other("Settings folder unavailable"))
            .and_then(|path| save_atomic(path, &self.preferences));
        self.error = result
            .err()
            .map(|error| format!("Could not save settings: {error}"));
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.flush();
    }
}

fn save_atomic(path: &Path, preferences: &Preferences) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(preferences)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid settings path"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&json)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencrate_core::{EffectMode, RgbColor};

    #[test]
    fn language_migration_preserves_existing_hardware_preferences() {
        let legacy = r#"{"version":1,"restore_lighting":false,"start_in_tray":true,"last_lighting":{"mode":"static","color":"123456","speed":1.5,"brightness":42}}"#;
        let original = Preferences::parse(legacy).unwrap();
        assert_eq!(original.language, crate::i18n::Language::English);
        for (code, language) in [
            ("en", crate::i18n::Language::English),
            ("zh-CN", crate::i18n::Language::Chinese),
            ("tr", crate::i18n::Language::Turkish),
            ("future", crate::i18n::Language::English),
        ] {
            let mut json: serde_json::Value = serde_json::from_str(legacy).unwrap();
            json["language"] = code.into();
            let mut decoded = Preferences::parse(&json.to_string()).unwrap();
            assert_eq!(decoded.language, language);
            assert_eq!(decoded.lighting(), original.lighting());
            assert_eq!(decoded.restore_on_launch(), None);
            assert_eq!(
                Preferences::parse(&serde_json::to_string(&decoded).unwrap()).unwrap(),
                decoded
            );
            decoded.language = original.language;
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn lighting_roundtrip_preserves_all_fields_and_restore_can_be_disabled() {
        for &mode in EffectMode::all() {
            let settings = Settings {
                mode,
                color: RgbColor::new(18, 52, 86),
                speed: 0.25,
                brightness: 0,
            };
            let mut prefs = Preferences {
                last_lighting: Some(settings.into()),
                ..Preferences::default()
            };
            let restored = Preferences::parse(&serde_json::to_string(&prefs).unwrap()).unwrap();
            assert_eq!(restored.restore_on_launch(), Some(settings));
            prefs.restore_lighting = false;
            assert_eq!(prefs.lighting(), Some(settings));
            assert_eq!(prefs.restore_on_launch(), None);
        }
    }

    #[test]
    fn invalid_saved_settings_cannot_reach_hardware() {
        for json in [
            "{broken",
            r#"{"version":2}"#,
            r#"{"last_lighting":{"mode":"removed-effect","color":"FF0000","speed":1.0,"brightness":50}}"#,
            r#"{"last_lighting":{"mode":"static","color":"no-color","speed":1.0,"brightness":50}}"#,
            r#"{"last_lighting":{"mode":"static","color":"FF0000","speed":0.0,"brightness":50}}"#,
            r#"{"last_lighting":{"mode":"static","color":"FF0000","speed":1.0,"brightness":101}}"#,
        ] {
            assert!(Preferences::parse(json).is_err(), "{json}");
        }
        assert_eq!(Preferences::parse("{}").unwrap().restore_on_launch(), None);
    }

    #[test]
    fn saving_replaces_existing_file_and_unreadable_file_is_not_overwritten_on_load() {
        let directory =
            std::env::temp_dir().join(format!("opencrate-settings-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        let mut store = Store::load_path(path.clone());
        store.changed();
        store.flush();
        assert!(store.error.is_none());
        store.preferences.start_in_tray = false;
        store.preferences.language = crate::i18n::Language::Chinese;
        store.changed();
        store.flush();
        assert!(!Store::load_path(path.clone()).preferences.start_in_tray);
        assert_eq!(
            Store::load_path(path.clone()).preferences.language,
            crate::i18n::Language::Chinese
        );
        fs::write(&path, "broken settings").unwrap();
        let broken = Store::load_path(path.clone());
        assert!(broken.error.is_some());
        assert!(broken.preferences.restore_on_launch().is_none());
        drop(broken);
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken settings");
        drop(store);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}

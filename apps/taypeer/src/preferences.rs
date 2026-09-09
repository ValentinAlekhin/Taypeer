use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub language: String,
    pub theme: String,
    pub font_size: u8,
    pub group_width: f32,
    pub entry_width: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: "en".into(),
            theme: "system".into(),
            font_size: 16,
            group_width: 224.,
            entry_width: 480.,
        }
    }
}
impl Preferences {
    fn path() -> Result<PathBuf, ()> {
        let home = std::env::var_os("HOME").ok_or(())?;
        Ok(PathBuf::from(home).join("Library/Application Support/Taypeer/preferences.toml"))
    }
    pub fn load() -> (Self, bool) {
        match Self::path().and_then(|path| Self::load_from(&path)) {
            Ok(prefs) => (prefs, false),
            Err(()) => (Self::default(), true),
        }
    }
    fn load_from(path: &Path) -> Result<Self, ()> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err(()),
        };
        let prefs: Self = toml::from_str(&text).map_err(|_| ())?;
        prefs.validate()?;
        Ok(prefs)
    }
    fn validate(&self) -> Result<(), ()> {
        if !["en", "ru"].contains(&self.language.as_str())
            || !["system", "light", "dark"].contains(&self.theme.as_str())
            || ![14, 16, 18].contains(&self.font_size)
            || !self.group_width.is_finite()
            || !self.entry_width.is_finite()
            || !(192.0..=280.0).contains(&self.group_width)
            || !(380.0..=560.0).contains(&self.entry_width)
        {
            Err(())
        } else {
            Ok(())
        }
    }
    pub fn save(&self) -> Result<(), ()> {
        self.save_to(&Self::path()?)
    }
    fn save_to(&self, path: &Path) -> Result<(), ()> {
        self.validate()?;
        // Never overwrite a newer/invalid file, including one changed since startup.
        Self::load_from(path)?;
        std::fs::create_dir_all(path.parent().ok_or(())?).map_err(|_| ())?;
        let data = toml::to_string(self).map_err(|_| ())?;
        let temporary = path.with_extension("toml.new");
        std::fs::write(&temporary, data).map_err(|_| ())?;
        std::fs::rename(temporary, path).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "taypeer-preferences-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf {
            self.0.join("preferences.toml")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_preferences_use_defaults_and_system_choice_roundtrips() {
        let directory = Directory::new();
        let path = directory.file();
        let mut prefs = Preferences::load_from(&path).unwrap();
        assert_eq!(prefs.language, "en");
        assert_eq!(prefs.theme, "system");
        prefs.language = "ru".into();
        prefs.font_size = 18;
        prefs.save_to(&path).unwrap();
        let loaded = Preferences::load_from(&path).unwrap();
        assert_eq!(loaded.language, "ru");
        assert_eq!(loaded.theme, "system");
        assert_eq!(loaded.font_size, 18);
    }

    #[test]
    fn corrupt_unknown_and_out_of_range_preferences_are_not_overwritten() {
        let directory = Directory::new();
        let path = directory.file();
        let baseline = toml::to_string(&Preferences::default()).unwrap();
        for contents in [
            "not valid TOML".into(),
            format!("{baseline}\nfuture_setting = true\n"),
            baseline.replace("font_size = 16", "font_size = 99"),
            baseline.replace("group_width = 224.0", "group_width = nan"),
        ] {
            std::fs::write(&path, &contents).unwrap();
            assert!(Preferences::load_from(&path).is_err());
            assert!(Preferences::default().save_to(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn failed_preference_write_retains_the_previous_file() {
        let directory = Directory::new();
        let path = directory.file();
        let mut prefs = Preferences::default();
        prefs.save_to(&path).unwrap();
        let previous = std::fs::read_to_string(&path).unwrap();
        std::fs::create_dir(path.with_extension("toml.new")).unwrap();
        prefs.theme = "dark".into();
        assert!(prefs.save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), previous);
    }

    #[test]
    fn embedded_palettes_and_translations_cover_the_same_keys() {
        use std::collections::BTreeMap;
        let palette: BTreeMap<String, BTreeMap<String, u32>> =
            toml::from_str(include_str!("../../../resources/theme.toml")).unwrap();
        assert_eq!(
            palette["light"].keys().collect::<Vec<_>>(),
            palette["dark"].keys().collect::<Vec<_>>()
        );
        assert!(
            palette
                .values()
                .flat_map(|p| p.values())
                .all(|color| *color <= 0xffffff)
        );
        for role in [
            "background",
            "panel",
            "chrome",
            "foreground",
            "muted",
            "border",
            "primary",
            "on_primary",
            "selection",
            "focus",
            "danger",
            "warning",
            "success",
            "disabled",
        ] {
            assert!(palette["light"].contains_key(role));
        }
        let english: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/en.json")).unwrap();
        let russian: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/ru.json")).unwrap();
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            russian.keys().collect::<Vec<_>>()
        );
        assert!(
            english
                .values()
                .chain(russian.values())
                .all(|value| !value.is_empty())
        );
    }
}

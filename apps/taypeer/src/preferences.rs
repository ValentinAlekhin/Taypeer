mod options;

pub use options::{FONT_SIZES, Language, ThemePreference};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub language: Language,
    pub theme: ThemePreference,
    pub font_size: u8,
    pub group_width: f32,
    pub entry_width: f32,
    #[serde(default = "default_inspector_width")]
    pub inspector_width: f32,
}
fn default_inspector_width() -> f32 {
    520.
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: Language::default(),
            theme: ThemePreference::default(),
            font_size: 16,
            group_width: 224.,
            entry_width: 480.,
            inspector_width: default_inspector_width(),
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
    pub(crate) fn load_from(path: &Path) -> Result<Self, ()> {
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
        if !FONT_SIZES.contains(&self.font_size)
            || !self.group_width.is_finite()
            || !self.entry_width.is_finite()
            || !(192.0..=280.0).contains(&self.group_width)
            || !(330.0..=10000.0).contains(&self.entry_width)
            || !self.inspector_width.is_finite()
            || !(380.0..=10000.0).contains(&self.inspector_width)
        {
            Err(())
        } else {
            Ok(())
        }
    }
    pub fn save(&self) -> Result<(), ()> {
        self.save_to(&Self::path()?)
    }
    pub(crate) fn save_to(&self, path: &Path) -> Result<(), ()> {
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
        assert_eq!(prefs.language, Language::English);
        assert_eq!(prefs.theme, ThemePreference::System);
        prefs.language = Language::Russian;
        prefs.font_size = 18;
        prefs.save_to(&path).unwrap();
        let loaded = Preferences::load_from(&path).unwrap();
        assert_eq!(loaded.language, Language::Russian);
        assert_eq!(loaded.theme, ThemePreference::System);
        assert_eq!(loaded.font_size, 18);
    }

    #[test]
    fn typed_options_read_and_write_the_existing_toml_keys() {
        let directory = Directory::new();
        let path = directory.file();
        for (language, code) in [(Language::English, "en"), (Language::Russian, "ru")] {
            for (theme, key) in [
                (ThemePreference::System, "system"),
                (ThemePreference::Light, "light"),
                (ThemePreference::Dark, "dark"),
            ] {
                // Literal fixture models the file produced before typed preferences.
                let previous = format!(
                    "language = \"{code}\"\ntheme = \"{key}\"\nfont_size = 16\ngroup_width = 224.0\nentry_width = 480.0\n"
                );
                std::fs::write(&path, &previous).unwrap();
                let loaded = Preferences::load_from(&path).unwrap();
                assert_eq!(loaded.language, language);
                assert_eq!(loaded.theme, theme);
                loaded.save_to(&path).unwrap();
                assert_eq!(Preferences::load_from(&path).unwrap().inspector_width, 520.);
            }
        }
    }

    #[test]
    fn only_system_theme_follows_the_os_appearance() {
        for system_is_dark in [false, true] {
            assert_eq!(
                ThemePreference::System.is_dark(system_is_dark),
                system_is_dark
            );
            assert!(!ThemePreference::Light.is_dark(system_is_dark));
            assert!(ThemePreference::Dark.is_dark(system_is_dark));
        }
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
            baseline.replace("language = \"en\"", "language = \"future-language\""),
            baseline.replace("theme = \"system\"", "theme = \"future-theme\""),
        ] {
            std::fs::write(&path, &contents).unwrap();
            assert!(Preferences::load_from(&path).is_err());
            assert!(Preferences::default().save_to(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn inspector_width_roundtrips_and_invalid_width_does_not_replace_preferences() {
        let directory = Directory::new();
        let path = directory.file();
        let mut prefs = Preferences {
            inspector_width: 740.,
            entry_width: 1800.,
            ..Default::default()
        };
        prefs.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path).unwrap().inspector_width, 740.);
        let previous = std::fs::read_to_string(&path).unwrap();
        for invalid in [f32::NAN, f32::INFINITY, 0., 379.] {
            prefs.inspector_width = invalid;
            assert!(prefs.save_to(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), previous);
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
        prefs.theme = ThemePreference::Dark;
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

//! Supported preference values and their stable on-disk keys.

use serde::{Deserialize, Serialize};

/// Supported pixel anchors for the application zoom scale.
pub const FONT_SIZES: [u8; 3] = [14, 16, 18];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "&'static str", try_from = "String")]
/// Supported interface locales with stable persisted keys.
pub enum Language {
    #[default]
    /// English interface.
    English,
    /// Russian interface.
    Russian,
}

impl Language {
    /// Locales in settings display order.
    pub const ALL: [Self; 2] = [Self::English, Self::Russian];

    /// Stable locale and persistence key.
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Russian => "ru",
        }
    }

    /// Native language name shown in the language selector.
    pub const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Russian => "Русский",
        }
    }
}

impl From<Language> for &'static str {
    fn from(value: Language) -> Self {
        value.code()
    }
}

impl TryFrom<String> for Language {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == value)
            .ok_or("unsupported language")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "&'static str", try_from = "String")]
/// Appearance policy independent of the current operating-system appearance.
pub enum ThemePreference {
    #[default]
    /// Follow the current system appearance.
    System,
    /// Use the light palette.
    Light,
    /// Use the dark palette.
    Dark,
}

impl ThemePreference {
    /// Theme choices in settings display order.
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    /// Stable persisted theme key.
    pub const fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Resolve the effective appearance from the selected policy.
    pub const fn is_dark(self, system_is_dark: bool) -> bool {
        match self {
            Self::System => system_is_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

impl From<ThemePreference> for &'static str {
    fn from(value: ThemePreference) -> Self {
        value.key()
    }
}

impl TryFrom<String> for ThemePreference {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|theme| theme.key() == value)
            .ok_or("unsupported theme")
    }
}

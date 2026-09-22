//! Compose consumes the same embedded semantic colors as desktop.
use std::sync::OnceLock;

/// Semantic RGB roles from resources/theme.toml; alpha is supplied by the UI.
#[derive(Clone, serde::Deserialize, uniffi::Record)]
pub struct Palette {
    /// Window background.
    pub background: u32,
    /// Content panel.
    pub panel: u32,
    /// Navigation chrome.
    pub chrome: u32,
    /// Primary text.
    pub foreground: u32,
    /// Secondary text.
    pub muted: u32,
    /// Component outlines.
    pub border: u32,
    /// Primary action.
    pub primary: u32,
    /// Text on primary actions.
    pub on_primary: u32,
    /// Selection background.
    pub selection: u32,
    /// Focus outline.
    pub focus: u32,
    /// Error content.
    pub danger: u32,
    /// Warning content.
    pub warning: u32,
    /// Success content.
    pub success: u32,
    /// Disabled content.
    pub disabled: u32,
}
#[derive(serde::Deserialize)]
struct Palettes {
    light: Palette,
    dark: Palette,
}

/// Read the bundled product palette without filesystem or network access.
#[uniffi::export]
pub fn palette(dark: bool) -> Palette {
    static PALETTES: OnceLock<Palettes> = OnceLock::new();
    let palettes = PALETTES.get_or_init(|| {
        toml::from_str(include_str!("../../../../resources/theme.toml"))
            .expect("the bundled product palette is validated by tests")
    });
    if dark {
        palettes.dark.clone()
    } else {
        palettes.light.clone()
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn embedded_palettes_are_valid_rgb() {
        for dark in [false, true] {
            let palette = super::palette(dark);
            assert!(palette.background <= 0xffffff);
            assert!(palette.foreground <= 0xffffff);
            assert_ne!(palette.background, palette.foreground);
        }
    }
}

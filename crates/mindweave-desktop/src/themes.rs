//! Bundled Mindweave themes and helpers to load and apply them.

use gpui_kit::component::{Theme, ThemeRegistry};
use gpui_kit::{App, SharedString};

/// The bundled theme set, in gpui-component's `ThemeSet` JSON schema.
pub const THEMES_JSON: &str = include_str!("themes/mindweave.json");

/// The theme used when a requested theme is not registered.
pub const DEFAULT_THEME: &str = "Mindweave One Dark";

/// Registers the bundled themes with the global theme registry.
pub fn init_themes(cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(THEMES_JSON)
        .expect("bundled themes are valid");
}

/// Names of all registered themes, sorted case-insensitively.
pub fn theme_names(cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = ThemeRegistry::global(cx).themes().keys().cloned().collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// Makes the named theme active, falling back to [`DEFAULT_THEME`] when the
/// name is not registered, and refreshes every window.
pub fn apply_theme(name: &str, cx: &mut App) {
    let themes = ThemeRegistry::global(cx).themes();
    let Some(config) = themes
        .get(name)
        .or_else(|| themes.get(DEFAULT_THEME))
        .cloned()
    else {
        return;
    };
    // `Theme::update` runs `apply_config`, keeps the Base layer's copy of the
    // theme in step, and calls `cx.refresh_windows()`.
    Theme::update(cx, |theme| theme.apply_config(&config));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{ThemeMode, ThemeSet};

    #[test]
    fn bundled_themes_parse() {
        let set: ThemeSet = serde_json::from_str(THEMES_JSON).expect("bundled themes parse");
        let names: Vec<&str> = set.themes.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "Mindweave One Dark",
                "Mindweave Graphite",
                "Mindweave Light"
            ]
        );
        let light: Vec<&str> = set
            .themes
            .iter()
            .filter(|t| t.mode == ThemeMode::Light)
            .map(|t| t.name.as_ref())
            .collect();
        assert_eq!(light, ["Mindweave Light"]);
    }

    #[test]
    fn bundled_themes_use_compact_metrics() {
        let set: ThemeSet = serde_json::from_str(THEMES_JSON).expect("bundled themes parse");
        for theme in &set.themes {
            assert_eq!(theme.font_size, Some(13.0), "{}", theme.name);
            assert_eq!(theme.radius, Some(4), "{}", theme.name);
        }
    }
}

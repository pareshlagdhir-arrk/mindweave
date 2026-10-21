//! Application settings and the recent-files list, persisted as JSON.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::color::normalize_hex_color;

pub const MAX_RECENT: usize = 20;
/// How many recently picked colours the colour palette remembers.
pub const MAX_RECENT_COLORS: usize = 8;
/// How many recently picked font families the font list shows first.
pub const MAX_RECENT_FONTS: usize = 5;
const DEFAULT_THEME: &str = "Mindweave One Dark";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentFile {
    pub path: PathBuf,
    /// Unix seconds.
    pub opened_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: String,
    pub recent: Vec<RecentFile>,
    pub dashboard_window: Option<WindowSize>,
    pub editor_window: Option<WindowSize>,
    pub show_layers: bool,
    pub show_properties: bool,
    /// Colours picked in the colour palette, newest first, as uppercase
    /// `#RRGGBB`; at most [`MAX_RECENT_COLORS`].
    pub recent_colors: Vec<String>,
    /// Font families picked in the Text section's font list, newest
    /// first; at most [`MAX_RECENT_FONTS`].
    pub recent_fonts: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME.to_string(),
            recent: Vec::new(),
            dashboard_window: None,
            editor_window: None,
            show_layers: true,
            show_properties: true,
            recent_colors: Vec::new(),
            recent_fonts: Vec::new(),
        }
    }
}

fn normalized(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl AppSettings {
    /// Moves or inserts `path` at the front of the recent list.
    pub fn touch_recent(&mut self, path: &Path, now: u64) {
        let key = normalized(path);
        self.recent.retain(|r| normalized(&r.path) != key);
        self.recent.insert(
            0,
            RecentFile {
                path: path.to_path_buf(),
                opened_at: now,
            },
        );
        self.recent.truncate(MAX_RECENT);
    }

    pub fn remove_recent(&mut self, path: &Path) {
        let key = normalized(path);
        self.recent.retain(|r| normalized(&r.path) != key);
    }

    /// Points the recent entry for `old` at `new`, keeping its place in the
    /// list and its `opened_at`.
    pub fn rename_recent(&mut self, old: &Path, new: &Path) {
        let key = normalized(old);
        let Some(ix) = self.recent.iter().position(|r| normalized(&r.path) == key) else {
            return;
        };
        self.recent[ix].path = new.to_path_buf();
        // Drop any other (stale) entry for the new path.
        let new_key = normalized(new);
        let mut i = 0;
        self.recent.retain(|r| {
            let keep = i == ix || normalized(&r.path) != new_key;
            i += 1;
            keep
        });
    }

    /// Moves or inserts `color` at the front of the recent colours,
    /// dropping the oldest beyond [`MAX_RECENT_COLORS`]. Text that is not a
    /// `#RRGGBB` colour is ignored.
    pub fn push_recent_color(&mut self, color: &str) {
        let Some(color) = normalize_hex_color(color) else {
            return;
        };
        self.recent_colors.retain(|c| *c != color);
        self.recent_colors.insert(0, color);
        self.recent_colors.truncate(MAX_RECENT_COLORS);
    }

    /// Moves or inserts font family `family` at the front of the recent
    /// fonts, dropping the oldest beyond [`MAX_RECENT_FONTS`]. A blank name
    /// is ignored.
    pub fn push_recent_font(&mut self, family: &str) {
        if family.trim().is_empty() {
            return;
        }
        self.recent_fonts.retain(|f| f != family);
        self.recent_fonts.insert(0, family.to_string());
        self.recent_fonts.truncate(MAX_RECENT_FONTS);
    }

    pub fn recent_existing(&self) -> Vec<&RecentFile> {
        self.recent.iter().filter(|r| r.path.exists()).collect()
    }
}

pub fn settings_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Mindweave")
        .join("settings.json")
}

/// Missing or corrupt files yield the defaults; never errors.
pub fn load_settings(path: &Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &AppSettings, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &std::path::Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "x").unwrap();
        p
    }

    #[test]
    fn touch_recent_moves_to_front_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a.mindweave");
        let b = touch(dir.path(), "b.mindweave");
        let mut s = AppSettings::default();
        s.touch_recent(&a, 1);
        s.touch_recent(&b, 2);
        s.touch_recent(&dir.path().join(".").join("a.mindweave"), 3);
        assert_eq!(s.recent.len(), 2);
        assert_eq!(s.recent[0].opened_at, 3);
        assert_eq!(s.recent[1].opened_at, 2);
    }

    #[test]
    fn recent_is_capped_at_20() {
        let mut s = AppSettings::default();
        for i in 0..25 {
            s.touch_recent(Path::new(&format!("/nonexistent/{i}.mindweave")), i);
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0].opened_at, 24);
    }

    #[test]
    fn recent_existing_hides_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a.mindweave");
        let mut s = AppSettings::default();
        s.touch_recent(&a, 1);
        s.touch_recent(&dir.path().join("gone.mindweave"), 2);
        assert_eq!(s.recent.len(), 2);
        let existing = s.recent_existing();
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].opened_at, 1);
    }

    #[test]
    fn remove_recent_drops_entry() {
        let mut s = AppSettings::default();
        s.touch_recent(Path::new("/nonexistent/a"), 1);
        s.remove_recent(Path::new("/nonexistent/a"));
        assert!(s.recent.is_empty());
    }

    #[test]
    fn rename_recent_keeps_position_and_opened_at() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a.mindweave");
        let mut s = AppSettings::default();
        s.touch_recent(Path::new("/nonexistent/c"), 3);
        s.touch_recent(&a, 2);
        s.touch_recent(Path::new("/nonexistent/b"), 1);
        // A roundabout spelling of `a` still matches its entry.
        let renamed = dir.path().join("renamed.mindweave");
        s.rename_recent(&dir.path().join(".").join("a.mindweave"), &renamed);
        let paths: Vec<_> = s.recent.iter().map(|r| r.path.clone()).collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("/nonexistent/b"),
                renamed.clone(),
                PathBuf::from("/nonexistent/c")
            ]
        );
        assert_eq!(s.recent[1].opened_at, 2);
        // A stale entry already holding the new path is dropped, leaving one
        // entry for the file, where the renamed one was.
        let again = dir.path().join("again.mindweave");
        s.touch_recent(&again, 9);
        s.rename_recent(&renamed, &again);
        let paths: Vec<_> = s.recent.iter().map(|r| r.path.clone()).collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("/nonexistent/b"),
                again.clone(),
                PathBuf::from("/nonexistent/c")
            ]
        );
        assert_eq!(s.recent[1].opened_at, 2);
        // An unknown path changes nothing.
        let before = s.recent.clone();
        s.rename_recent(Path::new("/nonexistent/x"), Path::new("/nonexistent/y"));
        assert_eq!(s.recent, before);
    }

    #[test]
    fn recent_colors_dedupe_and_cap() {
        let mut s = AppSettings::default();
        assert!(s.recent_colors.is_empty());
        s.push_recent_color("#ff0000");
        s.push_recent_color("#00FF00");
        // The same colour in another spelling moves to the front, once.
        s.push_recent_color("FF0000");
        assert_eq!(s.recent_colors, ["#FF0000", "#00FF00"]);
        // Text that is not a colour is ignored.
        s.push_recent_color("red");
        assert_eq!(s.recent_colors.len(), 2);
        for i in 0..10u8 {
            s.push_recent_color(&format!("#0000{i:02X}"));
        }
        assert_eq!(s.recent_colors.len(), MAX_RECENT_COLORS);
        assert_eq!(s.recent_colors[0], "#000009");
        assert_eq!(s.recent_colors[7], "#000002");
        // Old settings files without the field load with no recent colours.
        let old: AppSettings = serde_json::from_str(r#"{"theme": "X"}"#).unwrap();
        assert!(old.recent_colors.is_empty());
    }

    #[test]
    fn recent_fonts_dedupe_and_cap() {
        let mut s = AppSettings::default();
        assert!(s.recent_fonts.is_empty());
        s.push_recent_font("Georgia");
        s.push_recent_font("Menlo");
        // Picking a font again moves it to the front, once.
        s.push_recent_font("Georgia");
        assert_eq!(s.recent_fonts, ["Georgia", "Menlo"]);
        // Blank names are ignored; names are kept as given otherwise.
        s.push_recent_font("   ");
        assert_eq!(s.recent_fonts, ["Georgia", "Menlo"]);
        for name in ["A", "B", "C", "D", "E", "F"] {
            s.push_recent_font(name);
        }
        assert_eq!(s.recent_fonts.len(), MAX_RECENT_FONTS);
        assert_eq!(s.recent_fonts, ["F", "E", "D", "C", "B"]);
        // Old settings files without the field load with no recent fonts.
        let old: AppSettings = serde_json::from_str(r#"{"theme": "X"}"#).unwrap();
        assert!(old.recent_fonts.is_empty());
    }

    #[test]
    fn load_settings_missing_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        let s = load_settings(&dir.path().join("nope.json"));
        assert_eq!(s, AppSettings::default());
    }

    #[test]
    fn load_settings_corrupt_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(load_settings(&p), AppSettings::default());
    }

    #[test]
    fn settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("settings.json");
        let mut s = AppSettings {
            theme: "Other".into(),
            show_layers: false,
            dashboard_window: Some(WindowSize {
                width: 800.0,
                height: 600.0,
            }),
            ..AppSettings::default()
        };
        s.touch_recent(Path::new("/nonexistent/a"), 5);
        save_settings(&s, &p).unwrap();
        assert_eq!(load_settings(&p), s);
        assert!(!dir.path().join("sub").join("settings.json.tmp").exists());
    }

    #[test]
    fn default_theme_is_one_dark() {
        let s = AppSettings::default();
        assert_eq!(s.theme, "Mindweave One Dark");
        assert!(s.show_layers && s.show_properties);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        std::fs::write(&p, r#"{"show_layers": false}"#).unwrap();
        let s = load_settings(&p);
        assert!(!s.show_layers);
        assert_eq!(s.theme, "Mindweave One Dark");
    }
}

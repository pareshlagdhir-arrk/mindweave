//! Application settings and the recent-files list, persisted as JSON.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECENT: usize = 20;
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

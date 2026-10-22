//! Application settings and the recent-files list, persisted as JSON.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::color::normalize_hex_color;
use crate::line::LineProps;
use crate::shape::{Shape, Style};

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
    /// The left sidebar, holding the Layers and Icons panes; hiding it
    /// keeps which panes it holds.
    pub show_left_sidebar: bool,
    pub show_layers: bool,
    pub show_properties: bool,
    /// The Icons pane, below Layers.
    pub show_icons: bool,
    /// Colours picked in the colour palette, newest first, as uppercase
    /// `#RRGGBB`; at most [`MAX_RECENT_COLORS`].
    pub recent_colors: Vec<String>,
    /// Font families picked in the Text section's font list, newest
    /// first; at most [`MAX_RECENT_FONTS`].
    pub recent_fonts: Vec<String>,
    /// The style last set on a shape in Properties, given to new shapes.
    pub shape_style: Option<Style>,
    /// The style and line properties last set on a line, given to new
    /// lines (their ends are never attached).
    pub line_style: Option<Style>,
    pub line_props: Option<LineProps>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME.to_string(),
            recent: Vec::new(),
            dashboard_window: None,
            editor_window: None,
            show_left_sidebar: true,
            show_layers: true,
            show_properties: true,
            show_icons: false,
            recent_colors: Vec::new(),
            recent_fonts: Vec::new(),
            shape_style: None,
            line_style: None,
            line_props: None,
        }
    }
}

fn normalized(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl AppSettings {
    /// Whether the left sidebar is on screen: shown, and holding a pane.
    pub fn left_sidebar_shown(&self) -> bool {
        self.show_left_sidebar && (self.show_layers || self.show_icons)
    }

    /// Whether the Layers pane is on screen.
    pub fn layers_shown(&self) -> bool {
        self.show_left_sidebar && self.show_layers
    }

    /// Whether the Icons pane is on screen.
    pub fn icons_shown(&self) -> bool {
        self.show_left_sidebar && self.show_icons
    }

    /// Hides the left sidebar with both its panes, or shows it again;
    /// shown empty, it shows Layers.
    pub fn toggle_left_sidebar(&mut self) {
        if self.left_sidebar_shown() {
            self.show_left_sidebar = false;
        } else {
            self.show_left_sidebar = true;
            if !self.show_icons {
                self.show_layers = true;
            }
        }
    }

    /// Hides the Layers pane, or shows it with the left sidebar.
    pub fn toggle_layers(&mut self) {
        self.show_layers = !self.layers_shown();
        self.show_left_sidebar |= self.show_layers;
    }

    /// Hides the Icons pane, or shows it with the left sidebar.
    pub fn toggle_icons(&mut self) {
        self.show_icons = !self.icons_shown();
        self.show_left_sidebar |= self.show_icons;
    }

    /// Gives a new `shape` the style last set on its kind: lines take the
    /// remembered line style and properties, other library shapes the
    /// remembered shape style. Text boxes and images keep their own.
    pub fn apply_remembered_style(&self, shape: &mut Shape) {
        if shape.is_text_box() || shape.is_image() || shape.is_topic() {
            return;
        }
        if shape.is_line() {
            if let Some(style) = &self.line_style {
                shape.style = style.clone();
            }
            if let (Some(remembered), Some(props)) = (&self.line_props, shape.line.as_mut()) {
                *props = LineProps {
                    start: None,
                    end: None,
                    relationship: props.relationship,
                    vertical: props.vertical,
                    ..remembered.clone()
                };
            }
        } else if let Some(style) = &self.shape_style {
            shape.style = style.clone();
        }
        shape.normalize();
    }

    /// Remembers `shape`'s style for new shapes of its kind (see
    /// [`Self::apply_remembered_style`]). Returns whether anything changed.
    pub fn remember_style(&mut self, shape: &Shape) -> bool {
        if shape.is_text_box() || shape.is_image() || shape.is_topic() {
            return false;
        }
        let before = (
            self.shape_style.clone(),
            self.line_style.clone(),
            self.line_props.clone(),
        );
        if shape.is_line() {
            self.line_style = Some(shape.style.clone());
            self.line_props = shape.line.clone().map(|p| LineProps {
                start: None,
                end: None,
                relationship: false,
                vertical: false,
                ..p
            });
        } else {
            self.shape_style = Some(shape.style.clone());
        }
        before
            != (
                self.shape_style.clone(),
                self.line_style.clone(),
                self.line_props.clone(),
            )
    }

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

    #[test]
    fn new_shapes_take_the_style_last_set_on_their_kind() {
        let libs = crate::library::builtin_libraries();
        let new = |id: &str| {
            Shape::new(
                libs[0].shape(id).unwrap(),
                "basic-shapes",
                crate::geometry::Frame::default(),
            )
        };
        let mut settings = AppSettings::default();
        // Nothing remembered: the library's theme colours.
        let mut r = new("rectangle");
        settings.apply_remembered_style(&mut r);
        assert_eq!(r.style.fill.as_deref(), Some("auto"));
        assert_eq!(r.style.stroke.as_deref(), Some("auto"));

        let mut red = new("ellipse");
        red.style.fill = Some("#FF0000".into());
        red.style.stroke_width = 4.0;
        assert!(settings.remember_style(&red));
        assert!(!settings.remember_style(&red));
        let mut line = new("line");
        line.style.dash = crate::line::LineDash::Dashed;
        let props = line.line.as_mut().unwrap();
        props.end_head = crate::line::ArrowHead::Triangle;
        props.start = Some(uuid::Uuid::new_v4());
        settings.remember_style(&line);

        let mut star = new("star");
        settings.apply_remembered_style(&mut star);
        assert_eq!(star.style, red.style);
        let mut l = new("line");
        settings.apply_remembered_style(&mut l);
        assert_eq!(l.style.dash, crate::line::LineDash::Dashed);
        let p = l.line.unwrap();
        assert_eq!(p.end_head, crate::line::ArrowHead::Triangle);
        assert_eq!(p.start, None);
        // Text boxes keep their own style and are never remembered.
        let mut t = Shape::new_text_box((0.0, 0.0));
        assert!(!settings.remember_style(&t));
        settings.apply_remembered_style(&mut t);
        assert_eq!(t.style, Style::default());
        // Settings without the new fields still load.
        let old: AppSettings = serde_json::from_str(r#"{"theme":"x"}"#).unwrap();
        assert_eq!(old.shape_style, None);
    }

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
    fn left_sidebar_hides_and_shows_both_panes() {
        let mut s = AppSettings {
            show_icons: true,
            ..AppSettings::default()
        };
        assert!(s.left_sidebar_shown());
        s.toggle_left_sidebar();
        assert!(!s.left_sidebar_shown() && !s.layers_shown() && !s.icons_shown());
        // Shown again, it holds the same panes.
        s.toggle_left_sidebar();
        assert!(s.layers_shown() && s.icons_shown());
    }

    #[test]
    fn showing_an_empty_left_sidebar_shows_layers() {
        let mut s = AppSettings {
            show_layers: false,
            ..AppSettings::default()
        };
        assert!(!s.left_sidebar_shown());
        s.toggle_left_sidebar();
        assert!(s.layers_shown() && !s.icons_shown());
    }

    #[test]
    fn showing_a_pane_shows_the_left_sidebar() {
        let mut s = AppSettings::default();
        s.toggle_left_sidebar();
        // Layers is still on but hidden with the sidebar: one toggle shows it.
        s.toggle_layers();
        assert!(s.layers_shown());
        s.toggle_left_sidebar();
        s.toggle_icons();
        assert!(s.icons_shown() && s.layers_shown());
        s.toggle_layers();
        assert!(!s.layers_shown() && s.icons_shown());
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

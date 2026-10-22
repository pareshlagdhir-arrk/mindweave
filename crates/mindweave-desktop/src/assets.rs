//! The app's assets: gpui-kit's full Lucide set, plus the accent layers
//! painted under the ribbon's own icons to give them a Fluent-style
//! two-tone look.

use std::borrow::Cow;

use gpui_kit::assets::AllAssets;
use gpui_kit::{AssetSource, Result, SharedString};

/// Where accent layers are served: `accent/<icon>/<layer>.svg`, where the
/// layer is an index into [`ACCENTS`].
const ACCENT_DIR: &str = "accent/";

/// One filled layer under an icon: SVG elements in Lucide's 24-unit box and
/// the colour they are painted in.
pub struct Accent {
    pub body: &'static str,
    pub color: u32,
}

const BLUE: u32 = 0x3a96dd;
const AMBER: u32 = 0xf2b33d;
const GREEN: u32 = 0x4cb782;
const PURPLE: u32 = 0x9071e8;
const TEAL: u32 = 0x2bb3c0;
const ORANGE: u32 = 0xf08a3c;
const ROSE: u32 = 0xe5537a;

/// Accent layers drawn for the ribbon's own commands, by Lucide name.
const ACCENTS: &[(&str, &[Accent])] = &[
    (
        "mouse-pointer-2",
        &[Accent {
            body: r#"<path d="M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z"/>"#,
            color: BLUE,
        }],
    ),
    (
        "move",
        &[Accent {
            body: r#"<circle cx="12" cy="12" r="3.5"/>"#,
            color: AMBER,
        }],
    ),
    (
        "type",
        &[Accent {
            body: r#"<rect x="4" y="4" width="16" height="3" rx="1"/><rect x="9" y="18.5" width="6" height="3" rx="1"/>"#,
            color: PURPLE,
        }],
    ),
    (
        "image",
        &[
            Accent {
                body: r#"<path d="M6 21l9.086-9.086a2 2 0 0 1 2.828 0L21 15v4a2 2 0 0 1-2 2z"/>"#,
                color: GREEN,
            },
            Accent {
                body: r#"<circle cx="9" cy="9" r="2"/>"#,
                color: AMBER,
            },
        ],
    ),
    (
        "network",
        &[Accent {
            body: r#"<rect x="16" y="16" width="6" height="6" rx="1"/><rect x="2" y="16" width="6" height="6" rx="1"/><rect x="9" y="2" width="6" height="6" rx="1"/>"#,
            color: TEAL,
        }],
    ),
    (
        "corner-down-right",
        &[Accent {
            body: r#"<path d="M15 10l5 5-5 5z"/>"#,
            color: GREEN,
        }],
    ),
    (
        "list-plus",
        &[Accent {
            body: r#"<circle cx="18" cy="12" r="4.5"/>"#,
            color: GREEN,
        }],
    ),
    (
        "braces",
        &[Accent {
            body: r#"<rect x="9" y="7" width="6" height="10" rx="1.5"/>"#,
            color: ORANGE,
        }],
    ),
    (
        "square-dashed",
        &[Accent {
            body: r#"<rect x="6" y="6" width="12" height="12" rx="1"/>"#,
            color: PURPLE,
        }],
    ),
    (
        "spline",
        &[Accent {
            body: r#"<circle cx="19" cy="5" r="2.5"/><circle cx="5" cy="19" r="2.5"/>"#,
            color: ROSE,
        }],
    ),
    (
        "sticky-note",
        &[Accent {
            body: r#"<path d="M15 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V9h-5a1 1 0 0 1-1-1z"/>"#,
            color: AMBER,
        }],
    ),
    (
        "sigma",
        &[Accent {
            body: r#"<path d="M6.5 4.8H17v14.4H6.5l4.1-5.4a3 3 0 0 0 0-3.6z"/>"#,
            color: PURPLE,
        }],
    ),
];

/// The accent layers of Lucide icon `name`, if it has hand-drawn ones.
pub fn accents(name: &str) -> Option<&'static [Accent]> {
    ACCENTS
        .iter()
        .find(|(icon, _)| *icon == name)
        .map(|(_, layers)| *layers)
}

/// The asset path of layer `ix` of [`accents`]`(name)`.
pub fn accent_path(name: &str, ix: usize) -> SharedString {
    format!("{ACCENT_DIR}{name}/{ix}.svg").into()
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let Some(rest) = path.strip_prefix(ACCENT_DIR) else {
            return AllAssets.load(path);
        };
        let Some((name, layer)) = rest
            .strip_suffix(".svg")
            .and_then(|rest| rest.rsplit_once('/'))
        else {
            return Ok(None);
        };
        let accent = layer
            .parse::<usize>()
            .ok()
            .and_then(|ix| accents(name)?.get(ix));
        Ok(accent.map(|accent| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">{}</svg>"#,
                accent.body
            )
            .into_bytes()
            .into()
        }))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        AllAssets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(path: &str) -> Option<String> {
        Assets
            .load(path)
            .unwrap()
            .map(|bytes| String::from_utf8(bytes.into_owned()).unwrap())
    }

    #[test]
    fn accent_layers_are_served_as_svg() {
        let svg = load(&accent_path("image", 1)).unwrap();
        assert!(svg.starts_with("<svg") && svg.contains(r#"<circle cx="9""#));
        assert!(load(&accent_path("image", 2)).is_none());
        assert!(load(&accent_path("no-such-icon", 0)).is_none());
        assert!(load("accent/garbage").is_none());
    }

    #[test]
    fn other_paths_come_from_gpui_kit() {
        assert!(load("icons/square.svg").is_some());
        assert!(Assets
            .list("icons/")
            .unwrap()
            .iter()
            .any(|p| p == "icons/square.svg"));
    }

    #[test]
    fn every_accent_names_a_bundled_icon() {
        for (name, layers) in ACCENTS {
            assert!(load(&format!("icons/{name}.svg")).is_some(), "{name}");
            assert!(!layers.is_empty(), "{name}");
        }
    }
}

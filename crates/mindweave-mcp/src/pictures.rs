//! Pictures for documents: image files an agent names, and icons from
//! the icon sets imported in the app. Read only when their extension is a
//! picture's and they are at most 20 MB; measured so they keep their
//! proportions.

use std::io::Cursor;
use std::path::Path;

use mindweave_core::ops::Picture;
use mindweave_core::MAX_IMAGE_BYTES;
use mindweave_core::{load_icon_sets, search_icons, IconSet, ImageData, ImageFormat};

use crate::args::expand;

/// The picture in the file at `path` (absolute, or starting `~/`).
pub fn read_file(path: &str) -> Result<Picture, String> {
    let path = expand(path)?;
    read_path(&path)
}

/// The picture in the file at `path`.
fn read_path(path: &Path) -> Result<Picture, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if !ImageFormat::EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "{} is not a picture; use a file ending in {}.",
            path.display(),
            ImageFormat::EXTENSIONS.map(|e| format!(".{e}")).join(", ")
        ));
    }
    let len = std::fs::metadata(path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?
        .len();
    if len > MAX_IMAGE_BYTES as u64 {
        return Err(format!("{} is larger than 20 MB.", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let data = ImageData::from_file_bytes(bytes)
        .ok_or_else(|| format!("{} is not a picture Mindweave can open.", path.display()))?;
    let size = measure(&data)
        .ok_or_else(|| format!("{} could not be read as a picture.", path.display()))?;
    Ok(Picture { data, size })
}

/// The size of `data`: pixels, or an SVG's own size.
fn measure(data: &ImageData) -> Option<(f32, f32)> {
    if data.format == ImageFormat::Svg {
        let tree =
            resvg::usvg::Tree::from_data(&data.bytes, &resvg::usvg::Options::default()).ok()?;
        let s = tree.size();
        return Some((s.width(), s.height()));
    }
    let (w, h) = image::ImageReader::new(Cursor::new(data.bytes.as_slice()))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    (w > 0 && h > 0).then_some((w as f32, h as f32))
}

/// The icon sets in `dir`.
pub fn icon_sets(dir: &Path) -> Vec<IconSet> {
    load_icon_sets(dir)
}

/// The icon named `spec` (`"Set/icon name"`, or just the icon's name
/// when only one set has it), ignoring case.
pub fn read_icon(dir: &Path, spec: &str) -> Result<Picture, String> {
    let sets = icon_sets(dir);
    if sets.is_empty() {
        return Err(format!(
            "No icon sets are installed (in {}). Import one in the Mindweave app (Icons pane, +), or pass `file` with a picture file instead.",
            dir.display()
        ));
    }
    let (set, name) = match spec.split_once('/') {
        Some((set, name)) => (Some(set.trim()), name.trim()),
        None => (None, spec.trim()),
    };
    let matches: Vec<_> = sets
        .iter()
        .filter(|s| set.is_none_or(|n| s.name.eq_ignore_ascii_case(n)))
        .flat_map(|s| s.icons.iter().map(move |i| (s, i)))
        .filter(|(_, i)| i.name.eq_ignore_ascii_case(name))
        .collect();
    match matches.as_slice() {
        [(_, icon)] => read_path(&icon.path),
        [] => {
            let near: Vec<String> = search_icons(&sets, set, name)
                .into_iter()
                .take(5)
                .map(|(s, i)| format!("{}/{}", s.name, i.name))
                .collect();
            Err(if near.is_empty() {
                format!("No icon \"{spec}\"; find one with list_icons.")
            } else {
                format!("No icon \"{spec}\". Close matches: {}.", near.join(", "))
            })
        }
        many => Err(format!(
            "Several sets have an icon \"{name}\": {}. Write it as \"Set/{name}\".",
            many.iter()
                .map(|(s, _)| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

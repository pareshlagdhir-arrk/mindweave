//! Icon sets: folders of SVG or PNG icons the user imports (for example
//! a vendor's architecture icon pack), kept under [`user_icons_dir`], one
//! folder per set. An icon placed on the canvas becomes an image.
//!
//! Imports come from a folder or a `.zip` and are flattened into the set's
//! folder. When an import holds SVG files, only those are taken (packs
//! often ship the same icons as PNG too); otherwise its PNG files are.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Most icons taken from one import.
pub const MAX_ICONS_PER_SET: usize = 20_000;
/// Largest icon file taken, in bytes.
pub const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;
/// Most bytes taken from one import.
pub const MAX_SET_BYTES: u64 = 500 * 1024 * 1024;

/// Where imported icon sets are kept.
pub fn user_icons_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Mindweave")
        .join("icons")
}

/// One icon of a set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconEntry {
    /// Shown under the icon and searched: the file name without its
    /// extension, separators as spaces.
    pub name: String,
    /// The group the icon is listed under within its set (its category in
    /// the pack); empty for none.
    pub group: String,
    pub path: PathBuf,
    /// `name` in lowercase, for search.
    key: String,
}

/// An imported icon set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconSet {
    /// The folder name.
    pub name: String,
    pub dir: PathBuf,
    /// Sorted by name.
    pub icons: Vec<IconEntry>,
}

#[derive(Debug, Error)]
pub enum IconImportError {
    #[error("Could not read or write the icons: {0}")]
    Io(#[from] std::io::Error),
    #[error("This is not a .zip file Mindweave can read: {0}")]
    Zip(String),
    #[error("No SVG or PNG icons were found.")]
    NoIcons,
}

/// The kind of icon file `name` is, by extension.
fn icon_kind(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "svg" => Some("svg"),
        "png" => Some("png"),
        _ => None,
    }
}

/// Whether a path inside an import is skipped: hidden files and macOS
/// archive metadata.
fn skipped(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|part| part.starts_with('.') || part == "__MACOSX")
}

/// The name shown for icon file `stem`.
fn display_name(stem: &str) -> String {
    stem.replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sizes icon packs ship the same icon in, named by a number in the
/// file name (`Arch_Amazon-EC2_48.svg`).
const PACK_SIZES: [u32; 11] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512];

/// An icon's shown name without its size number, and that size (0 when
/// it names none): `Arch Amazon EC2 48` gives `("Arch Amazon EC2", 48)`.
fn split_size(name: &str) -> (String, u32) {
    let mut size = 0;
    let words: Vec<&str> = name
        .split(' ')
        .filter(|w| match w.parse::<u32>() {
            Ok(n) if size == 0 && PACK_SIZES.contains(&n) => {
                size = n;
                false
            }
            _ => true,
        })
        .collect();
    if words.is_empty() {
        return (name.to_string(), 0);
    }
    (words.join(" "), size)
}

/// `icons` with one icon per group and name once size numbers are
/// dropped: the largest, as packs ship each icon in several sizes.
/// Sorted by group, then name.
fn one_per_size(icons: Vec<IconEntry>) -> Vec<IconEntry> {
    let mut best: std::collections::BTreeMap<String, (u32, IconEntry)> =
        std::collections::BTreeMap::new();
    for icon in icons {
        let (name, size) = split_size(&icon.name);
        let key = name.to_lowercase();
        let slot = format!("{}\u{1f}{key}", icon.group.to_lowercase());
        let keep = best
            .get(&slot)
            .is_none_or(|(s, kept)| size > *s || (size == *s && icon.path < kept.path));
        if keep {
            best.insert(
                slot,
                (
                    size,
                    IconEntry {
                        name,
                        group: icon.group,
                        key,
                        path: icon.path,
                    },
                ),
            );
        }
    }
    best.into_values().map(|(_, icon)| icon).collect()
}

/// The icons directly in `dir`, listed under `group`.
fn icons_in(dir: &Path, group: &str) -> Vec<IconEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|f| {
            let file = f.file_name().to_str()?.to_string();
            icon_kind(&file)?;
            if file.starts_with('.') {
                return None;
            }
            let stem = Path::new(&file).file_stem()?.to_str()?.to_string();
            let name = display_name(&stem);
            Some(IconEntry {
                key: name.to_lowercase(),
                name,
                group: group.to_string(),
                path: f.path(),
            })
        })
        .collect()
}

/// Drops a first word several groups share (`Arch Analytics`, `Arch
/// Compute` → `Analytics`, `Compute`), so the category stands out. Groups
/// left with the same name are one group.
fn shorten_group_names(icons: &mut [IconEntry]) {
    let groups: HashSet<&str> = icons.iter().map(|i| i.group.as_str()).collect();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for group in &groups {
        if let Some((first, rest)) = group.split_once(' ') {
            if !rest.trim().is_empty() {
                *counts.entry(first).or_default() += 1;
            }
        }
    }
    let shared: HashSet<String> = counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(w, _)| w.to_string())
        .collect();
    for icon in icons.iter_mut() {
        if let Some((first, rest)) = icon.group.split_once(' ') {
            if shared.contains(first) && !rest.trim().is_empty() {
                icon.group = rest.trim().to_string();
            }
        }
    }
}

/// The group of icons whose names share no common first word.
const OTHER_GROUP: &str = "Other";

/// Fewest icons sharing a first word for that word to name a group.
const MIN_GROUP: usize = 3;

/// Groups icons that came without groups by the first word of their
/// names, when at least two words are each shared by [`MIN_GROUP`] icons
/// (`Arch …`, `Res …`); icons with a rarer first word go under "Other".
/// Otherwise they stay ungrouped.
fn group_by_first_word(icons: &mut [IconEntry]) {
    let first = |i: &IconEntry| i.name.split(' ').next().unwrap_or_default().to_string();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for icon in icons.iter() {
        *counts.entry(first(icon)).or_default() += 1;
    }
    if counts.values().filter(|&&n| n >= MIN_GROUP).count() < 2 {
        return;
    }
    for icon in icons.iter_mut() {
        let word = first(icon);
        icon.group = if counts[&word] >= MIN_GROUP {
            word
        } else {
            OTHER_GROUP.to_string()
        };
    }
}

/// The icon sets under `root`, sorted by name; folders without icons are
/// left out. Each icon shipped in several sizes is listed once.
pub fn load_icon_sets(root: &Path) -> Vec<IconSet> {
    let Ok(dirs) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut sets: Vec<IconSet> = dirs
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if name.starts_with('.') {
                return None;
            }
            // Icons in the set's folder, and in its group folders.
            let mut icons = icons_in(&e.path(), "");
            for sub in fs::read_dir(e.path()).ok()?.flatten() {
                let group = sub.file_name().to_str().unwrap_or_default().to_string();
                if sub.path().is_dir() && !group.starts_with('.') {
                    icons.extend(icons_in(&sub.path(), &group));
                }
            }
            if icons.is_empty() {
                return None;
            }
            shorten_group_names(&mut icons);
            let mut icons = one_per_size(icons);
            if icons.iter().all(|i| i.group.is_empty()) {
                group_by_first_word(&mut icons);
            }
            // By group ("Other" last), then name.
            icons.sort_by(|a, b| {
                let rank = |i: &IconEntry| (i.group == OTHER_GROUP, i.group.to_lowercase());
                rank(a).cmp(&rank(b)).then_with(|| a.key.cmp(&b.key))
            });
            Some(IconSet {
                name,
                dir: e.path(),
                icons,
            })
        })
        .collect();
    sets.sort_by_key(|s| s.name.to_lowercase());
    sets
}

/// The icon sets shown in the Icons pane: those of `sets` not named in
/// `hidden`. Hidden sets stay on disk, so icons placed from them still
/// draw.
pub fn shown_icon_sets(sets: &[IconSet], hidden: &[String]) -> Vec<IconSet> {
    sets.iter()
        .filter(|s| !hidden.contains(&s.name))
        .cloned()
        .collect()
}

/// The icons of `sets` (all, or only the set named `only`) whose names
/// hold every word of `query`, ignoring case; the set each comes from.
pub fn search_icons<'a>(
    sets: &'a [IconSet],
    only: Option<&str>,
    query: &str,
) -> Vec<(&'a IconSet, &'a IconEntry)> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    sets.iter()
        .filter(|s| only.is_none_or(|name| s.name == name))
        .flat_map(|s| s.icons.iter().map(move |i| (s, i)))
        .filter(|(_, i)| words.iter().all(|w| i.key.contains(w.as_str())))
        .collect()
}

/// An icon file found in an import: its file name and how to read it.
struct Found<R> {
    file_name: String,
    /// The group folder it goes in; empty for none.
    group: String,
    read: R,
}

/// Picks the files of an import (`(path inside it, size)`) to take: SVGs
/// when there are any, else PNGs; hidden files, metadata and files over
/// [`MAX_ICON_BYTES`] left out; at most [`MAX_ICONS_PER_SET`] and
/// [`MAX_SET_BYTES`] in all. Returns indexes into `files`.
fn pick(files: &[(String, u64)]) -> Vec<usize> {
    let wanted = |kind: &str| {
        files
            .iter()
            .enumerate()
            .filter(|(_, (p, size))| {
                !skipped(p) && icon_kind(p) == Some(kind) && *size <= MAX_ICON_BYTES
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let svgs = wanted("svg");
    let mut picked = if svgs.is_empty() { wanted("png") } else { svgs };
    picked.truncate(MAX_ICONS_PER_SET);
    let mut total = 0;
    picked.retain(|&i| {
        total += files[i].1;
        total <= MAX_SET_BYTES
    });
    picked
}

/// A folder name under `root` not yet taken: `name` (without leading
/// dots, which would hide it), else `name (2)`, ….
fn unique_dir(root: &Path, name: &str) -> PathBuf {
    let name = name.trim().trim_start_matches('.').trim();
    let name = if name.is_empty() { "Icons" } else { name };
    (1..)
        .map(|n| {
            if n == 1 {
                root.join(name)
            } else {
                root.join(format!("{name} ({n})"))
            }
        })
        .find(|p| !p.exists())
        .unwrap_or_else(|| root.join(name))
}

/// Writes `found` icons into a new set named after `name` under `root`,
/// through a temporary folder so a failed import leaves nothing behind.
fn write_set<R: Read>(
    root: &Path,
    name: &str,
    found: impl Iterator<Item = Found<R>>,
) -> Result<IconSet, IconImportError> {
    fs::create_dir_all(root)?;
    let tmp = root.join(format!(".import-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&tmp)?;
    let result = (|| {
        let mut used: HashSet<String> = HashSet::new();
        for Found {
            file_name,
            group,
            read,
        } in found
        {
            let path = Path::new(&file_name);
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("icon");
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("svg");
            let ext = ext.to_ascii_lowercase();
            // Same-named icons from different folders are kept apart.
            let mut out = format!("{stem}.{ext}");
            let mut n = 2;
            while !used.insert(format!("{group}/{out}").to_lowercase()) {
                out = format!("{stem}-{n}.{ext}");
                n += 1;
            }
            let mut bytes = Vec::new();
            read.take(MAX_ICON_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 <= MAX_ICON_BYTES {
                let dir = tmp.join(&group);
                fs::create_dir_all(&dir)?;
                fs::write(dir.join(out), bytes)?;
            }
        }
        if used.is_empty() {
            return Err(IconImportError::NoIcons);
        }
        let dir = unique_dir(root, name);
        fs::rename(&tmp, &dir)?;
        let set_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name)
            .to_string();
        load_icon_sets(root)
            .into_iter()
            .find(|s| s.name == set_name)
            .ok_or(IconImportError::NoIcons)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// Every file under `dir`, as (path relative to it, size).
fn walk(dir: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                out.push((rel.to_string_lossy().into_owned(), meta.len()));
            }
        }
    }
    out.sort();
    out
}

/// Imports the icons in folder `src` (and its subfolders) as a new set
/// named after the folder.
pub fn import_icon_folder(src: &Path, root: &Path) -> Result<IconSet, IconImportError> {
    let files = walk(src);
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Icons")
        .to_string();
    let picked = pick(&files);
    let groups = groups_of(&files, &picked);
    let found = picked.into_iter().zip(groups).map(|(i, group)| {
        let rel = &files[i].0;
        Found {
            file_name: file_name_of(rel),
            group,
            read: fs::File::open(src.join(rel)).map_or_else(
                |_| Box::new(std::io::empty()) as Box<dyn Read>,
                |f| Box::new(f) as Box<dyn Read>,
            ),
        }
    });
    write_set(root, &name, found)
}

/// Folder names that only sort a pack by format.
const FORMAT_FOLDERS: [&str; 4] = ["svg", "png", "icons", "icon"];

/// The group of each picked file of an import: the innermost folder
/// above it that names more than a size or a format, size numbers
/// dropped (`Arch_Analytics/48/x.svg` → "Arch Analytics"). A top folder
/// every picked file shares (the pack's own folder) names no group.
fn groups_of(files: &[(String, u64)], picked: &[usize]) -> Vec<String> {
    let folders = |path: &str| -> Vec<String> {
        let mut parts: Vec<String> = path.split(['/', '\\']).map(str::to_string).collect();
        parts.pop();
        parts
    };
    let all: Vec<Vec<String>> = picked.iter().map(|&i| folders(&files[i].0)).collect();
    let shared_top = all
        .first()
        .and_then(|f| f.first())
        .filter(|top| all.iter().all(|f| f.first() == Some(top)))
        .is_some();
    all.into_iter()
        .map(|parts| {
            let skip = usize::from(shared_top);
            parts
                .iter()
                .skip(skip)
                .rev()
                .map(|p| without_dates(&split_size(&display_name(p)).0))
                .find(|name| {
                    let lower = name.to_lowercase();
                    !name.is_empty()
                        && !FORMAT_FOLDERS.contains(&lower.as_str())
                        && !lower
                            .chars()
                            .all(|c| c.is_ascii_digit() || c == 'x' || c == ' ')
                        && !lower
                            .trim_end_matches("px")
                            .chars()
                            .all(|c| c.is_ascii_digit())
                })
                .unwrap_or_default()
        })
        .collect()
}

/// `name` without date stamps (runs of six or more digits), as in
/// `Architecture-Service-Icons_07312024`.
fn without_dates(name: &str) -> String {
    name.split(' ')
        .filter(|w| !(w.len() >= 6 && w.chars().all(|c| c.is_ascii_digit())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The last part of a path inside an import.
fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Imports the icons in `.zip` file `zip` as a new set named after it.
/// Paths inside the archive only name the icons; nothing is written
/// outside the set's folder.
pub fn import_icon_zip(zip: &Path, root: &Path) -> Result<IconSet, IconImportError> {
    let file = fs::File::open(zip)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| IconImportError::Zip(e.to_string()))?;
    let files: Vec<(String, u64)> = (0..archive.len())
        .map(|i| match archive.by_index(i) {
            Ok(f) if !f.is_dir() => (f.name().to_string(), f.size()),
            // Folders and unreadable entries are never picked.
            _ => (String::new(), u64::MAX),
        })
        .collect();
    let picked = pick(&files);
    let name = zip
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Icons")
        .to_string();
    // Entries are read one at a time, in the order picked.
    let groups = groups_of(&files, &picked);
    let mut contents = Vec::with_capacity(picked.len());
    for (i, group) in picked.into_iter().zip(groups) {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| IconImportError::Zip(e.to_string()))?;
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_ICON_BYTES + 1)
            .read_to_end(&mut bytes)?;
        contents.push(Found {
            file_name: file_name_of(&files[i].0),
            group,
            read: std::io::Cursor::new(bytes),
        });
    }
    write_set(root, &name, contents.into_iter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"/>"#;

    fn put(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn folder_imports_take_svgs_flattened_and_named() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let pack = src.path().join("AWS Icons");
        put(&pack, "Arch/Arch_Amazon-EC2_48.svg", SVG);
        put(&pack, "Res/Res_Amazon-S3_Bucket.svg", SVG);
        put(&pack, "Other/Arch_Amazon-EC2_48.svg", SVG);
        put(&pack, "PNG/Arch_Amazon-EC2_48.png", "png");
        put(&pack, ".hidden/secret.svg", SVG);
        put(&pack, "readme.txt", "x");
        let set = import_icon_folder(&pack, root.path()).unwrap();
        assert_eq!(set.name, "AWS Icons");
        let names: Vec<String> = set
            .icons
            .iter()
            .map(|i| format!("{}: {}", i.group, i.name))
            .collect();
        assert_eq!(
            names,
            [
                "Arch: Arch Amazon EC2",
                "Res: Res Amazon S3 Bucket",
                "Other: Arch Amazon EC2"
            ]
        );
        assert!(set
            .icons
            .iter()
            .all(|i| i.path.extension().unwrap() == "svg"));
        // No temporary folder is left, and a second import gets its own name.
        let again = import_icon_folder(&pack, root.path()).unwrap();
        assert_eq!(again.name, "AWS Icons (2)");
        assert_eq!(load_icon_sets(root.path()).len(), 2);
    }

    #[test]
    fn icons_shipped_in_several_sizes_are_listed_once() {
        let root = tempfile::tempdir().unwrap();
        let aws = root.path().join("AWS");
        for size in [16, 32, 48, 64] {
            put(&aws, &format!("Arch_Amazon-EC2_{size}.svg"), SVG);
        }
        put(&aws, "Res_Amazon-EC2_Instance_48_Light.svg", SVG);
        put(&aws, "Res_Amazon-EC2_Instance_48_Dark.svg", SVG);
        put(&aws, "Res_Amazon-EC2_Instance_32_Light.svg", SVG);
        put(&aws, "Route-53.svg", SVG);
        let sets = load_icon_sets(root.path());
        let icons = &sets[0].icons;
        let names: Vec<&str> = icons.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Arch Amazon EC2",
                "Res Amazon EC2 Instance Dark",
                "Res Amazon EC2 Instance Light",
                "Route 53"
            ]
        );
        // The largest size is kept.
        assert!(icons[0].path.ends_with("Arch_Amazon-EC2_64.svg"));
        assert!(icons[2]
            .path
            .ends_with("Res_Amazon-EC2_Instance_48_Light.svg"));
        assert_eq!(split_size("64"), ("64".to_string(), 0));
    }

    #[test]
    fn categories_come_from_folders_without_sizes_formats_or_the_pack_folder() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let pack = src.path().join("AWS");
        put(
            &pack,
            "Architecture-Icons/Arch_Analytics/48/Arch_Amazon-Athena_48.svg",
            SVG,
        );
        put(
            &pack,
            "Architecture-Icons/Arch_Analytics/64/Arch_Amazon-Athena_64.svg",
            SVG,
        );
        put(
            &pack,
            "Architecture-Icons/Arch-Category_64/Arch-Category_Compute_64.svg",
            SVG,
        );
        put(&pack, "Google/Compute Engine/SVG/Compute-Engine.svg", SVG);
        put(&pack, "Group-Icons_07312024/AWS-Cloud_32.svg", SVG);
        let set = import_icon_folder(&pack, root.path()).unwrap();
        let listed: Vec<String> = set
            .icons
            .iter()
            .map(|i| format!("{}: {}", i.group, i.name))
            .collect();
        assert_eq!(
            listed,
            [
                // "Arch" heads two groups, so it is dropped from both.
                "Analytics: Arch Amazon Athena",
                "Category: Arch Category Compute",
                "Compute Engine: Compute Engine",
                "Group Icons: AWS Cloud",
            ]
        );
        // A zip whose files all sit in one top folder: that folder is the
        // pack, not a group.
        let zip_path = src.path().join("Material.zip");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["Material/home.svg", "Material/action/search.svg"] {
                z.start_file(name, opts).unwrap();
                z.write_all(SVG.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let set = import_icon_zip(&zip_path, root.path()).unwrap();
        let groups: Vec<&str> = set.icons.iter().map(|i| i.group.as_str()).collect();
        assert_eq!(groups, ["", "action"]);
    }

    #[test]
    fn a_first_word_shared_by_groups_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let set = root.path().join("AWS");
        put(&set.join("Arch Analytics"), "Athena.svg", SVG);
        put(&set.join("Arch CloudFormation"), "Stack.svg", SVG);
        put(&set.join("Res Analytics"), "Query.svg", SVG);
        put(&set.join("General"), "Cloud.svg", SVG);
        let sets = load_icon_sets(root.path());
        let listed: Vec<String> = sets[0]
            .icons
            .iter()
            .map(|i| format!("{}: {}", i.group, i.name))
            .collect();
        // "Res" heads a single group, so it stays; the two Analytics
        // groups become one.
        assert_eq!(
            listed,
            [
                "Analytics: Athena",
                "CloudFormation: Stack",
                "General: Cloud",
                "Res Analytics: Query",
            ]
        );
    }

    #[test]
    fn sets_without_folders_are_grouped_by_a_shared_first_word() {
        let root = tempfile::tempdir().unwrap();
        let set = root.path().join("Old");
        for name in [
            "Arch_A", "Arch_B", "Arch_C", "Res_A", "Res_B", "Res_C", "Lone",
        ] {
            put(&set, &format!("{name}.svg"), SVG);
        }
        let sets = load_icon_sets(root.path());
        let groups: Vec<&str> = sets[0].icons.iter().map(|i| i.group.as_str()).collect();
        assert_eq!(
            groups,
            ["Arch", "Arch", "Arch", "Res", "Res", "Res", "Other"]
        );
        // Without a shared word, nothing is grouped.
        let plain = root.path().join("Plain");
        put(&plain, "home.svg", SVG);
        put(&plain, "search.svg", SVG);
        let sets = load_icon_sets(root.path());
        assert!(sets[1].icons.iter().all(|i| i.group.is_empty()));
    }

    #[test]
    fn png_only_packs_import_their_pngs() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        put(src.path(), "a.png", "png");
        put(src.path(), "b.PNG", "png");
        let set = import_icon_folder(src.path(), root.path()).unwrap();
        assert_eq!(set.icons.len(), 2);
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(
            import_icon_folder(empty.path(), root.path()),
            Err(IconImportError::NoIcons)
        ));
        // A failed import leaves no folder behind.
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn zip_imports_never_write_outside_the_set() {
        let src = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let zip_path = src.path().join("Material.zip");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for name in [
                "icons/home.svg",
                "../../escape.svg",
                "__MACOSX/icons/._home.svg",
            ] {
                z.start_file(name, opts).unwrap();
                z.write_all(SVG.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let set = import_icon_zip(&zip_path, root.path()).unwrap();
        assert_eq!(set.name, "Material");
        // Paths climbing out of the archive are skipped altogether.
        let names: Vec<&str> = set.icons.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["home"]);
        assert!(set.icons.iter().all(|i| i.path.starts_with(root.path())));
        assert!(!src.path().parent().unwrap().join("escape.svg").exists());
        let not_zip = src.path().join("x.zip");
        fs::write(&not_zip, "nope").unwrap();
        assert!(matches!(
            import_icon_zip(&not_zip, root.path()),
            Err(IconImportError::Zip(_))
        ));
    }

    #[test]
    fn hidden_sets_leave_the_shown_list() {
        let root = tempfile::tempdir().unwrap();
        put(&root.path().join("AWS"), "a.svg", SVG);
        put(&root.path().join("GCP"), "b.svg", SVG);
        let sets = load_icon_sets(root.path());
        let names = |sets: Vec<IconSet>| sets.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(shown_icon_sets(&sets, &[])), ["AWS", "GCP"]);
        assert_eq!(
            names(shown_icon_sets(&sets, &["AWS".into(), "gone".into()])),
            ["GCP"]
        );
    }

    #[test]
    fn search_matches_every_word_in_any_or_one_set() {
        let root = tempfile::tempdir().unwrap();
        put(&root.path().join("AWS"), "Amazon-EC2.svg", SVG);
        put(&root.path().join("AWS"), "Amazon-S3.svg", SVG);
        put(&root.path().join("GCP"), "Compute-Engine.svg", SVG);
        let sets = load_icon_sets(root.path());
        assert_eq!(sets.len(), 2);
        let names = |found: Vec<(&IconSet, &IconEntry)>| {
            found
                .iter()
                .map(|(_, i)| i.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(search_icons(&sets, None, "amazon s3")), ["Amazon S3"]);
        assert_eq!(names(search_icons(&sets, None, "")).len(), 3);
        assert_eq!(
            names(search_icons(&sets, Some("GCP"), "")),
            ["Compute Engine"]
        );
        assert!(search_icons(&sets, Some("GCP"), "amazon").is_empty());
    }
}

//! Managing shape libraries: the order and visibility the user gives
//! them, and installing and uninstalling user libraries.
//!
//! Libraries are installed from a folder or a `.zip` of one library
//! folder (holding the folder, or its files at the archive root). The
//! library is loaded with [`load_library`] before it is installed, so a
//! library that would be skipped at start-up is refused. Archives are
//! untrusted: a path climbing out of the archive refuses it, and files are
//! held to the library limits ([`MAX_FILE_BYTES`] each,
//! [`MAX_LIBRARY_BYTES`] in all, [`MAX_INSTALL_FILES`] files). Files are
//! staged in a hidden folder of the libraries folder and moved into place
//! only once the copy loads, so a failed install leaves nothing behind.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::library::{
    load_library, quote, DirFiles, Library, LibraryError, LibraryRegistry, MAX_FILE_BYTES,
    MAX_LIBRARY_BYTES, MAX_USER_LIBRARIES,
};

/// Most files taken from one library folder or archive.
pub const MAX_INSTALL_FILES: usize = 2_000;

/// The file every library folder holds.
const LIBRARY_TOML: &str = "library.toml";

/// `items` in the user's `order` (of ids): the listed ones first, in that
/// order, then the others in their own order. Listed ids that match no
/// item are ignored.
pub fn apply_order<T>(mut items: Vec<T>, id_of: impl Fn(&T) -> &str, order: &[String]) -> Vec<T> {
    // A stable sort keeps unlisted items in their own order.
    items.sort_by_key(|item| {
        order
            .iter()
            .position(|o| o == id_of(item))
            .unwrap_or(order.len())
    });
    items
}

/// Every library of `libraries`, in the user's `order`.
pub fn ordered_libraries<'a>(libraries: &'a [Library], order: &[String]) -> Vec<&'a Library> {
    apply_order(libraries.iter().collect(), |l| l.id.as_str(), order)
}

/// The libraries shown to the user (in the ribbon and the More gallery):
/// those not `disabled`, in the user's `order`. Hidden libraries stay in
/// the registry, so their shapes in documents still draw.
pub fn shown_libraries<'a>(
    libraries: &'a [Library],
    order: &[String],
    disabled: &[String],
) -> Vec<&'a Library> {
    ordered_libraries(libraries, order)
        .into_iter()
        .filter(|l| !disabled.contains(&l.id))
        .collect()
}

/// Why a library was not installed or uninstalled.
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Could not read or write the library: {0}")]
    Io(#[from] std::io::Error),
    #[error("This is not a .zip file Mindweave can read: {0}")]
    Zip(String),
    #[error(
        "No library was found: a library folder holds a library.toml (in a .zip, at its root \
         or in its one folder)."
    )]
    NotALibrary,
    #[error("This library cannot be installed: {0}.")]
    Invalid(LibraryError),
    #[error("A library with the id `{0}` is already installed.")]
    DuplicateId(String),
    #[error("The .zip holds a path outside its folder: `{0}`.")]
    UnsafePath(String),
    #[error(
        "The library is too large: at most 1 MB a file, 16 MB and {max} files in all.",
        max = MAX_INSTALL_FILES
    )]
    TooLarge,
    #[error("At most {max} libraries can be installed.", max = MAX_USER_LIBRARIES)]
    TooMany,
    #[error("Built-in libraries cannot be uninstalled; disable them instead.")]
    BuiltIn,
    #[error("No installed library has the id `{0}`.")]
    NotInstalled(String),
}

/// Loads the library in folder `root` and checks it can join `registry`:
/// it loads, its id is free, and there is room for another library.
pub fn validate_library(root: &Path, registry: &LibraryRegistry) -> Result<Library, InstallError> {
    let (library, _) =
        load_library(&DirFiles(root.to_path_buf()), false).map_err(InstallError::Invalid)?;
    if registry.library(&library.id).is_some() {
        return Err(InstallError::DuplicateId(quote(&library.id)));
    }
    let installed = registry.libraries().iter().filter(|l| !l.builtin).count();
    if installed >= MAX_USER_LIBRARIES {
        return Err(InstallError::TooMany);
    }
    Ok(library)
}

/// Installs the library in folder `src` into `user_dir`, in a folder named
/// after its id. Returns the library as installed.
pub fn install_library_folder(
    src: &Path,
    registry: &LibraryRegistry,
    user_dir: &Path,
) -> Result<Library, InstallError> {
    validate_library(src, registry)?;
    let staging = staging_dir(user_dir)?;
    let result =
        copy_tree(src, &staging).and_then(|()| finish_install(&staging, registry, user_dir));
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Installs the library in `.zip` file `zip` into `user_dir`, in a folder
/// named after its id. The archive holds one library folder, or the
/// files of one at its root. Returns the library as installed.
pub fn install_library_zip(
    zip: &Path,
    registry: &LibraryRegistry,
    user_dir: &Path,
) -> Result<Library, InstallError> {
    let staging = staging_dir(user_dir)?;
    let result = extract_zip(zip, &staging).and_then(|()| {
        let root = library_root(&staging).ok_or(InstallError::NotALibrary)?;
        finish_install(&root, registry, user_dir)
    });
    let _ = fs::remove_dir_all(&staging);
    result
}

/// The user library folder to move to the Trash to uninstall library `id`.
/// Built-in libraries and folders not directly inside `user_dir` are
/// refused.
pub fn uninstall_folder(
    registry: &LibraryRegistry,
    id: &str,
    user_dir: &Path,
) -> Result<PathBuf, InstallError> {
    let library = registry
        .library(id)
        .ok_or_else(|| InstallError::NotInstalled(quote(id)))?;
    if library.builtin {
        return Err(InstallError::BuiltIn);
    }
    let folder = registry
        .folder(id)
        .ok_or_else(|| InstallError::NotInstalled(quote(id)))?;
    let inside = folder.file_name().is_some()
        && folder
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|parent| user_dir.canonicalize().ok() == Some(parent));
    if !inside {
        return Err(InstallError::NotInstalled(quote(id)));
    }
    Ok(folder.to_path_buf())
}

/// A new hidden folder in `user_dir` to stage an install in; the
/// registry skips hidden folders.
fn staging_dir(user_dir: &Path) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(user_dir)?;
    let dir = user_dir.join(format!(".install-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir)?;
    Ok(dir)
}

/// Checks the staged library at `root` and moves it into `user_dir`.
fn finish_install(
    root: &Path,
    registry: &LibraryRegistry,
    user_dir: &Path,
) -> Result<Library, InstallError> {
    let library = validate_library(root, registry)?;
    let dest = free_folder(user_dir, &library.id);
    fs::rename(root, &dest)?;
    Ok(library)
}

/// A folder name under `dir` not yet taken: `name`, else `name-2`, ….
/// Library ids are safe folder names.
fn free_folder(dir: &Path, name: &str) -> PathBuf {
    (1..)
        .map(|n| match n {
            1 => dir.join(name),
            n => dir.join(format!("{name}-{n}")),
        })
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(name))
}

/// Whether a path inside a library folder or archive is left out: hidden
/// files and macOS archive metadata.
fn skipped(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|part| part.starts_with('.') || part == "__MACOSX")
}

/// Adds `bytes` to the running `total` and `files` count, refusing when
/// either goes past the install limits.
fn charge(total: &mut u64, files: &mut usize, bytes: u64) -> Result<(), InstallError> {
    *total += bytes;
    *files += 1;
    if bytes > MAX_FILE_BYTES as u64
        || *total > MAX_LIBRARY_BYTES as u64
        || *files > MAX_INSTALL_FILES
    {
        return Err(InstallError::TooLarge);
    }
    Ok(())
}

/// Copies the files under `src` into `dst`, leaving out hidden files and
/// symbolic links, within the install limits.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), InstallError> {
    let (mut total, mut files) = (0, 0);
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        fs::create_dir_all(dst.join(&rel))?;
        for entry in fs::read_dir(src.join(&rel))? {
            let entry = entry?;
            let name = entry.file_name();
            if skipped(&name.to_string_lossy()) {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path())?;
            let rel = rel.join(&name);
            if meta.is_dir() {
                stack.push(rel);
            } else if meta.is_file() {
                let mut bytes = Vec::new();
                fs::File::open(entry.path())?
                    .take(MAX_FILE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                charge(&mut total, &mut files, bytes.len() as u64)?;
                fs::write(dst.join(&rel), bytes)?;
            }
        }
    }
    Ok(())
}

/// Extracts `.zip` file `zip` into `dst`, leaving out hidden files and
/// macOS metadata, within the install limits. A path climbing out of the
/// archive refuses it.
fn extract_zip(zip: &Path, dst: &Path) -> Result<(), InstallError> {
    let zip_error = |e: zip::result::ZipError| InstallError::Zip(e.to_string());
    let mut archive = zip::ZipArchive::new(fs::File::open(zip)?).map_err(zip_error)?;
    if archive.len() > MAX_INSTALL_FILES {
        return Err(InstallError::TooLarge);
    }
    let (mut total, mut files) = (0, 0);
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_error)?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(InstallError::UnsafePath(quote(entry.name())));
        };
        if skipped(entry.name()) || entry.is_symlink() {
            continue;
        }
        let out = dst.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        // The size in the archive is not trusted: reading stops past it.
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        charge(&mut total, &mut files, bytes.len() as u64)?;
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(out, bytes)?;
    }
    Ok(())
}

/// The library folder in extracted archive `dir`: `dir` itself when it
/// holds a `library.toml`, else its one visible subfolder holding one.
fn library_root(dir: &Path) -> Option<PathBuf> {
    if dir.join(LIBRARY_TOML).is_file() {
        return Some(dir.to_path_buf());
    }
    let folders: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| !skipped(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    match folders.as_slice() {
        [only] if only.join(LIBRARY_TOML).is_file() => Some(only.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_libraries;
    use std::io::Write;

    const SHAPES: &str = "[[shape]]\nid = \"r\"\nname = \"R\"\nprimitive = \"rect\"\n";

    fn library_toml(id: &str) -> String {
        format!("format = 1\nid = \"{id}\"\nname = \"Lib {id}\"\nversion = \"1\"\n")
    }

    /// Writes a library folder `dir/name` with id `id` and returns it.
    fn write_lib(dir: &Path, name: &str, id: &str) -> PathBuf {
        let root = dir.join(name);
        fs::create_dir_all(root.join("shapes")).unwrap();
        fs::write(root.join("library.toml"), library_toml(id)).unwrap();
        fs::write(root.join("shapes.toml"), SHAPES).unwrap();
        fs::write(root.join("shapes/readme.txt"), "x").unwrap();
        root
    }

    /// Writes `.zip` file `path` holding `entries` as `(name, contents)`.
    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (name, text) in entries {
            z.start_file(*name, opts).unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    fn registry(user_dir: &Path) -> LibraryRegistry {
        LibraryRegistry::new(builtin_libraries(), Some(user_dir)).0
    }

    /// The visible folder names in `dir`, sorted.
    fn folders(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn order_puts_listed_ids_first_and_keeps_the_rest_in_place() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let items = ids(&["a", "b", "c", "d"]);
        let order = ids(&["c", "gone", "a"]);
        assert_eq!(
            apply_order(items.clone(), |s| s.as_str(), &order),
            ids(&["c", "a", "b", "d"])
        );
        assert_eq!(apply_order(items.clone(), |s| s.as_str(), &[]), items);
    }

    #[test]
    fn disabled_libraries_are_hidden_but_still_resolve() {
        let reg = LibraryRegistry::new(builtin_libraries(), None).0;
        let order = vec!["uml".to_string(), "flowchart".to_string()];
        let disabled = vec!["flowchart".to_string()];
        let all: Vec<&str> = ordered_libraries(reg.libraries(), &order)
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(all, ["uml", "flowchart", "basic-shapes", "wireframe"]);
        let shown: Vec<&str> = shown_libraries(reg.libraries(), &order, &disabled)
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(shown, ["uml", "basic-shapes", "wireframe"]);
        // The registry still draws a hidden library's shapes.
        let flowchart = reg.library("flowchart").unwrap();
        let first = &flowchart.shapes[0].id;
        assert!(reg.shape("flowchart", first).is_some());
    }

    #[test]
    fn a_valid_folder_installs_under_its_id() {
        let src = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let lib = write_lib(src.path(), "My Pack", "my-pack");
        fs::write(lib.join(".DS_Store"), "x").unwrap();
        let installed = install_library_folder(&lib, &registry(user.path()), user.path()).unwrap();
        assert_eq!(installed.id, "my-pack");
        assert_eq!(folders(user.path()), ["my-pack"]);
        let dest = user.path().join("my-pack");
        assert!(dest.join("shapes/readme.txt").is_file());
        assert!(!dest.join(".DS_Store").exists());
        let reg = registry(user.path());
        assert!(reg.library("my-pack").is_some());
        assert_eq!(reg.folder("my-pack"), Some(dest.as_path()));
        assert_eq!(reg.folder("basic-shapes"), None);
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let src = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        // A built-in id.
        let lib = write_lib(src.path(), "copy", "basic-shapes");
        assert!(matches!(
            install_library_folder(&lib, &registry(user.path()), user.path()),
            Err(InstallError::DuplicateId(_))
        ));
        // An installed id.
        let lib = write_lib(src.path(), "pack", "pack");
        install_library_folder(&lib, &registry(user.path()), user.path()).unwrap();
        let err = install_library_folder(&lib, &registry(user.path()), user.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "A library with the id `pack` is already installed."
        );
        assert_eq!(folders(user.path()), ["pack"]);
    }

    #[test]
    fn broken_libraries_are_refused_and_leave_nothing() {
        let src = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let lib = write_lib(src.path(), "broken", "broken");
        fs::remove_file(lib.join("shapes.toml")).unwrap();
        let err = install_library_folder(&lib, &registry(user.path()), user.path()).unwrap_err();
        assert!(matches!(err, InstallError::Invalid(_)));
        assert_eq!(
            err.to_string(),
            "This library cannot be installed: `shapes.toml` is missing."
        );
        assert!(folders(user.path()).is_empty());
        // A zip without a library.
        let zip = src.path().join("none.zip");
        write_zip(&zip, &[("a.txt", "x"), ("b/c.txt", "y")]);
        assert!(matches!(
            install_library_zip(&zip, &registry(user.path()), user.path()),
            Err(InstallError::NotALibrary)
        ));
        assert!(folders(user.path()).is_empty());
    }

    #[test]
    fn oversized_libraries_are_refused() {
        let src = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let lib = write_lib(src.path(), "big", "big");
        fs::write(lib.join("huge.bin"), vec![b'x'; MAX_FILE_BYTES + 1]).unwrap();
        assert!(matches!(
            install_library_folder(&lib, &registry(user.path()), user.path()),
            Err(InstallError::TooLarge)
        ));
        assert!(folders(user.path()).is_empty());
    }

    #[test]
    fn zips_install_with_a_folder_or_files_at_the_root() {
        let src = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let toml = library_toml("zipped");
        let in_folder = src.path().join("in-folder.zip");
        write_zip(
            &in_folder,
            &[
                ("Zipped/library.toml", &toml),
                ("Zipped/shapes.toml", SHAPES),
                ("__MACOSX/Zipped/._library.toml", "junk"),
            ],
        );
        let lib = install_library_zip(&in_folder, &registry(user.path()), user.path()).unwrap();
        assert_eq!(lib.id, "zipped");
        assert_eq!(folders(user.path()), ["zipped"]);

        let toml = library_toml("rooted");
        let at_root = src.path().join("at-root.zip");
        write_zip(
            &at_root,
            &[("library.toml", &toml), ("shapes.toml", SHAPES)],
        );
        install_library_zip(&at_root, &registry(user.path()), user.path()).unwrap();
        assert_eq!(folders(user.path()), ["rooted", "zipped"]);
        assert!(registry(user.path()).library("rooted").is_some());
    }

    #[test]
    fn zips_climbing_out_of_the_archive_are_refused() {
        let src = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let user = parent.path().join("libraries");
        let toml = library_toml("sneaky");
        let zip = src.path().join("sneaky.zip");
        write_zip(
            &zip,
            &[
                ("library.toml", &toml),
                ("shapes.toml", SHAPES),
                ("../../escape.txt", "x"),
            ],
        );
        let err = install_library_zip(&zip, &registry(&user), &user).unwrap_err();
        assert!(matches!(err, InstallError::UnsafePath(_)), "{err}");
        assert!(!parent.path().join("escape.txt").exists());
        assert!(folders(&user).is_empty());
        let not_zip = src.path().join("x.zip");
        fs::write(&not_zip, "nope").unwrap();
        assert!(matches!(
            install_library_zip(&not_zip, &registry(&user), &user),
            Err(InstallError::Zip(_))
        ));
    }

    #[test]
    fn only_user_libraries_in_the_libraries_folder_uninstall() {
        let user = tempfile::tempdir().unwrap();
        write_lib(user.path(), "mine", "mine");
        let reg = registry(user.path());
        assert_eq!(
            uninstall_folder(&reg, "mine", user.path()).unwrap(),
            user.path().join("mine")
        );
        assert!(matches!(
            uninstall_folder(&reg, "basic-shapes", user.path()),
            Err(InstallError::BuiltIn)
        ));
        assert!(matches!(
            uninstall_folder(&reg, "nope", user.path()),
            Err(InstallError::NotInstalled(_))
        ));
        // A registry loaded from elsewhere is not this folder's.
        let other = tempfile::tempdir().unwrap();
        assert!(matches!(
            uninstall_folder(&reg, "mine", other.path()),
            Err(InstallError::NotInstalled(_))
        ));
    }
}

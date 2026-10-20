//! File operations on documents: renaming a `.mindweave` file in place.

use std::path::{Path, PathBuf};

use thiserror::Error;

const EXTENSION: &str = "mindweave";
/// The longest file name stem accepted, in characters.
pub const MAX_STEM_LEN: usize = 200;
/// The longest stem in UTF-8 bytes: the 255-byte file name limit less
/// ".mindweave".
const MAX_STEM_BYTES: usize = 255 - EXTENSION.len() - 1;

#[derive(Debug, Error)]
pub enum RenameError {
    #[error("Name can't be empty.")]
    Empty,
    #[error("Name can't contain / or : or start with a dot.")]
    Invalid,
    #[error("Name can't be longer than {MAX_STEM_LEN} characters.")]
    TooLong,
    #[error("A file named \"{0}.{EXTENSION}\" already exists.")]
    Exists(String),
    #[error("Could not rename the file: {0}")]
    Io(#[from] std::io::Error),
}

/// Checks a new file name stem typed by the user and returns it trimmed.
pub fn validate_new_stem(stem: &str) -> Result<String, RenameError> {
    let stem = stem.trim();
    if stem.is_empty() {
        Err(RenameError::Empty)
    } else if stem.contains(['/', ':']) || stem.starts_with('.') {
        Err(RenameError::Invalid)
    } else if stem.chars().count() > MAX_STEM_LEN || stem.len() > MAX_STEM_BYTES {
        Err(RenameError::TooLong)
    } else {
        Ok(stem.to_string())
    }
}

/// Renames the document at `path` to `<new_stem>.mindweave` in the same
/// folder and returns the new path. The same name is a no-op; another file
/// already at the new path is never replaced, though a change of case only
/// is allowed on case-insensitive volumes.
pub fn rename_document(path: &Path, new_stem: &str) -> Result<PathBuf, RenameError> {
    let stem = validate_new_stem(new_stem)?;
    let new_path = path.with_file_name(format!("{stem}.{EXTENSION}"));
    if new_path == path {
        return Ok(new_path);
    }
    // A link at the new name counts as a file there, even a dangling one.
    if std::fs::symlink_metadata(&new_path).is_ok() {
        if !is_same_file(path, &new_path) {
            return Err(RenameError::Exists(stem));
        }
        // A change of case only: the "existing" file is this one.
        std::fs::rename(path, &new_path)?;
        return Ok(new_path);
    }
    // Exclusive, so a file created at the new name since the check above
    // is not replaced.
    match rename_exclusive(path, &new_path) {
        Ok(()) => Ok(new_path),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(RenameError::Exists(stem))
        }
        Err(err) => Err(err.into()),
    }
}

/// Renames `from` to `to`, failing with `AlreadyExists` when `to` exists.
#[cfg(target_os = "macos")]
fn rename_exclusive(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both arguments are valid NUL-terminated strings that outlive
    // the call.
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        // EEXIST maps to `ErrorKind::AlreadyExists`.
        Err(std::io::Error::last_os_error())
    }
}

/// Renames `from` to `to`, failing with `AlreadyExists` when `to` exists:
/// a hard link (which never replaces) then removal of the old name.
#[cfg(not(target_os = "macos"))]
fn rename_exclusive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

/// Whether `a` and `b` name the same file, as `plan` and `Plan` do on a
/// case-insensitive volume.
#[cfg(unix)]
fn is_same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, name).unwrap();
        p
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn validate_trims_and_accepts_plain_names() {
        assert_eq!(validate_new_stem("  Q4 plan ").unwrap(), "Q4 plan");
        assert_eq!(validate_new_stem("a.b").unwrap(), "a.b");
        assert_eq!(
            validate_new_stem(&"x".repeat(MAX_STEM_LEN)).unwrap().len(),
            MAX_STEM_LEN
        );
    }

    #[test]
    fn validate_rejects_bad_names() {
        assert!(matches!(validate_new_stem(""), Err(RenameError::Empty)));
        assert!(matches!(validate_new_stem("   "), Err(RenameError::Empty)));
        assert!(matches!(
            validate_new_stem("a/b"),
            Err(RenameError::Invalid)
        ));
        assert!(matches!(
            validate_new_stem("a:b"),
            Err(RenameError::Invalid)
        ));
        assert!(matches!(
            validate_new_stem(".hidden"),
            Err(RenameError::Invalid)
        ));
        assert!(matches!(
            validate_new_stem("  .hidden"),
            Err(RenameError::Invalid)
        ));
        assert!(matches!(
            validate_new_stem(&"x".repeat(MAX_STEM_LEN + 1)),
            Err(RenameError::TooLong)
        ));
        // Within 200 characters, a name must also fit in 245 UTF-8 bytes
        // (the 255-byte file name limit less ".mindweave").
        assert!(validate_new_stem(&"é".repeat(122)).is_ok());
        assert!(matches!(
            validate_new_stem(&"é".repeat(123)),
            Err(RenameError::TooLong)
        ));
    }

    #[test]
    fn errors_read_as_messages() {
        assert_eq!(RenameError::Empty.to_string(), "Name can't be empty.");
        assert_eq!(
            RenameError::Invalid.to_string(),
            "Name can't contain / or : or start with a dot."
        );
        assert_eq!(
            RenameError::Exists("Plan".into()).to_string(),
            "A file named \"Plan.mindweave\" already exists."
        );
    }

    #[test]
    fn rename_moves_the_file_within_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let old = write(dir.path(), "Old.mindweave");
        let new = rename_document(&old, "  New name ").unwrap();
        assert_eq!(new, dir.path().join("New name.mindweave"));
        assert_eq!(names(dir.path()), ["New name.mindweave"]);
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "Old.mindweave");
    }

    #[test]
    fn rename_to_the_same_name_does_nothing() {
        // The file does not exist, so any file system call would fail.
        let path = Path::new("/nonexistent/Plan.mindweave");
        assert_eq!(rename_document(path, " Plan ").unwrap(), path);
    }

    #[test]
    fn rename_refuses_to_replace_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "A.mindweave");
        write(dir.path(), "B.mindweave");
        let err = rename_document(&a, "B").unwrap_err();
        assert!(matches!(&err, RenameError::Exists(name) if name == "B"));
        assert_eq!(names(dir.path()), ["A.mindweave", "B.mindweave"]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("B.mindweave")).unwrap(),
            "B.mindweave"
        );
    }

    #[test]
    fn exclusive_rename_never_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "A.mindweave");
        let b = write(dir.path(), "B.mindweave");
        let err = rename_exclusive(&a, &b).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "A.mindweave");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "B.mindweave");
    }

    #[test]
    fn exclusive_rename_moves_to_a_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "A.mindweave");
        rename_exclusive(&a, &dir.path().join("C.mindweave")).unwrap();
        assert_eq!(names(dir.path()), ["C.mindweave"]);
    }

    #[cfg(unix)]
    #[test]
    fn rename_treats_a_symlink_at_the_new_name_as_existing() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "A.mindweave");
        // A dangling link, and one pointing at the file being renamed.
        std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("B.mindweave"))
            .unwrap();
        std::os::unix::fs::symlink(&a, dir.path().join("C.mindweave")).unwrap();
        assert!(matches!(
            rename_document(&a, "B"),
            Err(RenameError::Exists(_))
        ));
        assert!(matches!(
            rename_document(&a, "C"),
            Err(RenameError::Exists(_))
        ));
        assert!(a.exists());
    }

    #[test]
    fn rename_allows_a_case_only_change() {
        let dir = tempfile::tempdir().unwrap();
        let old = write(dir.path(), "plan.mindweave");
        let new = rename_document(&old, "Plan").unwrap();
        assert_eq!(new, dir.path().join("Plan.mindweave"));
        assert_eq!(names(dir.path()), ["Plan.mindweave"]);
    }

    #[test]
    fn rename_validates_before_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let old = write(dir.path(), "A.mindweave");
        assert!(matches!(
            rename_document(&old, "a/b"),
            Err(RenameError::Invalid)
        ));
        assert_eq!(names(dir.path()), ["A.mindweave"]);
    }

    #[test]
    fn rename_of_a_missing_file_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("Gone.mindweave");
        assert!(matches!(
            rename_document(&missing, "Other"),
            Err(RenameError::Io(_))
        ));
    }
}

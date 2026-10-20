//! File operations on documents: renaming a `.mindweave` file in place.

use std::path::{Path, PathBuf};

use thiserror::Error;

const EXTENSION: &str = "mindweave";
/// The longest file name stem accepted, in characters.
pub const MAX_STEM_LEN: usize = 200;

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
    } else if stem.chars().count() > MAX_STEM_LEN {
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
    if new_path.exists() && !is_same_file(path, &new_path) {
        return Err(RenameError::Exists(stem));
    }
    std::fs::rename(path, &new_path)?;
    Ok(new_path)
}

/// Whether `a` and `b` name the same file, as `plan` and `Plan` do on a
/// case-insensitive volume.
#[cfg(unix)]
fn is_same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
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
        // Characters, not bytes, count towards the limit.
        assert!(validate_new_stem(&"é".repeat(MAX_STEM_LEN)).is_ok());
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

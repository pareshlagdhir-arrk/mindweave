//! Reading tool arguments, with messages an agent can act on.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use uuid::Uuid;

/// The file extension of documents.
pub const DOC_EXTENSION: &str = "mindweave";
/// The most ids one argument may list.
pub const MAX_IDS: usize = 500;

/// A tool call's arguments.
pub struct Args<'a>(pub &'a Map<String, Value>);

impl Args<'_> {
    /// Argument `key`, if given and not null.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    /// String argument `key`.
    pub fn str_opt(&self, key: &str) -> Result<Option<&str>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(format!("`{key}` must be a string.")),
        }
    }

    /// Required string argument `key`.
    pub fn str_req(&self, key: &str) -> Result<&str, String> {
        self.str_opt(key)?
            .ok_or_else(|| format!("`{key}` is required."))
    }

    /// Number argument `key`.
    pub fn f32_opt(&self, key: &str) -> Result<Option<f32>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|f| f.is_finite())
                .map(|f| Some(f as f32))
                .ok_or_else(|| format!("`{key}` must be a number.")),
        }
    }

    /// Required number argument `key`.
    pub fn f32_req(&self, key: &str) -> Result<f32, String> {
        self.f32_opt(key)?
            .ok_or_else(|| format!("`{key}` is required."))
    }

    /// Whole-number argument `key` (a page or layer index).
    pub fn usize_opt(&self, key: &str) -> Result<Option<usize>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| format!("`{key}` must be a whole number from 0.")),
        }
    }

    /// Yes/no argument `key`.
    pub fn bool_opt(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("`{key}` must be true or false.")),
        }
    }

    /// Object id argument `key`.
    pub fn id_opt(&self, key: &str) -> Result<Option<Uuid>, String> {
        self.str_opt(key)?.map(|s| parse_id(key, s)).transpose()
    }

    /// Required object id argument `key`.
    pub fn id_req(&self, key: &str) -> Result<Uuid, String> {
        self.id_opt(key)?
            .ok_or_else(|| format!("`{key}` is required."))
    }

    /// Required list of object ids `key`.
    pub fn ids_req(&self, key: &str) -> Result<Vec<Uuid>, String> {
        let Some(Value::Array(items)) = self.get(key) else {
            return Err(format!("`{key}` must be a list of object ids."));
        };
        if items.is_empty() {
            return Err(format!("`{key}`: pass at least one id."));
        }
        if items.len() > MAX_IDS {
            return Err(format!("`{key}` may list at most {MAX_IDS} ids."));
        }
        items
            .iter()
            .map(|v| match v {
                Value::String(s) => parse_id(key, s),
                _ => Err(format!("`{key}` must be a list of object id strings.")),
            })
            .collect()
    }

    /// Required list of strings `key`.
    pub fn strings_req(&self, key: &str) -> Result<Vec<String>, String> {
        self.parse_req(key, "a list of strings")
    }

    /// Argument `key` parsed as `T`; `what` describes it for errors.
    pub fn parse_opt<T: DeserializeOwned>(
        &self,
        key: &str,
        what: &str,
    ) -> Result<Option<T>, String> {
        self.get(key)
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("`{key}` must be {what}: {e}."))
            })
            .transpose()
    }

    /// Required argument `key` parsed as `T`.
    pub fn parse_req<T: DeserializeOwned>(&self, key: &str, what: &str) -> Result<T, String> {
        self.parse_opt(key, what)?
            .ok_or_else(|| format!("`{key}` is required."))
    }

    /// The arguments `keys` that are given, as an object (to parse into
    /// an input type).
    pub fn subset(&self, keys: &[&str]) -> Value {
        Value::Object(
            keys.iter()
                .filter_map(|k| self.get(k).map(|v| (k.to_string(), v.clone())))
                .collect(),
        )
    }

    /// The document path argument `path`: absolute (or starting with
    /// `~/`), ending in `.mindweave`.
    pub fn doc_path(&self) -> Result<PathBuf, String> {
        let path = expand(self.str_req("path")?)?;
        if path.extension().and_then(|e| e.to_str()) != Some(DOC_EXTENSION) {
            return Err(format!(
                "`path` must end in .{DOC_EXTENSION}; the server reads and writes only Mindweave documents."
            ));
        }
        Ok(path)
    }
}

/// `s` as an object id.
fn parse_id(key: &str, s: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s.trim()).map_err(|_| {
        format!("`{key}`: `{s}` is not an object id; ids look like 6f9619ff-8b86-d011-b42d-00cf4fc964ff (see list_objects).")
    })
}

/// `p` with a leading `~/` made the home folder; it must then be
/// absolute.
pub fn expand(p: &str) -> Result<PathBuf, String> {
    let p = p.trim();
    let path = match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .ok_or("The home folder is unknown; pass an absolute path.")?
            .join(rest),
        None => PathBuf::from(p),
    };
    if !path.is_absolute() {
        return Err(format!(
            "`{p}` is not an absolute path; pass the full path (for example /Users/me/notes/plan.mindweave)."
        ));
    }
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err("Paths may not contain `..`.".into());
    }
    Ok(path)
}

/// Writes `bytes` to `path` through a temporary file renamed over it,
/// so a reader never sees half a file; an existing file is replaced.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    let fail = |e: std::io::Error| format!("Could not write {}: {e}", path.display());
    std::fs::write(&tmp, bytes).map_err(fail)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

/// Whether `path`'s folder exists, with a message if not.
pub fn check_folder(path: &Path) -> Result<(), String> {
    match path.parent() {
        Some(dir) if dir.is_dir() => Ok(()),
        _ => Err(format!(
            "The folder of {} does not exist; create it first.",
            path.display()
        )),
    }
}

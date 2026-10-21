//! Shape libraries: folders of data files (`library.toml`, `shapes.toml`
//! and optional SVG outlines) that describe the shapes users can place.
//! The format is documented for contributors in `docs/libraries.md`.
//!
//! Library files are untrusted. Every file is limited to
//! [`MAX_FILE_BYTES`], a library to [`MAX_SHAPES`] shapes and
//! [`MAX_LIBRARY_BYTES`] read in all, the user folder to
//! [`MAX_USER_LIBRARIES`] libraries, paths must stay inside the library
//! folder, and SVG files are read for geometry only (each once). Values
//! echoed in messages are shortened.
//! Problems never panic: they skip the library ([`LibraryError`]), skip
//! one shape or field (a warning), or fall back to a default (a warning).
//! Everything is parsed once at load; drawing never reads files.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use toml::{Table, Value};

use crate::color::normalize_hex_color;
use crate::geometry::{primitive_geometry, scale_unit, Geometry, Primitive};
use crate::svg::parse_outline;

/// The library format this build reads (`format` in `library.toml`).
pub const LIBRARY_FORMAT: u32 = 1;

/// Largest library file accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 1_048_576;

/// Most shapes one library may declare.
pub const MAX_SHAPES: usize = 500;

/// Most bytes read from the files of one library, in all.
pub const MAX_LIBRARY_BYTES: usize = 16 * MAX_FILE_BYTES;

/// Most user library folders loaded.
pub const MAX_USER_LIBRARIES: usize = 100;

/// Longest value from a library file echoed in a message, in characters.
const MAX_QUOTED_CHARS: usize = 60;

/// Longest notice or log line, in characters (before the `…`).
const MAX_NOTICE_CHARS: usize = 600;

/// Longest library or shape id, in bytes.
const MAX_ID_LEN: usize = 64;

/// Longest library or shape name, in characters.
const MAX_NAME_LEN: usize = 100;

/// Largest default width or height, in canvas units.
const MAX_DEFAULT_SIZE: f32 = 10_000.0;

/// Widest default stroke, in canvas units.
const MAX_STROKE_WIDTH: f32 = 100.0;

/// Warnings listed in one registry notice before the rest are counted.
const MAX_NOTICE_WARNINGS: usize = 5;

const DEFAULT_ICON: &str = "shapes";
const DEFAULT_SIZE: (f32, f32) = (120.0, 80.0);
const DEFAULT_FILL: &str = "#F5F5F5";
const DEFAULT_STROKE: &str = "#7F8EA3";
const DEFAULT_STROKE_WIDTH: f32 = 1.5;
const DEFAULT_OPACITY: f32 = 1.0;

const LIBRARY_FIELDS: &[&str] = &[
    "format",
    "id",
    "name",
    "version",
    "author",
    "description",
    "icon",
    "order",
];
const SHAPE_FIELDS: &[&str] = &[
    "id",
    "name",
    "icon",
    "default_size",
    "primitive",
    "svg",
    "style",
    "param",
];
const STYLE_FIELDS: &[&str] = &["fill", "stroke", "stroke_width", "opacity"];
const PARAM_FIELDS: &[&str] = &["key", "label", "min", "max", "step", "default"];

/// A toolbar icon: a Lucide icon name, or an outline from the library.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    Lucide(String),
    /// Normalised to the unit box, like shape outlines.
    Outline(Geometry),
}

/// A shape property users can change in the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    /// The primitive parameter it sets (see [`Primitive::param_keys`]).
    pub key: String,
    pub label: String,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// Within `min..=max`.
    pub default: f32,
}

/// The style a newly placed shape starts with.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleDefaults {
    /// Uppercase `#RRGGBB`, or `None` for no fill.
    pub fill: Option<String>,
    /// Uppercase `#RRGGBB`, or `None` for no stroke.
    pub stroke: Option<String>,
    pub stroke_width: f32,
    /// From 0 to 1.
    pub opacity: f32,
}

impl Default for StyleDefaults {
    fn default() -> Self {
        StyleDefaults {
            fill: Some(DEFAULT_FILL.into()),
            stroke: Some(DEFAULT_STROKE.into()),
            stroke_width: DEFAULT_STROKE_WIDTH,
            opacity: DEFAULT_OPACITY,
        }
    }
}

/// Where a shape's outline comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeSource {
    Primitive(Primitive),
    /// Normalised to the unit box `0..1 × 0..1`.
    Outline(Geometry),
}

/// One shape of a library.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeDef {
    pub id: String,
    pub name: String,
    pub icon: Icon,
    /// Width and height of a shape placed with a click.
    pub default_size: (f32, f32),
    pub style: StyleDefaults,
    pub source: ShapeSource,
    /// Declared parameters; always empty for outline shapes.
    pub params: Vec<ParamDef>,
}

impl ShapeDef {
    /// The outline in the local box `0..w × 0..h`. `value` gives the
    /// shape's parameter values; a declared parameter without a usable
    /// value takes its default, and values are clamped to the declared
    /// range. Undeclared parameters use the primitive's own defaults.
    pub fn local_geometry(&self, w: f32, h: f32, value: &dyn Fn(&str) -> Option<f32>) -> Geometry {
        match &self.source {
            ShapeSource::Primitive(kind) => {
                let param = |key: &str| match self.params.iter().find(|p| p.key == key) {
                    Some(def) => Some(
                        value(key)
                            .filter(|v| v.is_finite())
                            .map_or(def.default, |v| v.clamp(def.min, def.max)),
                    ),
                    None => value(key),
                };
                primitive_geometry(*kind, w, h, &param)
            }
            ShapeSource::Outline(g) => scale_unit(g, w, h),
        }
    }
}

/// A loaded library.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub icon: Icon,
    /// Sort key among user libraries; lower first.
    pub order: i32,
    /// Compiled into the app rather than read from the user folder.
    pub builtin: bool,
    /// In file order; ids are unique.
    pub shapes: Vec<ShapeDef>,
}

impl Library {
    /// The shape with this id, if any.
    pub fn shape(&self, id: &str) -> Option<&ShapeDef> {
        self.shapes.iter().find(|s| s.id == id)
    }
}

/// Why a whole library was skipped. The messages complete a sentence that
/// starts with the library folder name.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LibraryError {
    #[error("`{0}` is missing")]
    Missing(String),
    #[error("`{0}` is larger than 1 MB")]
    TooLarge(String),
    #[error("invalid TOML in {0}")]
    Toml(String),
    #[error("needs a newer version of Mindweave (library format {0})")]
    NewerFormat(u32),
    #[error("has {0} shapes; a library may have at most {max}", max = MAX_SHAPES)]
    TooManyShapes(usize),
    #[error("its files add up to more than 16 MB")]
    TooMuchData,
    #[error("{0}")]
    Invalid(String),
}

/// Pays for bytes about to be read; an error stops the read.
pub type Charge<'a> = &'a mut dyn FnMut(usize) -> Result<(), LibraryError>;

/// Read access to the files of one library, by path relative to its folder.
pub trait LibraryFiles {
    /// Reads `rel`. Before reading any bytes, passes their number to
    /// `charge` and returns its error, if any, so reads that then fail
    /// (such as text that is not UTF-8) are paid for too.
    fn read(&self, rel: &str, charge: Charge) -> Result<String, LibraryError>;
}

/// A library folder on disk. Paths that are absolute, contain `..`, or
/// resolve (through symbolic links) outside the folder are rejected, and
/// files larger than [`MAX_FILE_BYTES`] are not read.
pub struct DirFiles(pub PathBuf);

impl LibraryFiles for DirFiles {
    fn read(&self, rel: &str, charge: Charge) -> Result<String, LibraryError> {
        check_relative_path(rel)?;
        let shown = quote(rel);
        let unreadable = |e: std::io::Error| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LibraryError::Missing(shown.clone())
            } else {
                LibraryError::Invalid(format!("`{shown}` could not be read: {e}"))
            }
        };
        let root = self.0.canonicalize().map_err(|e| {
            LibraryError::Invalid(format!("the library folder could not be read: {e}"))
        })?;
        let path = root.join(rel).canonicalize().map_err(unreadable)?;
        if !path.starts_with(&root) {
            return Err(outside_folder(rel));
        }
        let meta = std::fs::metadata(&path).map_err(unreadable)?;
        if !meta.is_file() {
            return Err(LibraryError::Invalid(format!("`{shown}` is not a file")));
        }
        if meta.len() > MAX_FILE_BYTES as u64 {
            return Err(LibraryError::TooLarge(shown));
        }
        // At most MAX_FILE_BYTES, checked above.
        let size = meta.len() as usize;
        charge(size)?;
        // The file may grow between the size check and the read; bytes
        // beyond `size` are paid for after the read.
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|f| f.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut bytes))
            .map_err(unreadable)?;
        charge(bytes.len().saturating_sub(size))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(LibraryError::TooLarge(shown));
        }
        String::from_utf8(bytes)
            .map_err(|_| LibraryError::Invalid(format!("`{shown}` is not UTF-8 text")))
    }
}

/// Library files compiled into the app, as `(relative path, contents)`.
pub struct EmbeddedFiles(pub &'static [(&'static str, &'static str)]);

impl LibraryFiles for EmbeddedFiles {
    fn read(&self, rel: &str, charge: Charge) -> Result<String, LibraryError> {
        check_relative_path(rel)?;
        let (_, text) = self
            .0
            .iter()
            .find(|(path, _)| *path == rel)
            .ok_or_else(|| LibraryError::Missing(quote(rel)))?;
        charge(text.len())?;
        Ok((*text).to_owned())
    }
}

fn outside_folder(rel: &str) -> LibraryError {
    LibraryError::Invalid(format!(
        "`{}` is not a path inside the library folder",
        quote(rel)
    ))
}

/// `text`, cut to `max` characters with `…` added when it is longer.
fn shortened(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// A value from a library file, shortened for a message: at most 60
/// characters, then `…`.
pub fn quote(value: &str) -> String {
    shortened(value, MAX_QUOTED_CHARS)
}

/// A notice or log line about libraries, shortened to at most 600
/// characters, then `…`.
pub fn shorten_notice(text: &str) -> String {
    shortened(text, MAX_NOTICE_CHARS)
}

/// Messages from loading libraries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadReport {
    /// For the user: skipped libraries, shapes and parameters, and values
    /// replaced by defaults.
    pub notices: Vec<String>,
    /// For the log only: unknown fields, which a library written for a
    /// later Mindweave may add.
    pub log: Vec<String>,
}

/// The files of the library being loaded. Counts the bytes read against
/// [`MAX_LIBRARY_BYTES`], including those of reads that fail, and reads
/// each SVG outline once, however many shapes and icons use it and
/// however its path is written (`a.svg`, `./a.svg`).
struct Reader<'a> {
    files: &'a dyn LibraryFiles,
    bytes_read: usize,
    outlines: HashMap<String, Result<Geometry, String>>,
}

impl<'a> Reader<'a> {
    fn new(files: &'a dyn LibraryFiles) -> Self {
        Reader {
            files,
            bytes_read: 0,
            outlines: HashMap::new(),
        }
    }

    fn over_budget(&self) -> bool {
        self.bytes_read > MAX_LIBRARY_BYTES
    }

    /// Reads `rel`, enforcing [`MAX_FILE_BYTES`] whatever the source.
    /// Bytes are paid for before they are read, and nothing more is read
    /// once the budget is spent.
    fn read(&mut self, rel: &str) -> Result<String, LibraryError> {
        if self.over_budget() {
            return Err(LibraryError::TooMuchData);
        }
        let rel = normalized_path(rel)?;
        let before = self.bytes_read;
        let bytes_read = &mut self.bytes_read;
        let mut charge = |n: usize| {
            *bytes_read = bytes_read.saturating_add(n);
            if *bytes_read > MAX_LIBRARY_BYTES {
                Err(LibraryError::TooMuchData)
            } else {
                Ok(())
            }
        };
        let text = self.files.read(&rel, &mut charge)?;
        // In case a source did not pay for all it returned.
        let paid = self.bytes_read - before;
        self.bytes_read = self
            .bytes_read
            .saturating_add(text.len().saturating_sub(paid));
        if text.len() > MAX_FILE_BYTES {
            return Err(LibraryError::TooLarge(quote(&rel)));
        }
        if self.over_budget() {
            return Err(LibraryError::TooMuchData);
        }
        Ok(text)
    }

    /// The SVG outline file `rel`.
    fn outline(&mut self, rel: &str) -> Result<Geometry, String> {
        let rel = normalized_path(rel).map_err(|e| e.to_string())?;
        if let Some(outline) = self.outlines.get(&rel) {
            return outline.clone();
        }
        let outline = self
            .read(&rel)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_outline(&text).map_err(|e| format!("`{}`: {e}", quote(&rel))));
        self.outlines.insert(rel, outline.clone());
        outline
    }
}

/// `rel`, checked by [`check_relative_path`], without `.` parts or
/// repeated `/`, so each file has one name.
fn normalized_path(rel: &str) -> Result<String, LibraryError> {
    check_relative_path(rel)?;
    let parts: Vec<&str> = rel
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.is_empty() {
        // `.` names the folder itself; reading it reports that.
        return Ok(rel.to_owned());
    }
    Ok(parts.join("/"))
}

/// Accepts only plain relative paths (`shapes/a.svg`, `./a.svg`).
fn check_relative_path(rel: &str) -> Result<(), LibraryError> {
    if rel.is_empty() || rel.contains(['\\', '\0']) {
        return Err(outside_folder(rel));
    }
    let plain = Path::new(rel)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if plain {
        Ok(())
    } else {
        Err(outside_folder(rel))
    }
}

/// Loads one library. The report describes skipped shapes and
/// parameters, values replaced by defaults and (in its log) ignored fields.
pub fn load_library(
    files: &dyn LibraryFiles,
    builtin: bool,
) -> Result<(Library, LoadReport), LibraryError> {
    let mut report = LoadReport::default();
    let mut files = Reader::new(files);
    let files = &mut files;

    let table = read_toml(files, "library.toml")?;
    // Checked first: a newer format may change every other field.
    check_format(&table)?;
    warn_unknown(&table, LIBRARY_FIELDS, "library.toml", &mut report.log);
    let invalid = |e: String| LibraryError::Invalid(format!("library.toml: {e}"));
    let id = valid_id(required_str(&table, "id").map_err(invalid)?).map_err(invalid)?;
    let name = valid_name(required_str(&table, "name").map_err(invalid)?).map_err(invalid)?;
    let version = required_str(&table, "version").map_err(invalid)?;
    if version.trim().is_empty() {
        return Err(invalid("`version` is empty".into()));
    }
    // Optional fields fall back with a warning, as shape fields do.
    let mut optional = |key: &str| {
        optional_str(&table, key).unwrap_or_else(|e| {
            report.notices.push(format!("library.toml: {e}; ignored"));
            None
        })
    };
    let author = optional("author");
    let description = optional("description");
    let icon_value = optional("icon");
    let order = match table.get("order") {
        None => 0,
        Some(Value::Integer(n)) => i32::try_from(*n).unwrap_or_else(|_| {
            report
                .notices
                .push("library.toml: `order` is out of range; using 0".into());
            0
        }),
        Some(_) => {
            report
                .notices
                .push("library.toml: `order` must be a whole number; using 0".into());
            0
        }
    };
    let icon = load_icon(
        files,
        icon_value,
        Icon::Lucide(DEFAULT_ICON.into()),
        "library.toml",
        &mut report.notices,
    );
    if files.over_budget() {
        return Err(LibraryError::TooMuchData);
    }

    let mut table = read_toml(files, "shapes.toml")?;
    let entries = match table.remove("shape") {
        None => Vec::new(),
        Some(Value::Array(entries)) => entries,
        Some(_) => {
            return Err(LibraryError::Invalid(
                "shapes.toml: shapes must be written as [[shape]] tables".into(),
            ))
        }
    };
    warn_unknown(&table, &[], "shapes.toml", &mut report.log);
    if entries.len() > MAX_SHAPES {
        return Err(LibraryError::TooManyShapes(entries.len()));
    }
    let mut shapes: Vec<ShapeDef> = Vec::with_capacity(entries.len());
    let mut ids = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        match load_shape(files, entry, index + 1, &ids, &mut report) {
            Ok(shape) => {
                ids.insert(shape.id.clone());
                shapes.push(shape);
            }
            Err(warning) => report.notices.push(warning),
        }
        if files.over_budget() {
            return Err(LibraryError::TooMuchData);
        }
    }

    let library = Library {
        id: id.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        author: author.map(str::to_owned),
        description: description.map(str::to_owned),
        icon,
        order,
        builtin,
        shapes,
    };
    Ok((library, report))
}

fn read_toml(files: &mut Reader, rel: &str) -> Result<Table, LibraryError> {
    let text = files.read(rel)?;
    text.parse::<Table>().map_err(|e| {
        let message = e.message().trim_end();
        LibraryError::Toml(match e.span() {
            Some(span) => {
                let start = span.start.min(text.len());
                let line = text.as_bytes()[..start]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count()
                    + 1;
                format!("{rel}, line {line}: {message}")
            }
            None => format!("{rel}: {message}"),
        })
    })
}

fn check_format(table: &Table) -> Result<(), LibraryError> {
    match table.get("format") {
        None => Err(LibraryError::Invalid(format!(
            "library.toml has no `format` field; add `format = {LIBRARY_FORMAT}`"
        ))),
        Some(Value::Integer(n)) if *n == i64::from(LIBRARY_FORMAT) => Ok(()),
        Some(Value::Integer(n)) if *n > i64::from(LIBRARY_FORMAT) => Err(
            LibraryError::NewerFormat(u32::try_from(*n).unwrap_or(u32::MAX)),
        ),
        Some(Value::Integer(n)) => Err(LibraryError::Invalid(format!(
            "library.toml: library format {n} is not supported"
        ))),
        Some(_) => Err(LibraryError::Invalid(
            "library.toml: `format` must be a whole number".into(),
        )),
    }
}

/// Adds one log line per key of `table` not in `known`.
fn warn_unknown(table: &Table, known: &[&str], context: &str, log: &mut Vec<String>) {
    for key in table.keys().filter(|k| !known.contains(&k.as_str())) {
        log.push(format!("{context}: unknown field `{}` ignored", quote(key)));
    }
}

fn optional_str<'a>(table: &'a Table, key: &str) -> Result<Option<&'a str>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn required_str<'a>(table: &'a Table, key: &str) -> Result<&'a str, String> {
    optional_str(table, key)?.ok_or_else(|| format!("`{key}` is missing"))
}

/// A finite number (TOML integer or float), if present.
fn optional_number(table: &Table, key: &str) -> Result<Option<f32>, String> {
    let n = match table.get(key) {
        None => return Ok(None),
        Some(Value::Integer(i)) => *i as f32,
        Some(Value::Float(f)) => *f as f32,
        Some(_) => return Err(format!("`{key}` must be a number")),
    };
    if n.is_finite() {
        Ok(Some(n))
    } else {
        Err(format!("`{key}` must be a finite number"))
    }
}

/// Ids are stored in documents, so they are kept simple and stable.
fn valid_id(id: &str) -> Result<&str, String> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(id)
    } else {
        Err(format!(
            "id `{}` must be 1 to {MAX_ID_LEN} lowercase letters, digits, `-`, `_` or `.`, \
             starting with a letter or digit",
            quote(id)
        ))
    }
}

fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        Err("`name` is empty".into())
    } else if name.chars().count() > MAX_NAME_LEN {
        Err(format!("`name` is longer than {MAX_NAME_LEN} characters"))
    } else {
        Ok(name)
    }
}

fn valid_lucide_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ID_LEN
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The icon named by `value` (a Lucide name, or an SVG file when it ends
/// in `.svg`), or `fallback` with a warning when it cannot be used.
fn load_icon(
    files: &mut Reader,
    value: Option<&str>,
    fallback: Icon,
    context: &str,
    warnings: &mut Vec<String>,
) -> Icon {
    let Some(value) = value else {
        return fallback;
    };
    if value.ends_with(".svg") {
        match files.outline(value) {
            Ok(g) => Icon::Outline(g),
            Err(e) => {
                warnings.push(format!("{context}: icon {e}; the default icon is used"));
                fallback
            }
        }
    } else if valid_lucide_name(value) {
        Icon::Lucide(value.to_owned())
    } else {
        warnings.push(format!(
            "{context}: icon `{}` is neither a Lucide icon name nor an .svg file; \
             the default icon is used",
            quote(value)
        ));
        fallback
    }
}

/// Loads `[[shape]]` number `number` (1-based). `Err` is the warning for
/// a skipped shape; smaller problems add warnings and use defaults.
fn load_shape(
    files: &mut Reader,
    entry: &Value,
    number: usize,
    taken_ids: &HashSet<String>,
    report: &mut LoadReport,
) -> Result<ShapeDef, String> {
    let Value::Table(table) = entry else {
        return Err(format!(
            "shapes.toml: shape #{number} is not a table; skipped"
        ));
    };
    let context = match table.get("id") {
        Some(Value::String(id)) => format!("shape `{}`", quote(id)),
        _ => format!("shape #{number}"),
    };
    let skip = |e: String| format!("{context}: {e}; skipped");

    let id = valid_id(required_str(table, "id").map_err(skip)?).map_err(skip)?;
    if taken_ids.contains(id) {
        return Err(skip("duplicate shape id".into()));
    }
    let name = valid_name(required_str(table, "name").map_err(skip)?).map_err(skip)?;
    let primitive = optional_str(table, "primitive").map_err(skip)?;
    let svg = optional_str(table, "svg").map_err(skip)?;
    let source = match (primitive, svg) {
        (Some(p), None) => ShapeSource::Primitive(
            Primitive::from_name(p)
                .ok_or_else(|| skip(format!("unknown primitive `{}`", quote(p))))?,
        ),
        (None, Some(rel)) => ShapeSource::Outline(files.outline(rel).map_err(skip)?),
        (Some(_), Some(_)) => return Err(skip("has both `primitive` and `svg`".into())),
        (None, None) => return Err(skip("has neither `primitive` nor `svg`".into())),
    };
    // Only reached for shapes that load, so warnings are not doubled up
    // with a skip message.
    warn_unknown(table, SHAPE_FIELDS, &context, &mut report.log);

    let default_icon = match &source {
        ShapeSource::Outline(g) => Icon::Outline(g.clone()),
        ShapeSource::Primitive(_) => Icon::Lucide(DEFAULT_ICON.into()),
    };
    let icon = match optional_str(table, "icon") {
        Ok(value) => load_icon(files, value, default_icon, &context, &mut report.notices),
        Err(e) => {
            report
                .notices
                .push(format!("{context}: {e}; the default icon is used"));
            default_icon
        }
    };
    let default_size = load_default_size(table.get("default_size")).unwrap_or_else(|e| {
        report
            .notices
            .push(format!("{context}: {e}; using 120 × 80"));
        DEFAULT_SIZE
    });
    let style = match table.get("style") {
        None => StyleDefaults::default(),
        Some(Value::Table(style)) => load_style(style, &context, report),
        Some(_) => {
            report
                .notices
                .push(format!("{context}: `style` must be a table; defaults used"));
            StyleDefaults::default()
        }
    };
    let params = match (&source, table.get("param")) {
        (_, None) => Vec::new(),
        (ShapeSource::Outline(_), Some(_)) => {
            report.notices.push(format!(
                "{context}: SVG shapes have no parameters; `param` ignored"
            ));
            Vec::new()
        }
        (ShapeSource::Primitive(kind), Some(Value::Array(entries))) => {
            load_params(*kind, entries, &context, report)
        }
        (ShapeSource::Primitive(_), Some(_)) => {
            report.notices.push(format!(
                "{context}: parameters must be written as [[shape.param]] tables; ignored"
            ));
            Vec::new()
        }
    };

    Ok(ShapeDef {
        id: id.to_owned(),
        name: name.to_owned(),
        icon,
        default_size,
        style,
        source,
        params,
    })
}

fn load_default_size(value: Option<&Value>) -> Result<(f32, f32), String> {
    let Some(value) = value else {
        return Ok(DEFAULT_SIZE);
    };
    let as_size = |v: &Value| match v {
        Value::Integer(i) => Some(*i as f32),
        Value::Float(f) => Some(*f as f32),
        _ => None,
    };
    let in_range = |n: f32| (1.0..=MAX_DEFAULT_SIZE).contains(&n);
    match value {
        Value::Array(a) if a.len() == 2 => match (as_size(&a[0]), as_size(&a[1])) {
            (Some(w), Some(h)) if in_range(w) && in_range(h) => Ok((w, h)),
            _ => Err(format!(
                "`default_size` values must be from 1 to {MAX_DEFAULT_SIZE}"
            )),
        },
        _ => Err("`default_size` must be [width, height]".into()),
    }
}

fn load_style(table: &Table, context: &str, report: &mut LoadReport) -> StyleDefaults {
    let context = format!("{context} style");
    warn_unknown(table, STYLE_FIELDS, &context, &mut report.log);
    let mut colour = |key: &str, default: &str| match table.get(key) {
        None => Some(default.to_owned()),
        Some(Value::String(s)) if s.trim() == "none" => None,
        Some(Value::String(s)) => normalize_hex_color(s).or_else(|| {
            report.notices.push(format!(
                "{context}: `{key}` \"{}\" is not a #RRGGBB colour; no {key} is used",
                quote(s)
            ));
            None
        }),
        Some(_) => {
            report.notices.push(format!(
                "{context}: `{key}` must be a \"#RRGGBB\" string; no {key} is used"
            ));
            None
        }
    };
    let fill = colour("fill", DEFAULT_FILL);
    let stroke = colour("stroke", DEFAULT_STROKE);
    let mut number = |key: &str, max: f32, default: f32| match optional_number(table, key) {
        Ok(None) => default,
        Ok(Some(n)) if (0.0..=max).contains(&n) => n,
        Ok(Some(_)) => {
            report.notices.push(format!(
                "{context}: `{key}` must be from 0 to {max}; using {default}"
            ));
            default
        }
        Err(e) => {
            report
                .notices
                .push(format!("{context}: {e}; using {default}"));
            default
        }
    };
    let stroke_width = number("stroke_width", MAX_STROKE_WIDTH, DEFAULT_STROKE_WIDTH);
    let opacity = number("opacity", 1.0, DEFAULT_OPACITY);
    StyleDefaults {
        fill,
        stroke,
        stroke_width,
        opacity,
    }
}

fn load_params(
    kind: Primitive,
    entries: &[Value],
    context: &str,
    report: &mut LoadReport,
) -> Vec<ParamDef> {
    let mut params: Vec<ParamDef> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let number = index + 1;
        let Value::Table(table) = entry else {
            report.notices.push(format!(
                "{context}: parameter #{number} is not a table; ignored"
            ));
            continue;
        };
        let param_context = match table.get("key") {
            Some(Value::String(key)) => format!("{context} parameter `{}`", quote(key)),
            _ => format!("{context} parameter #{number}"),
        };
        match load_param(kind, table, &param_context, &params, report) {
            Ok(param) => params.push(param),
            Err(e) => report
                .notices
                .push(format!("{param_context}: {e}; ignored")),
        }
    }
    params
}

fn load_param(
    kind: Primitive,
    table: &Table,
    context: &str,
    earlier: &[ParamDef],
    report: &mut LoadReport,
) -> Result<ParamDef, String> {
    let key = required_str(table, "key")?;
    let Some((lowest, highest)) = kind.param_range(key) else {
        return Err(format!("the primitive has no parameter `{}`", quote(key)));
    };
    if earlier.iter().any(|p| p.key == key) {
        return Err("declared twice".into());
    }
    let label = match optional_str(table, "label")? {
        Some(label) => valid_name(label)
            .map_err(|e| e.replace("`name`", "`label`"))?
            .to_owned(),
        None => key_to_label(key),
    };
    let min = optional_number(table, "min")?.ok_or("`min` is missing")?;
    let max = optional_number(table, "max")?.ok_or("`max` is missing")?;
    if min > max {
        return Err("`min` is greater than `max`".into());
    }
    // The outline ignores values outside the primitive's own range, so
    // the sidebar must not offer them.
    let (min, max) = if min < lowest || max > highest {
        let clamped = (min.clamp(lowest, highest), max.clamp(lowest, highest));
        report.notices.push(format!(
            "{context}: the range {min} to {max} is wider than the primitive allows; \
             using {} to {}",
            clamped.0, clamped.1
        ));
        clamped
    } else {
        (min, max)
    };
    let step = optional_number(table, "step")?.unwrap_or(1.0);
    if step <= 0.0 {
        return Err("`step` must be greater than 0".into());
    }
    let mut default = optional_number(table, "default")?.unwrap_or(min);
    if !(min..=max).contains(&default) {
        default = default.clamp(min, max);
        report.notices.push(format!(
            "{context}: `default` is outside `min`..`max`; using {default}"
        ));
    }
    // Every field was read; report extras only for parameters that load.
    warn_unknown(table, PARAM_FIELDS, context, &mut report.log);
    Ok(ParamDef {
        key: key.to_owned(),
        label,
        min,
        max,
        step,
        default,
    })
}

/// `corner_radius` → `Corner radius`.
fn key_to_label(key: &str) -> String {
    let words = key.replace(['_', '-'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The Basic Shapes library files, compiled in.
const BASIC_SHAPES_FILES: &[(&str, &str)] = &[
    (
        "library.toml",
        include_str!("../libraries/basic-shapes/library.toml"),
    ),
    (
        "shapes.toml",
        include_str!("../libraries/basic-shapes/shapes.toml"),
    ),
    (
        "shapes/diamond.svg",
        include_str!("../libraries/basic-shapes/shapes/diamond.svg"),
    ),
];

/// The libraries compiled into the app. Panics only if a built-in library
/// is invalid, which the tests rule out.
pub fn builtin_libraries() -> Vec<Library> {
    let (basic_shapes, _) = load_library(&EmbeddedFiles(BASIC_SHAPES_FILES), true)
        .expect("the built-in Basic Shapes library is valid");
    vec![basic_shapes]
}

/// Where users install libraries: `<data dir>/Mindweave/libraries`
/// (`~/Library/Application Support/Mindweave/libraries` on macOS).
pub fn user_libraries_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Mindweave")
        .join("libraries")
}

/// The libraries available to the app, in toolbar order.
#[derive(Clone, Debug, Default)]
pub struct LibraryRegistry {
    libraries: Vec<Library>,
}

impl LibraryRegistry {
    /// Combines the built-in libraries with those in `user_dir` (one
    /// library per subfolder, at most [`MAX_USER_LIBRARIES`]). The report
    /// has notices for the user, one per skipped library and one per
    /// library with warnings, and log lines for ignored fields; each
    /// starts with the library folder name. A user library whose id is
    /// already taken is skipped.
    pub fn new(builtin: Vec<Library>, user_dir: Option<&Path>) -> (LibraryRegistry, LoadReport) {
        let mut notices = Vec::new();
        let mut log = Vec::new();
        let mut ids: HashSet<String> = HashSet::new();
        let mut libraries = Vec::new();
        for library in builtin {
            if ids.insert(library.id.clone()) {
                libraries.push(library);
            } else {
                notices.push(format!(
                    "{}: the library id is already in use; skipped",
                    library.id
                ));
            }
        }

        let mut user = Vec::new();
        for folder in user_dir.map_or_else(Vec::new, |dir| library_folders(dir, &mut notices)) {
            let label = folder
                .file_name()
                .map_or_else(String::new, |n| quote(&n.to_string_lossy()));
            match load_library(&DirFiles(folder), false) {
                Err(e) => notices.push(format!("{label}: {e}")),
                Ok((library, _)) if ids.contains(&library.id) => notices.push(format!(
                    "{label}: the library id `{}` is already in use; skipped",
                    quote(&library.id)
                )),
                Ok((library, report)) => {
                    if !report.notices.is_empty() {
                        notices.push(format!("{label}: {}", summarize(&report.notices)));
                    }
                    log.extend(report.log.iter().map(|line| format!("{label}: {line}")));
                    ids.insert(library.id.clone());
                    user.push(library);
                }
            }
        }
        user.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
        libraries.extend(user);
        let short = |lines: Vec<String>| lines.iter().map(|line| shorten_notice(line)).collect();
        let report = LoadReport {
            notices: short(notices),
            log: short(log),
        };
        (LibraryRegistry { libraries }, report)
    }

    /// Built-in libraries first, then user libraries by `order`, then name.
    pub fn libraries(&self) -> &[Library] {
        &self.libraries
    }

    /// The shape `shape` of library `library`, if both are installed.
    pub fn shape(&self, library: &str, shape: &str) -> Option<&ShapeDef> {
        self.libraries
            .iter()
            .find(|l| l.id == library)
            .and_then(|l| l.shape(shape))
    }
}

/// Subfolders of `dir`, sorted by name, skipping hidden ones, and at most
/// [`MAX_USER_LIBRARIES`] of them. A missing `dir` simply has none.
fn library_folders(dir: &Path, notices: &mut Vec<String>) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            notices.push(format!("The libraries folder could not be read: {e}"));
            return Vec::new();
        }
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    if folders.len() > MAX_USER_LIBRARIES {
        notices.push(format!(
            "{} library folders were not loaded: at most {MAX_USER_LIBRARIES} are read",
            folders.len() - MAX_USER_LIBRARIES
        ));
        folders.truncate(MAX_USER_LIBRARIES);
    }
    folders
}

/// Joins warnings into one notice, listing the first few.
fn summarize(warnings: &[String]) -> String {
    let mut text = warnings
        .iter()
        .take(MAX_NOTICE_WARNINGS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("; ");
    if warnings.len() > MAX_NOTICE_WARNINGS {
        text.push_str(&format!(
            " (and {} more)",
            warnings.len() - MAX_NOTICE_WARNINGS
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{bounds, PathCmd};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::fs;

    const LIB_TOML: &str = r#"
format = 1
id = "test-lib"
name = "Test Library"
version = "1.0.0"
"#;

    /// Writes a library folder `dir/name` and returns its path.
    fn write_lib(dir: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = dir.join(name);
        for (rel, text) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        root
    }

    /// Loads the library at `root`, with its notices and log lines as
    /// one list.
    fn load_dir(root: &Path) -> Result<(Library, Vec<String>), LibraryError> {
        load_library(&DirFiles(root.to_path_buf()), false)
            .map(|(lib, report)| (lib, [report.notices, report.log].concat()))
    }

    fn library_toml(id: &str, name: &str, order: i32) -> String {
        format!("format = 1\nid = \"{id}\"\nname = \"{name}\"\nversion = \"1\"\norder = {order}\n")
    }

    const ONE_RECT: &str = "[[shape]]\nid = \"r\"\nname = \"R\"\nprimitive = \"rect\"\n";

    #[test]
    fn builtin_basic_shapes_loads_cleanly() {
        let files = EmbeddedFiles(BASIC_SHAPES_FILES);
        let (lib, report) = load_library(&files, true).unwrap();
        assert_eq!(report, LoadReport::default());
        assert_eq!(lib.id, "basic-shapes");
        assert_eq!(lib.name, "Basic Shapes");
        assert_eq!(lib.order, 0);
        assert!(lib.builtin);
        let ids: Vec<_> = lib.shapes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "rectangle",
                "ellipse",
                "line",
                "triangle",
                "diamond",
                "star"
            ]
        );
        let icons: Vec<_> = lib
            .shapes
            .iter()
            .map(|s| match &s.icon {
                Icon::Lucide(name) => name.as_str(),
                Icon::Outline(_) => "<outline>",
            })
            .collect();
        assert_eq!(
            icons,
            ["square", "circle", "minus", "triangle", "diamond", "star"]
        );

        let rect = lib.shape("rectangle").unwrap();
        assert_eq!(rect.source, ShapeSource::Primitive(Primitive::Rect));
        assert_eq!(rect.default_size, (120.0, 80.0));
        assert_eq!(rect.style, StyleDefaults::default());
        assert_eq!(rect.style.fill.as_deref(), Some("#F5F5F5"));
        assert_eq!(rect.style.stroke.as_deref(), Some("#7F8EA3"));
        assert_eq!(
            rect.params,
            [ParamDef {
                key: "corner_radius".into(),
                label: "Corner radius".into(),
                min: 0.0,
                max: 1000.0,
                step: 1.0,
                default: 0.0,
            }]
        );

        let line = lib.shape("line").unwrap();
        assert_eq!(line.default_size, (120.0, 60.0));
        let line_style = StyleDefaults {
            fill: None,
            stroke: Some("#7F8EA3".into()),
            stroke_width: 2.0,
            opacity: 1.0,
        };
        assert_eq!(line.style, line_style);
        // Arrows are lines with heads now.
        assert!(lib.shape("arrow").is_none());

        let triangle = lib.shape("triangle").unwrap();
        assert_eq!(triangle.source, ShapeSource::Primitive(Primitive::Polygon));
        assert_eq!(triangle.params[0].key, "sides");
        assert_eq!(triangle.params[0].default, 3.0);

        assert!(matches!(
            lib.shape("diamond").unwrap().source,
            ShapeSource::Outline(_)
        ));

        let star = lib.shape("star").unwrap();
        let keys: Vec<_> = star.params.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["points", "inner_ratio"]);
        assert_eq!((star.params[0].min, star.params[0].max), (3.0, 20.0));
        assert_eq!(star.params[0].default, 5.0);
        assert_eq!((star.params[1].min, star.params[1].max), (0.1, 0.9));
        assert_eq!((star.params[1].step, star.params[1].default), (0.05, 0.5));
    }

    #[test]
    fn builtin_libraries_returns_basic_shapes() {
        let libs = builtin_libraries();
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].id, "basic-shapes");
        assert_eq!(libs[0].shapes.len(), 6);
    }

    #[test]
    fn loads_dir_library() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = r##"
[[shape]]
id = "box"
name = "Box"
icon = "square"
default_size = [200, 100]
primitive = "rect"
style = { fill = "#abcdef", stroke = "none", stroke_width = 3, opacity = 0.5 }

[[shape.param]]
key = "corner_radius"
min = 0
max = 50
default = 10

[[shape]]
id = "tri"
name = "Tri"
icon = "shapes/tri.svg"
svg = "shapes/tri.svg"
"##;
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><polygon points="5,0 10,10 0,10"/></svg>"#;
        let root = write_lib(
            dir.path(),
            "test",
            &[
                ("library.toml", LIB_TOML),
                ("shapes.toml", shapes),
                ("shapes/tri.svg", svg),
            ],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(lib.id, "test-lib");
        assert_eq!(lib.version, "1.0.0");
        assert_eq!(lib.icon, Icon::Lucide("shapes".into()));
        assert!(!lib.builtin);
        assert_eq!(lib.shapes.len(), 2);

        let rect = &lib.shapes[0];
        assert_eq!(rect.default_size, (200.0, 100.0));
        assert_eq!(
            rect.style,
            StyleDefaults {
                fill: Some("#ABCDEF".into()),
                stroke: None,
                stroke_width: 3.0,
                opacity: 0.5,
            }
        );
        assert_eq!(rect.params[0].label, "Corner radius");
        assert_eq!(rect.params[0].step, 1.0);
        assert_eq!(rect.params[0].default, 10.0);

        let tri = &lib.shapes[1];
        assert_eq!(tri.default_size, (120.0, 80.0));
        assert_eq!(tri.style, StyleDefaults::default());
        let ShapeSource::Outline(outline) = &tri.source else {
            panic!("expected an outline");
        };
        assert!(matches!(&tri.icon, Icon::Outline(g) if g == outline));
        let b = bounds(&tri.local_geometry(100.0, 50.0, &|_| None)).unwrap();
        assert!((b.w - 100.0).abs() < 1e-3 && (b.h - 50.0).abs() < 1e-3);
    }

    #[test]
    fn local_geometry_uses_param_defaults_and_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = r#"
[[shape]]
id = "r"
name = "R"
primitive = "rect"
[[shape.param]]
key = "corner_radius"
min = 0
max = 10
default = 5
"#;
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", shapes)],
        );
        let (lib, _) = load_dir(&root).unwrap();
        let shape = lib.shape("r").unwrap();
        let first_point = |g: Geometry| match g.paths[0].cmds[0] {
            PathCmd::MoveTo(p) => p,
            other => panic!("unexpected {other:?}"),
        };
        // The rounded rectangle starts after the top-left corner radius.
        assert_eq!(
            first_point(shape.local_geometry(100.0, 100.0, &|_| None)),
            (5.0, 0.0)
        );
        let huge = |_: &str| Some(40.0);
        assert_eq!(
            first_point(shape.local_geometry(100.0, 100.0, &huge)),
            (10.0, 0.0)
        );
        let nan = |_: &str| Some(f32::NAN);
        assert_eq!(
            first_point(shape.local_geometry(100.0, 100.0, &nan)),
            (5.0, 0.0)
        );
    }

    #[test]
    fn newer_format_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let lib = "format = 2\nid = \"x\"\nwhatever = { new = true }\n";
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", lib), ("shapes.toml", "")],
        );
        let err = load_dir(&root).unwrap_err();
        assert_eq!(err, LibraryError::NewerFormat(2));
        assert_eq!(
            err.to_string(),
            "needs a newer version of Mindweave (library format 2)"
        );
    }

    #[test]
    fn missing_format_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let lib = "id = \"x\"\nname = \"X\"\nversion = \"1\"\n";
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", lib), ("shapes.toml", ONE_RECT)],
        );
        let err = load_dir(&root).unwrap_err();
        assert!(
            matches!(&err, LibraryError::Invalid(m) if m.contains("format")),
            "{err:?}"
        );

        let zero = "format = 0\nid = \"x\"\nname = \"X\"\nversion = \"1\"\n";
        fs::write(root.join("library.toml"), zero).unwrap();
        assert!(matches!(load_dir(&root), Err(LibraryError::Invalid(_))));

        let text = "format = \"1\"\nid = \"x\"\nname = \"X\"\nversion = \"1\"\n";
        fs::write(root.join("library.toml"), text).unwrap();
        assert!(matches!(load_dir(&root), Err(LibraryError::Invalid(_))));
    }

    #[test]
    fn missing_files_and_fields_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = write_lib(dir.path(), "l", &[("library.toml", LIB_TOML)]);
        assert_eq!(
            load_dir(&root).unwrap_err(),
            LibraryError::Missing("shapes.toml".into())
        );
        let no_name = "format = 1\nid = \"x\"\nversion = \"1\"\n";
        fs::write(root.join("library.toml"), no_name).unwrap();
        fs::write(root.join("shapes.toml"), ONE_RECT).unwrap();
        let err = load_dir(&root).unwrap_err();
        assert!(
            matches!(&err, LibraryError::Invalid(m) if m.contains("name")),
            "{err:?}"
        );

        let bad_id = "format = 1\nid = \"Bad Id\"\nname = \"X\"\nversion = \"1\"\n";
        fs::write(root.join("library.toml"), bad_id).unwrap();
        assert!(matches!(load_dir(&root), Err(LibraryError::Invalid(_))));
    }

    #[test]
    fn invalid_toml_reports_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = "[[shape]]\nid = \"a\"\nname = = \"oops\"\n";
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", shapes)],
        );
        let err = load_dir(&root).unwrap_err();
        assert!(
            matches!(&err, LibraryError::Toml(m) if m.starts_with("shapes.toml, line 3")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut shapes = String::from(ONE_RECT);
        shapes.push_str(&"#".repeat(MAX_FILE_BYTES));
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", &shapes)],
        );
        assert_eq!(
            load_dir(&root).unwrap_err(),
            LibraryError::TooLarge("shapes.toml".into())
        );

        let big: &'static str = Box::leak("#".repeat(MAX_FILE_BYTES + 1).into_boxed_str());
        let files: &'static [(&'static str, &'static str)] =
            Box::leak(Box::new([("library.toml", big)]));
        assert_eq!(
            load_library(&EmbeddedFiles(files), true).unwrap_err(),
            LibraryError::TooLarge("library.toml".into())
        );
    }

    #[test]
    fn too_many_shapes_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let shapes: String = (0..=MAX_SHAPES)
            .map(|i| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nprimitive = \"rect\"\n"))
            .collect();
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", &shapes)],
        );
        assert_eq!(
            load_dir(&root).unwrap_err(),
            LibraryError::TooManyShapes(501)
        );

        let exactly: String = (0..MAX_SHAPES)
            .map(|i| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nprimitive = \"rect\"\n"))
            .collect();
        fs::write(root.join("shapes.toml"), exactly).unwrap();
        assert_eq!(load_dir(&root).unwrap().0.shapes.len(), MAX_SHAPES);
    }

    #[test]
    fn unknown_primitive_skips_only_that_shape() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = r#"
[[shape]]
id = "blob"
name = "Blob"
primitive = "blobby"

[[shape]]
id = "r"
name = "R"
primitive = "rect"
"#;
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", shapes)],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!(lib.shapes.len(), 1);
        assert_eq!(lib.shapes[0].id, "r");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("blob") && warnings[0].contains("blobby"),
            "{warnings:?}"
        );
    }

    #[test]
    fn bad_shapes_are_skipped_with_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = r#"
[[shape]]
id = "bad-svg"
name = "Bad"
svg = "shapes/bad.svg"

[[shape]]
id = "missing-svg"
name = "Missing"
svg = "shapes/none.svg"

[[shape]]
id = "both"
name = "Both"
primitive = "rect"
svg = "shapes/bad.svg"

[[shape]]
id = "neither"
name = "Neither"

[[shape]]
name = "No id"
primitive = "rect"

[[shape]]
id = "ok"
name = "Ok"
primitive = "ellipse"

[[shape]]
id = "ok"
name = "Duplicate"
primitive = "line"
"#;
        let root = write_lib(
            dir.path(),
            "l",
            &[
                ("library.toml", LIB_TOML),
                ("shapes.toml", shapes),
                ("shapes/bad.svg", "<svg><script>alert(1)</script>"),
            ],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!(lib.shapes.len(), 1);
        assert_eq!(lib.shapes[0].name, "Ok");
        assert_eq!(warnings.len(), 6, "{warnings:?}");
        for id in [
            "bad-svg",
            "missing-svg",
            "both",
            "neither",
            "shape #5",
            "duplicate",
        ] {
            assert!(
                warnings.iter().any(|w| w.contains(id)),
                "{id}: {warnings:?}"
            );
        }
    }

    #[test]
    fn unknown_fields_warn_but_load() {
        let dir = tempfile::tempdir().unwrap();
        let lib = format!("{LIB_TOML}homepage = \"https://example.com\"\n");
        let shapes = r##"
future_section = 1

[[shape]]
id = "r"
name = "R"
primitive = "rect"
shadow = true
style = { fill = "#000000", glow = 2 }

[[shape.param]]
key = "corner_radius"
min = 0
max = 5
unit = "px"
"##;
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", &lib), ("shapes.toml", shapes)],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!(lib.shapes.len(), 1);
        assert_eq!(lib.shapes[0].params.len(), 1);
        assert_eq!(lib.shapes[0].style.fill.as_deref(), Some("#000000"));
        for field in ["homepage", "future_section", "shadow", "glow", "unit"] {
            assert!(
                warnings
                    .iter()
                    .any(|w| w.contains(field) && w.contains("ignored")),
                "{field}: {warnings:?}"
            );
        }
    }

    #[test]
    fn invalid_values_fall_back_with_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = r##"
[[shape]]
id = "r"
name = "R"
icon = "Not An Icon!"
default_size = [0, 80]
primitive = "rect"
style = { fill = "red", stroke = "#12345", stroke_width = -1, opacity = 2 }

[[shape.param]]
key = "sides"
min = 3
max = 4

[[shape.param]]
key = "corner_radius"
min = 10
max = 0

[[shape]]
id = "s"
name = "S"
primitive = "star"

[[shape.param]]
key = "points"
min = 3
max = 8
default = 50

[[shape.param]]
key = "points"
min = 3
max = 8
"##;
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", shapes)],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        let r = lib.shape("r").unwrap();
        assert_eq!(r.icon, Icon::Lucide("shapes".into()));
        assert_eq!(r.default_size, (120.0, 80.0));
        assert_eq!(r.style.fill, None);
        assert_eq!(r.style.stroke, None);
        assert_eq!(r.style.stroke_width, 1.5);
        assert_eq!(r.style.opacity, 1.0);
        assert!(r.params.is_empty());
        let s = lib.shape("s").unwrap();
        assert_eq!(s.params.len(), 1);
        assert_eq!(s.params[0].default, 8.0);
        // icon, size, fill, stroke, width, opacity, sides, min > max,
        // default clamped, duplicate key.
        assert_eq!(warnings.len(), 10, "{warnings:?}");
    }

    #[test]
    fn path_traversal_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let svg = r#"<svg viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#;
        fs::write(dir.path().join("x.svg"), svg).unwrap();
        let shapes = format!(
            "{ONE_RECT}\n[[shape]]\nid = \"up\"\nname = \"Up\"\nsvg = \"../x.svg\"\n\n\
             [[shape]]\nid = \"abs\"\nname = \"Abs\"\nsvg = \"{}\"\n",
            dir.path().join("x.svg").display()
        );
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", &shapes)],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!(lib.shapes.len(), 1);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|w| w.contains("not a path inside the library folder")));

        let files = DirFiles(root.clone());
        for rel in ["", "..", "a/../../x.svg", "/etc/passwd", "a\\..\\x.svg"] {
            assert!(files.read(rel, &mut no_charge).is_err(), "{rel}");
        }
        assert!(EmbeddedFiles(&[("../x", "")])
            .read("../x", &mut no_charge)
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_outside_folder_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let svg = r#"<svg viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#;
        fs::write(dir.path().join("outside.svg"), svg).unwrap();
        let shapes =
            format!("{ONE_RECT}\n[[shape]]\nid = \"link\"\nname = \"L\"\nsvg = \"link.svg\"\n");
        let root = write_lib(
            dir.path(),
            "l",
            &[
                ("library.toml", LIB_TOML),
                ("shapes.toml", &shapes),
                ("inside.svg", svg),
            ],
        );
        std::os::unix::fs::symlink(dir.path().join("outside.svg"), root.join("link.svg")).unwrap();
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!(lib.shapes.len(), 1);
        assert!(warnings[0].contains("not a path inside"), "{warnings:?}");

        // A link that stays inside the folder is fine.
        fs::remove_file(root.join("link.svg")).unwrap();
        std::os::unix::fs::symlink(root.join("inside.svg"), root.join("link.svg")).unwrap();
        let (lib, warnings) = load_dir(&root).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(lib.shapes.len(), 2);
    }

    #[test]
    fn registry_orders_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path();
        write_lib(
            user,
            "dup",
            &[
                ("library.toml", &library_toml("basic-shapes", "Mine", 0)),
                ("shapes.toml", ONE_RECT),
            ],
        );
        write_lib(
            user,
            "a",
            &[
                ("library.toml", &library_toml("zeta", "Zeta", 5)),
                ("shapes.toml", ONE_RECT),
            ],
        );
        write_lib(
            user,
            "b",
            &[
                ("library.toml", &library_toml("alpha", "Alpha", 5)),
                ("shapes.toml", ONE_RECT),
            ],
        );
        write_lib(
            user,
            "c",
            &[
                ("library.toml", &library_toml("first", "Zzz", -1)),
                ("shapes.toml", ONE_RECT),
            ],
        );
        write_lib(
            user,
            "d",
            &[
                ("library.toml", &library_toml("alpha", "Again", 9)),
                ("shapes.toml", ONE_RECT),
            ],
        );
        fs::write(user.join(".DS_Store"), "junk").unwrap();
        fs::write(user.join("notes.txt"), "not a library").unwrap();

        let (registry, report) = LibraryRegistry::new(builtin_libraries(), Some(user));
        let notices = report.notices;
        let ids: Vec<_> = registry.libraries().iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["basic-shapes", "first", "alpha", "zeta"]);
        assert!(registry.libraries()[0].builtin);
        assert_eq!(registry.libraries()[0].name, "Basic Shapes");
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert!(notices
            .iter()
            .any(|n| n.starts_with("dup: ") && n.contains("basic-shapes")));
        assert!(notices
            .iter()
            .any(|n| n.starts_with("d: ") && n.contains("alpha")));
    }

    #[test]
    fn registry_skips_broken_library_with_notice() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path();
        write_lib(user, "broken", &[("library.toml", "this is not toml")]);
        write_lib(
            user,
            "future",
            &[("library.toml", "format = 7\n"), ("shapes.toml", "")],
        );
        write_lib(user, "empty", &[]);
        fs::create_dir_all(user.join("empty")).unwrap();
        let warn_shapes = format!("{ONE_RECT}icon = \"Blue Icon\"\n");
        write_lib(
            user,
            "warns",
            &[
                ("library.toml", &library_toml("warns", "Warns", 0)),
                ("shapes.toml", &warn_shapes),
            ],
        );
        write_lib(
            user,
            "good",
            &[
                ("library.toml", &library_toml("good", "Good", 0)),
                ("shapes.toml", ONE_RECT),
            ],
        );

        let (registry, report) = LibraryRegistry::new(builtin_libraries(), Some(user));
        let notices = report.notices;
        let ids: Vec<_> = registry.libraries().iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["basic-shapes", "good", "warns"]);
        assert_eq!(notices.len(), 4, "{notices:?}");
        assert!(notices
            .iter()
            .any(|n| n.starts_with("broken: invalid TOML in library.toml")));
        assert!(notices
            .iter()
            .any(|n| n == "future: needs a newer version of Mindweave (library format 7)"));
        assert!(notices
            .iter()
            .any(|n| n == "empty: `library.toml` is missing"));
        assert!(notices
            .iter()
            .any(|n| n.starts_with("warns: ") && n.contains("Blue Icon")));

        let (registry, report) =
            LibraryRegistry::new(builtin_libraries(), Some(&user.join("does-not-exist")));
        assert_eq!(registry.libraries().len(), 1);
        assert_eq!(report, LoadReport::default());
        let (registry, report) = LibraryRegistry::new(builtin_libraries(), None);
        assert_eq!(registry.libraries().len(), 1);
        assert_eq!(report, LoadReport::default());
    }

    /// In-memory library files that count how often each file is read.
    struct CountingFiles {
        files: HashMap<String, String>,
        reads: RefCell<HashMap<String, usize>>,
    }

    impl CountingFiles {
        fn new(files: Vec<(String, String)>) -> Self {
            CountingFiles {
                files: files.into_iter().collect(),
                reads: RefCell::default(),
            }
        }

        fn reads(&self, rel: &str) -> usize {
            self.reads.borrow().get(rel).copied().unwrap_or(0)
        }
    }

    impl LibraryFiles for CountingFiles {
        fn read(&self, rel: &str, charge: Charge) -> Result<String, LibraryError> {
            *self.reads.borrow_mut().entry(rel.to_owned()).or_default() += 1;
            let text = self
                .files
                .get(rel)
                .ok_or_else(|| LibraryError::Missing(rel.into()))?;
            charge(text.len())?;
            Ok(text.clone())
        }
    }

    /// Library files that total the bytes they were allowed to read.
    struct MeteredFiles<F> {
        files: F,
        paid: RefCell<usize>,
    }

    impl<F: LibraryFiles> LibraryFiles for MeteredFiles<F> {
        fn read(&self, rel: &str, charge: Charge) -> Result<String, LibraryError> {
            self.files.read(rel, &mut |n| {
                charge(n)?;
                *self.paid.borrow_mut() += n;
                Ok(())
            })
        }
    }

    fn no_charge(_: usize) -> Result<(), LibraryError> {
        Ok(())
    }

    #[test]
    fn quote_shortens_long_values() {
        assert_eq!(quote("short"), "short");
        let exact = "x".repeat(MAX_QUOTED_CHARS);
        assert_eq!(quote(&exact), exact);
        let long = "é".repeat(MAX_QUOTED_CHARS + 1);
        assert_eq!(quote(&long), "é".repeat(MAX_QUOTED_CHARS) + "…");
    }

    #[test]
    fn huge_echoed_values_stay_short() {
        let dir = tempfile::tempdir().unwrap();
        // A 1 MB unknown key, and long ids and values that are echoed.
        let key = "k".repeat(MAX_FILE_BYTES - 200);
        let lib = format!("{}{key} = 1\n", library_toml("huge", "Huge", 0));
        let huge = "v".repeat(200_000);
        let shapes = format!(
            "[[shape]]\nid = \"{huge}\"\nname = \"Bad\"\nprimitive = \"rect\"\n\
             [[shape]]\nid = \"r\"\nname = \"R\"\nprimitive = \"{huge}\"\n\
             [[shape]]\nid = \"s\"\nname = \"S\"\nicon = \"{huge}\"\nprimitive = \"rect\"\n\
             style = {{ fill = \"{huge}\" }}\n"
        );
        write_lib(
            dir.path(),
            &"f".repeat(200),
            &[("library.toml", &lib), ("shapes.toml", &shapes)],
        );
        let (registry, report) = LibraryRegistry::new(vec![], Some(dir.path()));
        assert_eq!(registry.libraries()[0].shapes.len(), 1);
        assert_eq!(report.notices.len(), 1, "{report:?}");
        assert_eq!(report.log.len(), 1, "{report:?}");
        for text in report.notices.iter().chain(&report.log) {
            assert!(
                text.chars().count() <= MAX_NOTICE_CHARS + 1,
                "{}",
                text.len()
            );
            assert!(text.contains('…'), "{text}");
        }
    }

    #[test]
    fn library_over_the_byte_budget_is_skipped() {
        // 16 SVGs and the TOML files fit in 16 MB; the 17th crosses it.
        let big = "x".repeat((MAX_LIBRARY_BYTES - 4096) / 16);
        let count = 17;
        let mut files = vec![
            ("library.toml".to_owned(), LIB_TOML.to_owned()),
            (
                "shapes.toml".to_owned(),
                (0..count + 5)
                    .map(|i| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nsvg = \"{i}.svg\"\n"))
                    .collect(),
            ),
        ];
        files.extend((0..count + 5).map(|i| (format!("{i}.svg"), big.clone())));
        let files = CountingFiles::new(files);
        let err = load_library(&files, false).unwrap_err();
        assert_eq!(err, LibraryError::TooMuchData);
        assert_eq!(err.to_string(), "its files add up to more than 16 MB");
        // Reading stopped at the file that crossed the budget.
        assert_eq!(files.reads(&format!("{}.svg", count - 1)), 1);
        assert_eq!(files.reads(&format!("{count}.svg")), 0);
    }

    #[test]
    fn files_that_do_not_pay_still_count_towards_the_budget() {
        /// Library files that never call `charge`.
        struct Unpaid(CountingFiles);
        impl LibraryFiles for Unpaid {
            fn read(&self, rel: &str, _: Charge) -> Result<String, LibraryError> {
                self.0.read(rel, &mut no_charge)
            }
        }
        let big = "x".repeat(MAX_FILE_BYTES);
        let mut files = vec![
            ("library.toml".to_owned(), LIB_TOML.to_owned()),
            (
                "shapes.toml".to_owned(),
                (0..20)
                    .map(|i| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nsvg = \"{i}.svg\"\n"))
                    .collect(),
            ),
        ];
        files.extend((0..20).map(|i| (format!("{i}.svg"), big.clone())));
        let files = Unpaid(CountingFiles::new(files));
        assert_eq!(
            load_library(&files, false).unwrap_err(),
            LibraryError::TooMuchData
        );
        assert_eq!(files.0.reads("16.svg"), 0);
    }

    #[test]
    fn an_svg_used_several_times_is_read_once() {
        let svg = r#"<svg viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#;
        let shapes = "[[shape]]\nid = \"a\"\nname = \"A\"\nicon = \"a.svg\"\nsvg = \"a.svg\"\n\
                      [[shape]]\nid = \"b\"\nname = \"B\"\nsvg = \"a.svg\"\n\
                      [[shape]]\nid = \"c\"\nname = \"C\"\nsvg = \"none.svg\"\n\
                      [[shape]]\nid = \"d\"\nname = \"D\"\nsvg = \"none.svg\"\n";
        let files = CountingFiles::new(vec![
            (
                "library.toml".into(),
                format!("{LIB_TOML}icon = \"a.svg\"\n"),
            ),
            ("shapes.toml".into(), shapes.into()),
            ("a.svg".into(), svg.into()),
        ]);
        let (lib, report) = load_library(&files, false).unwrap();
        assert_eq!(lib.shapes.len(), 2);
        assert!(matches!(lib.icon, Icon::Outline(_)));
        assert_eq!(files.reads("a.svg"), 1);
        // A missing file is looked for once but reported for each shape.
        assert_eq!(files.reads("none.svg"), 1);
        assert_eq!(report.notices.len(), 2, "{report:?}");
    }

    #[test]
    fn differently_written_paths_to_one_svg_are_read_once() {
        let svg = r#"<svg viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#;
        let shapes: String = ["a.svg", "./a.svg", "././a.svg", ".//a.svg"]
            .iter()
            .enumerate()
            .map(|(i, rel)| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nsvg = \"{rel}\"\n"))
            .collect();
        let files = CountingFiles::new(vec![
            ("library.toml".into(), LIB_TOML.into()),
            ("shapes.toml".into(), shapes),
            ("a.svg".into(), svg.into()),
        ]);
        let (lib, report) = load_library(&files, false).unwrap();
        assert_eq!(report, LoadReport::default());
        assert_eq!(lib.shapes.len(), 4);
        let svg_reads: usize = files
            .reads
            .borrow()
            .iter()
            .filter(|(rel, _)| rel.ends_with(".svg"))
            .map(|(_, n)| n)
            .sum();
        assert_eq!(svg_reads, 1);
    }

    #[test]
    fn failed_reads_count_towards_the_byte_budget() {
        // Every shape names the same 1 MB file that is not UTF-8, each
        // through its own hard link, so every read is in full and fails.
        let dir = tempfile::tempdir().unwrap();
        let count = 20;
        let shapes: String = (0..count)
            .map(|i| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nsvg = \"bad{i}.svg\"\n"))
            .collect();
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", &shapes)],
        );
        fs::write(root.join("bad0.svg"), vec![0xFF; MAX_FILE_BYTES]).unwrap();
        for i in 1..count {
            fs::hard_link(root.join("bad0.svg"), root.join(format!("bad{i}.svg"))).unwrap();
        }
        assert_eq!(load_dir(&root).unwrap_err(), LibraryError::TooMuchData);
        // Reads stop before the budget is exceeded.
        let files = MeteredFiles {
            files: DirFiles(root),
            paid: RefCell::default(),
        };
        assert_eq!(
            load_library(&files, false).unwrap_err(),
            LibraryError::TooMuchData
        );
        let paid = *files.paid.borrow();
        assert!(paid <= MAX_LIBRARY_BYTES, "{paid}");
        assert!(paid > MAX_LIBRARY_BYTES - MAX_FILE_BYTES, "{paid}");
    }

    #[test]
    fn duplicate_library_ids_are_quoted_in_notices() {
        let dir = tempfile::tempdir().unwrap();
        let id = "i".repeat(MAX_ID_LEN);
        for folder in ["a", "b"] {
            write_lib(
                dir.path(),
                folder,
                &[
                    ("library.toml", &library_toml(&id, "Same", 0)),
                    ("shapes.toml", ONE_RECT),
                ],
            );
        }
        let (registry, report) = LibraryRegistry::new(vec![], Some(dir.path()));
        assert_eq!(registry.libraries().len(), 1);
        assert_eq!(
            report.notices,
            [format!(
                "b: the library id `{}` is already in use; skipped",
                quote(&id)
            )]
        );
    }

    #[test]
    fn user_library_folders_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..MAX_USER_LIBRARIES + 2 {
            fs::create_dir(dir.path().join(format!("lib{i:03}"))).unwrap();
        }
        let (_, report) = LibraryRegistry::new(vec![], Some(dir.path()));
        assert_eq!(report.notices.len(), MAX_USER_LIBRARIES + 1);
        assert_eq!(
            report.notices[0],
            "2 library folders were not loaded: at most 100 are read"
        );
        assert!(report.notices.iter().any(|n| n.starts_with("lib099: ")));
        assert!(!report.notices.iter().any(|n| n.starts_with("lib100: ")));
    }

    #[test]
    fn wrongly_typed_optional_library_fields_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let lib = format!("{LIB_TOML}author = 1\ndescription = []\norder = \"first\"\nicon = 5\n");
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", &lib), ("shapes.toml", ONE_RECT)],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!((lib.author, lib.description, lib.order), (None, None, 0));
        assert_eq!(lib.icon, Icon::Lucide("shapes".into()));
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        for key in ["author", "description", "order", "icon"] {
            assert!(
                warnings.iter().any(|w| w.contains(&format!("`{key}`"))),
                "{key}: {warnings:?}"
            );
        }

        let far = format!("{LIB_TOML}order = 99999999999\n");
        fs::write(root.join("library.toml"), far).unwrap();
        let (lib, warnings) = load_dir(&root).unwrap();
        assert_eq!(lib.order, 0);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn unknown_fields_are_logged_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = format!("{ONE_RECT}glow = true\n");
        write_lib(
            dir.path(),
            "future",
            &[
                ("library.toml", &format!("{LIB_TOML}homepage = \"x\"\n")),
                ("shapes.toml", &shapes),
            ],
        );
        let (registry, report) = LibraryRegistry::new(vec![], Some(dir.path()));
        assert_eq!(registry.libraries().len(), 1);
        assert!(report.notices.is_empty(), "{report:?}");
        assert_eq!(
            report.log,
            [
                "future: library.toml: unknown field `homepage` ignored",
                "future: shape `r`: unknown field `glow` ignored",
            ]
        );
    }

    #[test]
    fn param_ranges_are_clamped_to_the_primitive() {
        let dir = tempfile::tempdir().unwrap();
        let shapes = r#"
[[shape]]
id = "s"
name = "S"
primitive = "star"

[[shape.param]]
key = "points"
min = 1
max = 50
default = 40

[[shape.param]]
key = "inner_ratio"
min = 0
max = 1
step = 0.1

[[shape]]
id = "p"
name = "P"
primitive = "polygon"

[[shape.param]]
key = "sides"
min = 30
max = 40

[[shape]]
id = "r"
name = "R"
primitive = "rect"

[[shape.param]]
key = "corner_radius"
min = -5
max = 100000
"#;
        let root = write_lib(
            dir.path(),
            "l",
            &[("library.toml", LIB_TOML), ("shapes.toml", shapes)],
        );
        let (lib, warnings) = load_dir(&root).unwrap();
        let range = |shape: &str, i: usize| {
            let p = &lib.shape(shape).unwrap().params[i];
            (p.min, p.max, p.default)
        };
        assert_eq!(range("s", 0), (3.0, 20.0, 20.0));
        assert_eq!(range("s", 1), (0.1, 0.9, 0.1));
        assert_eq!(range("p", 0), (12.0, 12.0, 12.0));
        assert_eq!(range("r", 0), (0.0, 100000.0, 0.0));
        // points: range and default; inner_ratio, sides, corner_radius: range.
        assert_eq!(warnings.len(), 5, "{warnings:?}");
        assert!(
            warnings[0].contains("1 to 50") && warnings[0].contains("3 to 20"),
            "{warnings:?}"
        );
    }

    #[test]
    fn shape_lookup() {
        let (registry, _) = LibraryRegistry::new(builtin_libraries(), None);
        assert_eq!(registry.shape("basic-shapes", "star").unwrap().name, "Star");
        assert!(registry.shape("basic-shapes", "hexagon").is_none());
        assert!(registry.shape("flowchart", "star").is_none());
        let lib = &registry.libraries()[0];
        assert_eq!(lib.shape("diamond").unwrap().name, "Diamond");
        assert!(lib.shape("").is_none());
    }

    #[test]
    fn user_libraries_dir_is_under_app_data() {
        assert!(user_libraries_dir().ends_with("Mindweave/libraries"));
    }

    /// The files marked `<!-- {marker}: path -->` in `docs/libraries.md`,
    /// each taken from the fenced block that follows the marker.
    fn doc_files(marker: &str) -> Vec<(String, String)> {
        let doc = include_str!("../../../docs/libraries.md");
        let prefix = format!("<!-- {marker}: ");
        let mut files = Vec::new();
        let mut lines = doc.lines();
        while let Some(line) = lines.next() {
            let Some(rest) = line.strip_prefix(&prefix) else {
                continue;
            };
            let path = rest.strip_suffix(" -->").unwrap().to_owned();
            assert!(lines.next().unwrap().starts_with("```"), "{path}");
            let body: Vec<_> = lines.by_ref().take_while(|l| *l != "```").collect();
            files.push((path, body.join("\n") + "\n"));
        }
        files
    }

    #[test]
    fn docs_example_library_loads() {
        let files = doc_files("example-file");
        assert_eq!(files.len(), 3);
        let dir = tempfile::tempdir().unwrap();
        let borrowed: Vec<_> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        let root = write_lib(dir.path(), "my-flowchart", &borrowed);
        let (lib, warnings) = load_dir(&root).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(lib.id, "my-flowchart");
        let ids: Vec<_> = lib.shapes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["process", "decision"]);
        assert!(matches!(lib.shapes[1].icon, Icon::Outline(_)));
    }

    #[test]
    fn docs_basic_shapes_match_builtin() {
        let files = doc_files("basic-shapes-file");
        assert_eq!(files.len(), BASIC_SHAPES_FILES.len());
        for (path, text) in files {
            let builtin = BASIC_SHAPES_FILES.iter().find(|(p, _)| *p == path).unwrap();
            assert_eq!(text, builtin.1, "{path} differs from the built-in file");
        }
    }
}

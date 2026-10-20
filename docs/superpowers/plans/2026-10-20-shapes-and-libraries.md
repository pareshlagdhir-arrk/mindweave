# Shapes & Libraries (Milestone 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Data-file shape libraries (Home tools + Basic Shapes), a toolbar strip, placing/selecting/transforming/styling shapes, undo/redo and clipboard in Mindweave.

**Architecture:** All logic that can be pure lives in `mindweave-core` (gpui-free, unit-tested): geometry and SVG outline parsing, the library format and loader (Basic Shapes is embedded there), the `Shape` model and file format v2, snapshot undo history inside `DocumentSession`, transform/hit-test maths, and clipboard payloads. `mindweave-desktop` loads libraries into a gpui global, draws shapes with `PathBuilder`, and wires the toolbar, pointer interactions, properties panel and Edit actions to core.

**Tech Stack:** Rust 1.95, gpui-kit `=0.8.0` (gpui-component 0.8.0, gpui-pre 0.3.9 `PathBuilder`), serde/serde_json, `toml` (exact pinned version), `roxmltree` + `svgtypes` (exact pinned versions) for the SVG subset, uuid.

**Spec:** `docs/superpowers/specs/2026-10-20-shapes-and-libraries-design.md`

## Global Constraints

- `mindweave-core` stays gpui-free; every new core module is unit-tested.
- New dependencies are pinned with `=x.y.z` in `[workspace.dependencies]`; commit `Cargo.lock`.
- Library folder: `library.toml` + `shapes.toml` + optional `shapes/*.svg`; `library.toml` must have `format = 1`; higher format → library skipped with a notice; unknown TOML fields ignored (warning); unknown primitive → that shape skipped (warning).
- Library files are untrusted: max 1 MB per file, max 500 shapes per library, SVG parsed for geometry only (no scripts, images, styles, external refs); errors never panic.
- User libraries folder: `~/Library/Application Support/Mindweave/libraries/` (i.e. `dirs::data_dir()/Mindweave/libraries`). Duplicate library id: built-in wins, user duplicate skipped with a notice.
- Document format version **2**; v1 files load; shapes live in `Layer.objects`; shapes reference `library` + `shape` ids (no embedded geometry); missing library/shape → editable dashed placeholder, params/unknown fields preserved.
- Shape frame: `x, y, w, h` (non-negative), `rotation` (degrees), `flip_x`, `flip_y`.
- Selection acts on the **active layer only** (visible, unlocked shapes); changing active layer/page clears selection; paste/duplicate go to the active layer.
- Tool returns to Pointer after placing; **double-click a tool to lock** it (lock dot) until Escape/other tool; Shift constrains (square/circle, 45° lines; aspect on resize; 15° on rotate); Alt from centre.
- Shortcuts: V Pointer, H Hand, R Rectangle, O Ellipse, L Line, A Arrow (work in any tab; switch tab); Escape → Pointer/clear; Cmd+Z / Shift+Cmd+Z; Cmd+C/X/V; Cmd+D duplicate (+10,+10); Cmd+A select all (active layer); Delete/Backspace; arrows nudge 1, Shift+arrows 10.
- Undo: every document content change, bounded at 200 steps, one step per gesture; view state not in history; dirty iff content differs from last save.
- Toolbar: 32px strip under the title bar; library tabs left; hairline divider; 28px buttons, 16px monochrome icons, tooltips "Name (Key)". Pen / Point edit / Text are NOT shown in this milestone.
- Zed look: 12–13px text, hairline borders, small crisp selection handles; no frame drops panning ~2,000 basic shapes.

## Review Focus

1. **Malformed or hostile user library** (bad TOML, 5 MB file, `<script>` in SVG, empty/zero-size viewBox, 600 shapes) — that library or shape is skipped with a readable notice; the app starts. (Task 2: `rejects_oversized_file`, `svg_ignores_script_and_style`, `zero_size_viewbox_is_error`, `too_many_shapes_is_error`, `newer_format_is_skipped`.)
2. **Click without drag / tiny drag when placing** — shape gets its default size centred on the click, never a 0×0 shape. (Task 5: `drag_create_click_uses_default_size`.)
3. **Resizing past the opposite edge or to zero** — flips toggle, sizes stay ≥ 1 canvas unit, never negative/NaN. (Task 5: `resize_past_edge_flips`, `resize_never_below_min_size`.)
4. **Pasting non-Mindweave clipboard text** (plain text, other JSON, truncated JSON) — paste does nothing, no error dialog. (Task 6: `from_clipboard_rejects_foreign_text`.)
5. **Undo/redo around save and page/layer edits** — undo back to saved state clears the dirty marker; redo after a new edit is cleared; undoing a shape add on a page you're not viewing still works; view state survives undo. (Task 4: `undo_to_saved_state_is_clean`, `new_edit_clears_redo`, `undo_keeps_viewport`.)

---

## File Structure

```
crates/mindweave-core/
  libraries/basic-shapes/{library.toml, shapes.toml, shapes/diamond.svg}   embedded built-in library
  src/geometry.rs     PathCmd/SubPath/Geometry, primitives, transforms, bounds, flatten, hit-test
  src/svg.rs          SVG subset → normalised Geometry
  src/library.rs      Library/ShapeDef/ParamDef, loader, limits, registry, builtin_libraries()
  src/shape.rs        Shape, Frame, Style, param accessors
  src/history.rs      History<T> snapshot undo/redo
  src/transform.rs    Rect, Handle, drag_create, translate, resize, rotate, marquee, nudge
  src/clipboard.rs    clipboard payload, paste placement, duplicate
  src/document.rs     Layer.objects: Vec<Shape>
  src/format.rs       FORMAT_VERSION = 2
  src/session.rs      history-backed dirty, undo/redo, shape ops, gestures
docs/libraries.md     contributor guide
crates/mindweave-desktop/src/
  libraries.rs        Libraries global: load built-in + user, notices
  actions.rs          new actions + keymap
  menu_bar.rs         Edit menu enabled items, Duplicate, Delete
  editor/tool.rs      Tool enum, tool state
  editor/toolbar.rs   toolbar strip
  editor/shape_paint.rs   draw shapes, placeholders, selection chrome
  editor/interaction.rs   pointer gesture state machine (place, select, marquee, move, resize, rotate)
  editor/shape_properties.rs  selected-shape sidebar sections
  editor/mod.rs, canvas.rs, properties_panel.rs   wiring
```

---

### Task 1: Geometry and SVG outlines (core)

**Files:** Create `crates/mindweave-core/src/geometry.rs`, `crates/mindweave-core/src/svg.rs`; modify `lib.rs`, root `Cargo.toml` (add `roxmltree`, `svgtypes` pinned), core `Cargo.toml`.

**Interfaces — Produces:**
- `pub type Pt = (f32, f32);`
- `pub enum PathCmd { MoveTo(Pt), LineTo(Pt), QuadTo { ctrl: Pt, to: Pt }, CubicTo { c1: Pt, c2: Pt, to: Pt }, Close }`
- `pub enum FillRole { Shape, StrokeColor, None }` — `Shape` = filled with the shape's fill (closed outlines), `StrokeColor` = filled with stroke colour (arrowheads), `None` = stroke only (lines).
- `pub struct SubPath { pub cmds: Vec<PathCmd>, pub fill: FillRole }`, `pub struct Geometry { pub paths: Vec<SubPath> }` (derive `Clone, Debug, PartialEq, Default`).
- `pub enum Primitive { Rect, Ellipse, Line, Arrow, Polygon, Star }` with `pub fn from_name(&str) -> Option<Primitive>` (`"rect" | "ellipse" | "line" | "arrow" | "polygon" | "star"`) and `pub fn param_keys(self) -> &'static [&'static str]` = Rect `["corner_radius"]`, Arrow `["head_size"]`, Polygon `["sides"]`, Star `["points", "inner_ratio"]`, others `[]`.
- `pub fn primitive_geometry(kind: Primitive, w: f32, h: f32, param: &dyn Fn(&str) -> Option<f32>) -> Geometry` — local box `0..w × 0..h`. Rect: corner radius clamped to `min(w,h)/2`, arcs as cubics. Ellipse: 4 cubics. Line: `(0,0)→(w,h)`, `FillRole::None`. Arrow: the line plus a closed triangular head at `(w,h)` of length `head_size` (default 12) with `StrokeColor`. Polygon: `sides` (default 3, clamped 3..=12) regular polygon inscribed in the box, first vertex top-centre. Star: `points` (default 5, 3..=20), `inner_ratio` (default 0.5, 0.1..=0.9), first point top-centre.
- `pub fn scale_unit(g: &Geometry, w: f32, h: f32) -> Geometry` (0..1 → 0..w,0..h).
- `pub struct Frame { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub rotation: f32, pub flip_x: bool, pub flip_y: bool }` (serde, `Default`) — defined here, re-used by `shape.rs`.
- `pub fn to_canvas(local: &Geometry, f: &Frame) -> Geometry` — flip inside the box, translate to `(x,y)`, rotate `rotation` degrees clockwise about the box centre.
- `pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }` with `contains(Pt)`, `contains_rect(&Rect)`, `union`, `center`.
- `pub fn bounds(g: &Geometry) -> Option<Rect>` (control-point hull is acceptable).
- `pub fn flatten(g: &Geometry, tolerance: f32) -> Vec<(Vec<Pt>, FillRole)>`.
- `pub fn hit_test(g: &Geometry, p: Pt, stroke_tolerance: f32) -> bool` — true if inside any `Shape`/`StrokeColor` subpath (even-odd) or within `stroke_tolerance` of any segment.
- `svg.rs`: `pub enum SvgError { Parse(String), NoGeometry, ZeroSize, TooLarge }` (thiserror, user-facing); `pub fn parse_outline(src: &str) -> Result<Geometry, SvgError>` — reads `viewBox` (or width/height), collects `path d`, `rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon` (with `transform` ignored except `translate`/`scale`, document as limitation), ignores everything else (scripts, images, styles, text), normalises to the 0..1 box; `line`/`polyline` → `FillRole::None`, others `Shape`. Rejects input > 1 MB (`TooLarge`).

- [ ] **Step 1: Failing tests** (geometry.rs): `rect_without_radius_is_four_lines` (MoveTo + 3 LineTo + Close, bounds 0,0,100,50), `rect_radius_is_clamped_to_half_min_side` (radius 999 on 100×50 → bounds unchanged, contains 8 cubic/line segments), `ellipse_bounds_match_box`, `line_is_stroke_only`, `arrow_has_filled_head_at_end` (2 subpaths, second `StrokeColor`, a head vertex at (w,h)), `polygon_sides_are_clamped` (sides 1 → 3 vertices, 50 → 12), `star_has_twice_points_vertices`, `to_canvas_flips_and_rotates` (box 0,0,10,10 at x=100,y=0, flip_x maps local (0,0)→(110,0); rotation 90 maps local top-left to top-right of the box), `hit_test_inside_rect_and_near_line` (inside → true; 1 px off a line with tolerance 3 → true; far → false), `flatten_curve_respects_tolerance`.
  svg.rs: `parses_path_and_normalises` (viewBox 0 0 200 100, path M0 0 L200 100 → (0,0),(1,1)), `rect_and_circle_become_paths`, `svg_ignores_script_and_style`, `zero_size_viewbox_is_error`, `no_geometry_is_error`, `rejects_oversized_input`.
- [ ] **Step 2:** `cargo test -p mindweave-core geometry svg` → FAIL (not defined).
- [ ] **Step 3:** Implement per Interfaces.
- [ ] **Step 4:** `cargo test -p mindweave-core` → PASS; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** `feat(core): shape geometry and SVG outlines`

---

### Task 2: Library format, loader, Basic Shapes, contributor guide (core)

**Files:** Create `crates/mindweave-core/src/library.rs`, `crates/mindweave-core/libraries/basic-shapes/{library.toml,shapes.toml,shapes/diamond.svg}`, `docs/libraries.md`; modify `lib.rs`, root/core `Cargo.toml` (add `toml` pinned).

**Interfaces — Consumes:** Task 1 `Geometry`, `Primitive`, `svg::parse_outline`. **Produces:**
- `pub const LIBRARY_FORMAT: u32 = 1; pub const MAX_FILE_BYTES: usize = 1_048_576; pub const MAX_SHAPES: usize = 500;`
- `pub enum Icon { Lucide(String), Outline(Geometry) }`
- `pub struct ParamDef { pub key: String, pub label: String, pub min: f32, pub max: f32, pub step: f32, pub default: f32 }`
- `pub struct StyleDefaults { pub fill: Option<String>, pub stroke: Option<String>, pub stroke_width: f32, pub opacity: f32 }` (colours validated with existing `parse_hex_color`; invalid → warning, `None`).
- `pub enum ShapeSource { Primitive(Primitive), Outline(Geometry) }`
- `pub struct ShapeDef { pub id: String, pub name: String, pub icon: Icon, pub default_size: (f32, f32), pub style: StyleDefaults, pub source: ShapeSource, pub params: Vec<ParamDef> }` with `pub fn local_geometry(&self, w: f32, h: f32, value: &dyn Fn(&str) -> Option<f32>) -> Geometry` (primitive → `primitive_geometry` with value falling back to the ParamDef default; outline → `scale_unit`).
- `pub struct Library { pub id: String, pub name: String, pub version: String, pub author: Option<String>, pub description: Option<String>, pub icon: Icon, pub order: i32, pub builtin: bool, pub shapes: Vec<ShapeDef> }` with `pub fn shape(&self, id: &str) -> Option<&ShapeDef>`.
- `pub enum LibraryError { Missing(String), TooLarge(String), Toml(String), NewerFormat(u32), TooManyShapes(usize), Invalid(String) }` (thiserror; Display is user-facing, e.g. NewerFormat → "needs a newer version of Mindweave (library format N)").
- `pub trait LibraryFiles { fn read(&self, rel: &str) -> Result<String, LibraryError>; }` with `pub struct DirFiles(pub PathBuf)` (enforces `MAX_FILE_BYTES`, rejects `..`/absolute paths) and `pub struct EmbeddedFiles(pub &'static [(&'static str, &'static str)])`.
- `pub fn load_library(files: &dyn LibraryFiles, builtin: bool) -> Result<(Library, Vec<String>), LibraryError>` — warnings for ignored unknown fields, skipped shapes (unknown primitive, bad SVG, duplicate shape id), unknown param keys.
- `pub fn builtin_libraries() -> Vec<Library>` — Basic Shapes via `include_str!` + `EmbeddedFiles`; panics only if the embedded library is invalid (guarded by a test).
- `pub fn user_libraries_dir() -> PathBuf` = `dirs::data_dir()/Mindweave/libraries`.
- `pub struct LibraryRegistry { libraries: Vec<Library> }` with `pub fn new(builtin: Vec<Library>, user_dir: Option<&Path>) -> (LibraryRegistry, Vec<String>)` (notices: one per skipped library/duplicate id/warning group, each prefixed with the library folder name), `pub fn libraries(&self) -> &[Library]` (built-ins first, then user by `order`, then name), `pub fn shape(&self, library: &str, shape: &str) -> Option<&ShapeDef>`.
- Basic Shapes (`id = "basic-shapes"`, `name = "Basic Shapes"`, `order = 0`): `rectangle` (rect, param corner_radius 0..1000 step 1 default 0), `ellipse`, `line`, `arrow` (head_size 4..100 step 1 default 12), `triangle` (polygon, sides default 3), `diamond` (SVG outline `shapes/diamond.svg`), `star` (points 3..20 default 5; inner_ratio 0.1..0.9 step 0.05 default 0.5). Default size 120×80 (line/arrow 120×60), style: closed shapes fill `#F5F5F5`, stroke `#7F8EA3`, width 1.5, opacity 1; line/arrow no fill, stroke `#7F8EA3`, width 2. Icons: Lucide `square`, `circle`, `minus`, `arrow-up-right`, `triangle`, `diamond`, `star`.
- `docs/libraries.md`: folder layout, every field of both TOML files with types/defaults, the primitives and their params, SVG subset and limits, compatibility rules, install location, the Basic Shapes files as the worked example.

- [ ] **Step 1: Failing tests:** `builtin_basic_shapes_loads_cleanly` (no warnings, 7 shapes, ids as above), `loads_dir_library` (tempdir with a two-shape library incl. an SVG), `newer_format_is_skipped` (`format = 2` → `NewerFormat(2)`), `missing_format_is_invalid`, `rejects_oversized_file`, `too_many_shapes_is_error` (501), `unknown_primitive_skips_only_that_shape` (warning names it), `unknown_fields_warn_but_load`, `path_traversal_is_rejected` (`svg = "../x.svg"`), `registry_orders_and_dedupes` (user library with id `basic-shapes` skipped with notice; user libs sorted by order then name), `registry_skips_broken_library_with_notice`, `shape_lookup`.
- [ ] **Step 2:** `cargo test -p mindweave-core library` → FAIL.
- [ ] **Step 3:** Implement; write the Basic Shapes files and `docs/libraries.md`.
- [ ] **Step 4:** `cargo test -p mindweave-core` PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(core): data-file shape libraries and Basic Shapes`

---

### Task 3: Shape model and file format v2 (core)

**Files:** Create `crates/mindweave-core/src/shape.rs`; modify `document.rs` (`Layer.objects: Vec<Shape>`), `format.rs` (`FORMAT_VERSION = 2`), `session.rs` (normalisation of shapes on open), `lib.rs`.

**Interfaces — Consumes:** `Frame` (Task 1), `ShapeDef`/`StyleDefaults` (Task 2). **Produces:**
- `pub struct Style { pub fill: Option<String>, pub stroke: Option<String>, pub stroke_width: f32, pub opacity: f32 }` (serde defaults: none, none, 1.0, 1.0).
- `pub struct Shape { pub id: Uuid, pub library: String, pub shape: String, #[serde(flatten)] pub frame: Frame, pub style: Style, #[serde(default)] pub params: BTreeMap<String, serde_json::Value>, #[serde(default)] pub name: Option<String>, #[serde(default)] pub locked: bool, #[serde(flatten)] pub extra: serde_json::Map<String, serde_json::Value> }` (Clone, Debug, PartialEq).
- `impl Shape { pub fn new(def: &ShapeDef, library: &str, frame: Frame) -> Shape` (style + params from defaults, new uuid); `pub fn param(&self, key: &str) -> Option<f32>`; `pub fn set_param(&mut self, key: &str, v: f32)` }`.
- Open normalisation (not dirty): non-finite frame values → 0 (size → 1), `w`/`h` clamped to `1.0..=1e6`, `x`/`y`/`rotation` clamped (±1e9, rotation wrapped to `[0, 360)`), opacity clamped 0..1, stroke_width 0..100, invalid colours dropped.
- v1 files (objects `[]`) load; saving writes `"version": 2`.

- [ ] **Step 1: Failing tests:** `shape_roundtrip_preserves_unknown_fields_and_params` (JSON with `"future": {...}` and param `"glow": 3` survives load→save), `missing_library_shape_survives_save` (library `"aws"` not installed: bytes-equivalent object after roundtrip), `v1_file_loads_and_saves_as_v2`, `open_normalises_bad_shape_values` (NaN x, w −5, opacity 7 → clamped, not dirty), `shape_new_takes_library_defaults` (Basic Shapes rectangle → fill `#F5F5F5`, corner_radius 0, size 120×80).
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** `cargo test -p mindweave-core` PASS (existing format/session tests updated only where version 1 → 2 is asserted). **Step 5: Commit** `feat(core): shape model and document format v2`

---

### Task 4: Undo history and shape operations in the session (core)

**Files:** Create `crates/mindweave-core/src/history.rs`; modify `session.rs`.

**Interfaces — Consumes:** `Shape` (Task 3). **Produces:**
- `pub struct History<T: Clone> { .. }` with `pub fn new(cap: usize) -> Self`, `pub fn record(&mut self, before: T)` (pushes, clears redo, drops oldest beyond cap), `pub fn undo(&mut self, current: T) -> Option<T>`, `pub fn redo(&mut self, current: T) -> Option<T>`, `can_undo`, `can_redo`. `pub const HISTORY_CAP: usize = 200;`
- Session snapshots `Vec<Page>` content. `DocumentSession` gains: `saved: Vec<Page>` (content at last save/open), `history: History<Vec<Page>>`, `gesture: Option<Vec<Page>>`.
  - `pub fn is_dirty(&self) -> bool` = content of `self.doc.pages` ≠ `saved`, ignoring view state (each page's `viewport` and `active_layer`; document `active_page`). Implement via a `fn content_eq(a: &[Page], b: &[Page]) -> bool`.
  - Every existing content setter (`add_page`, `rename_page`, `add_layer`, `rename_layer`, `toggle_layer_visibility`, canvas setters) records a step only when it actually changes content (existing "same value = no-op" rules keep working).
  - `pub fn begin_gesture(&mut self)` / `pub fn end_gesture(&mut self)` — edits between them form one step (recorded at end only if content changed); nested begin is a no-op.
  - `pub fn undo(&mut self) -> bool`, `pub fn redo(&mut self) -> bool`, `pub fn can_undo(&self) -> bool`, `pub fn can_redo(&self) -> bool` — restoring keeps the current view state for pages that still exist (matched by page id) and clamps active indices.
  - Shapes (active page, active layer unless stated): `pub fn add_shapes(&mut self, shapes: Vec<Shape>)` (append = top), `pub fn remove_shapes(&mut self, ids: &[Uuid]) -> Vec<Shape>` (searches all layers of the active page), `pub fn update_shapes(&mut self, ids: &[Uuid], f: impl FnMut(&mut Shape))`, `pub fn shapes_on_active_layer(&self) -> &[Shape]`, `pub fn find_shape(&self, id: Uuid) -> Option<&Shape>`, `pub fn active_layer_selectable(&self) -> bool` (visible).
  - `save()` sets `saved` = current content.

- [ ] **Step 1: Failing tests:** history.rs `record_undo_redo_roundtrip`, `cap_drops_oldest`, `new_edit_clears_redo`; session.rs `undo_to_saved_state_is_clean` (save, add shape → dirty, undo → clean, redo → dirty), `gesture_is_one_step` (begin, 3 updates, end → one undo restores original), `empty_gesture_records_nothing`, `undo_keeps_viewport` (pan after edit, undo → viewport unchanged), `undo_restores_page_add_while_on_other_page`, `existing_edits_are_undoable` (rename layer, set_pattern each undo), `remove_shapes_returns_removed`, `update_shapes_marks_dirty_only_on_change`. All existing session tests still pass.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** `cargo test -p mindweave-core` PASS. **Step 5: Commit** `feat(core): undo history and shape operations`

---

### Task 5: Transforms, hit-testing helpers, marquee (core)

**Files:** Create `crates/mindweave-core/src/transform.rs`.

**Interfaces — Consumes:** `Frame`, `Rect`, `Pt`, `to_canvas`, `bounds` (Task 1). **Produces:**
- `pub const MIN_SIZE: f32 = 1.0; pub const CLICK_DRAG_THRESHOLD: f32 = 3.0; pub const DUPLICATE_OFFSET: f32 = 10.0;`
- `pub enum Handle { N, NE, E, SE, S, SW, W, NW }`
- `pub fn frame_aabb(f: &Frame) -> Rect` (axis-aligned bounds of the rotated box); `pub fn selection_bounds(frames: &[Frame]) -> Option<Rect>`.
- `pub fn drag_create(start: Pt, end: Pt, default_size: (f32, f32), line_like: bool, constrain: bool, from_center: bool) -> Frame` — drag shorter than `CLICK_DRAG_THRESHOLD` → default size centred on `start`; box shapes: normalised rect (constrain → square of the larger side, from_center → start is the centre); line-like: box between points with `flip_x = end.x < start.x`, `flip_y = end.y < start.y`, constrain snaps direction to 45° steps.
- `pub fn translate(frames: &[Frame], dx: f32, dy: f32) -> Vec<Frame>`.
- `pub fn resize(start: &[Frame], handle: Handle, start_pointer: Pt, pointer: Pt, keep_aspect: bool, from_center: bool) -> Vec<Frame>` — one frame: resize in the shape's rotated local axes (opposite handle fixed in canvas space); several: scale each frame's position/size relative to `selection_bounds` (rotation kept, Figma-style). Crossing the opposite edge toggles `flip_x`/`flip_y`; sizes ≥ `MIN_SIZE`.
- `pub fn rotate(start: &[Frame], center: Pt, start_pointer: Pt, pointer: Pt, snap: bool) -> Vec<Frame>` — rotates positions about `center` and adds the angle to each `rotation` (wrapped `[0,360)`); snap → nearest 15° of the delta.
- `pub fn marquee_selects(f: &Frame, marquee: &Rect) -> bool` — AABB fully inside.
- `pub fn handle_positions(f_or_bounds: &Frame) -> [(Handle, Pt); 8]` and `pub fn rotate_handle_position(f: &Frame, offset: f32) -> Pt` (canvas coords; the desktop converts to screen).

- [ ] **Step 1: Failing tests:** `drag_create_click_uses_default_size`, `drag_create_normalises_backwards_box`, `drag_create_constrain_makes_square`, `drag_create_from_center`, `line_backwards_sets_flips`, `line_constrain_snaps_45`, `resize_se_grows`, `resize_past_edge_flips`, `resize_never_below_min_size`, `resize_keep_aspect`, `resize_rotated_shape_keeps_opposite_corner_fixed`, `group_resize_scales_positions_keeps_rotation`, `rotate_snaps_to_15`, `rotate_group_moves_positions`, `marquee_requires_full_containment`, `aabb_of_rotated_square_is_larger`.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** PASS + clippy. **Step 5: Commit** `feat(core): shape transforms`

---

### Task 6: Clipboard payloads and duplicate (core)

**Files:** Create `crates/mindweave-core/src/clipboard.rs`.

**Interfaces — Consumes:** `Shape`, `Rect`, `selection_bounds`, `DUPLICATE_OFFSET`. **Produces:**
- `pub fn to_clipboard(shapes: &[Shape]) -> String` → `{"mindweave":"shapes","version":1,"shapes":[...]}`.
- `pub fn from_clipboard(text: &str) -> Option<Vec<Shape>>` — `None` unless marker and version 1 match and shapes parse (cap 10,000 shapes).
- `pub fn prepare_paste(shapes: Vec<Shape>, view: &Rect) -> Vec<Shape>` — fresh uuids; if the shapes' bounds intersect `view` keep positions, else translate so the bounds centre is at the view centre.
- `pub fn duplicate(shapes: &[Shape]) -> Vec<Shape>` — fresh uuids, offset `+DUPLICATE_OFFSET` both axes.

- [ ] **Step 1: Failing tests:** `clipboard_roundtrip`, `from_clipboard_rejects_foreign_text` (plain text, `{}`, other JSON, truncated, version 2), `paste_keeps_position_when_visible`, `paste_centres_when_offscreen`, `paste_and_duplicate_assign_new_ids`, `duplicate_offsets_by_10`.
- [ ] **Step 2–4:** FAIL → implement → PASS. **Step 5: Commit** `feat(core): clipboard and duplicate`

---

### Task 7: Library loading and toolbar (desktop)

**Files:** Create `crates/mindweave-desktop/src/libraries.rs`, `src/editor/tool.rs`, `src/editor/toolbar.rs`; modify `main.rs`/`app_state.rs` (init), `actions.rs` (tool actions + keys), `editor/mod.rs` (layout: toolbar row between title bar and the panels row).

**Interfaces — Consumes:** `LibraryRegistry::new`, `builtin_libraries`, `user_libraries_dir` (Task 2). **Produces:**
- `pub struct Libraries(pub LibraryRegistry)` (gpui `Global`); `pub fn init(cx: &mut App) -> Vec<String>` — notices are shown as warning notifications in the first window that opens (dashboard).
- `pub enum Tool { Pointer, Hand, Shape { library: String, shape: String } }` (Clone, PartialEq); `pub struct ToolState { pub tool: Tool, pub locked: bool, pub tab: String }` where `tab` is `"home"` or a library id; `impl ToolState { pub fn select(&mut self, tool: Tool, lock: bool); pub fn after_place(&mut self) /* → Pointer unless locked */; pub fn cancel(&mut self) /* → Pointer, unlock */ }` with unit tests `after_place_returns_to_pointer_unless_locked`, `cancel_unlocks`, `selecting_shape_switches_tab`.
- `EditorWindow` gains `pub tools: ToolState`.
- Actions (namespace `mindweave`): `ToolPointer` (v), `ToolHand` (h), `ToolRectangle` (r), `ToolEllipse` (o), `ToolLine` (l), `ToolArrow` (a), `Cancel` (escape) — keys bound in the editor context only, and not while a text input has focus.
- Toolbar: 32px strip; tabs "Home" + each library `name` (12px, active tab underlined/highlighted); divider; Home → Pointer (`mouse-pointer-2`), Hand (`hand`); library tab → its shapes using `Icon::Lucide` or a 16px drawn `Icon::Outline`. Single click selects; double-click selects + locks (lock dot 4px in accent at the button's corner). Tooltip "Rectangle (R)" for shapes with a built-in key, otherwise the name.
- Hand tool: left-drag pans (reuse existing pan code path).

- [ ] **Step 1:** Write `ToolState` unit tests → FAIL → implement → PASS.
- [ ] **Step 2:** Implement registry global, notices, toolbar, actions/keymap, Hand tool.
- [ ] **Step 3:** `cargo test --workspace` PASS; clippy clean; app launches 6 s with empty stderr. Manual (record in report what you could not check): tabs switch, tool highlight, double-click lock dot, V/H/R/O/L/A switch tools, a broken user library in App Support shows a notice.
- [ ] **Step 4: Commit** `feat(desktop): libraries and toolbar`

---

### Task 8: Drawing shapes and placing them (desktop)

**Files:** Create `crates/mindweave-desktop/src/editor/shape_paint.rs`, `src/editor/interaction.rs` (placement part); modify `editor/canvas.rs`.

**Interfaces — Consumes:** `ShapeDef::local_geometry`, `to_canvas`, `Viewport::canvas_to_screen`, `drag_create`, `Shape::new`, `session.add_shapes`, `ToolState::after_place`. **Produces:**
- `pub fn paint_shapes(window: &mut Window, bounds: Bounds<Pixels>, page: &Page, libs: &LibraryRegistry, vp: &Viewport, theme: &Theme)` — for each visible layer in stack order (bottom first), each shape in list order: build canvas geometry (placeholder = dashed `#7F8EA3` box + name text when the library/shape is missing), map to screen, fill (`FillRole::Shape` with `style.fill`, `StrokeColor` with stroke colour) then stroke (`stroke_width × zoom`, min 0.5 px), apply `opacity`. Skip shapes whose screen AABB is off-canvas.
- `pub enum Gesture { Idle, Placing { start: Pt, current: Pt, library: String, shape: String } , /* more variants in Task 9 */ }` stored on `EditorWindow` as `pub gesture: Gesture`.
- Placement: with a shape tool, left mouse-down on canvas starts `Placing`; move updates a preview (drawn with the shape's style at 60% opacity); mouse-up → `drag_create(start, end, def.default_size, line_like, shift, alt)` → `Shape::new` → `add_shapes` (one undo step) → select it (Task 9 field `selection`, introduce `pub selection: Vec<Uuid>` here) → `tools.after_place()`. Placement on a hidden active layer is refused with a notification "Show the layer to draw on it.".
- Performance: cache each shape's canvas-space `Geometry` keyed by `(shape id, frame, params hash, library shape)`; reuse across frames; measure in the report the frame time for 2,000 rectangles while panning (temporary debug counter, removed before commit).

- [ ] **Step 1:** Unit-test any pure helper you add (e.g. stroke width clamp, off-screen cull) → FAIL → implement → PASS.
- [ ] **Step 2:** Implement painting, placeholders, preview and placement.
- [ ] **Step 3:** `cargo test --workspace` PASS; clippy clean; launch check. Manual: draw each Basic Shape; Shift squares, Alt from centre; line backwards; tool returns to Pointer; locked tool keeps drawing; save + reopen shows shapes; a file with an unknown library shows a placeholder.
- [ ] **Step 4: Commit** `feat(desktop): draw and place shapes`

---

### Task 9: Selection and transform interactions (desktop)

**Files:** Modify `editor/interaction.rs`, `editor/shape_paint.rs` (selection chrome), `editor/canvas.rs`, `editor/mod.rs`, `actions.rs` (nudge actions).

**Interfaces — Consumes:** `hit_test`, `marquee_selects`, `translate`, `resize`, `rotate`, `handle_positions`, `rotate_handle_position`, `selection_bounds`, `session.begin_gesture/end_gesture/update_shapes`. **Produces:**
- `Gesture` variants: `Marquee { start: Pt, current: Pt, additive: bool }`, `Moving { start: Pt, originals: Vec<(Uuid, Frame)> }`, `Resizing { handle: Handle, start: Pt, originals: Vec<(Uuid, Frame)> }`, `Rotating { center: Pt, start: Pt, originals: Vec<(Uuid, Frame)> }`.
- `pub fn hit_shape(session: &DocumentSession, libs: &LibraryRegistry, p: Pt, tolerance: f32) -> Option<Uuid>` — topmost on the active layer only; hidden layer → none; locked skipped; placeholder shapes (missing library/shape) hit-test as their filled box.
- `Cancel` (Escape): if a gesture is active, abort it (restore originals, no undo step); else clear selection and `tools.cancel()`.
- Pointer tool: click shape → select (Shift toggles); click empty → clear; drag on empty → marquee (Shift adds); drag on selected shape → move; handle drag → resize (Shift aspect, Alt centre); rotate handle (12 px above top-centre in screen space) → rotate (Shift snaps 15°). Each drag wraps `begin_gesture`/`end_gesture`. Selection chrome: 1px accent box (rotated box for one shape, AABB for several), 8 handles 6×6 px white fill + accent border, rotate handle 6 px circle; marquee: accent 1px border, accent 8% fill. Cursor changes on handles (resize/rotate cursors where gpui provides them).
- `selection` cleared on active layer/page change and when shapes it references disappear (undo).
- Nudge actions: `NudgeLeft/Right/Up/Down` (arrows), `NudgeLeftFar/…` (shift-arrows) → `translate` 1 / 10 as one undo step each; editor context only, not while an input has focus.

- [ ] **Step 1:** Unit-test `hit_shape` ordering/active-layer rules with a session built in a test (pure function) → FAIL → implement → PASS.
- [ ] **Step 2:** Implement gestures, chrome, nudges.
- [ ] **Step 3:** tests/clippy/launch. Manual: click/shift-click/marquee; move; resize each handle incl. past the edge; rotate with snap; group resize/rotate; shapes on another layer not selectable; switching layer clears selection; each drag is one Cmd+Z.
- [ ] **Step 4: Commit** `feat(desktop): select, move, resize and rotate shapes`

---

### Task 10: Shape properties sidebar (desktop)

**Files:** Create `crates/mindweave-desktop/src/editor/shape_properties.rs`; modify `editor/properties_panel.rs`, `editor/mod.rs` (`commit_pending_edits`).

**Interfaces — Consumes:** `selection`, `selection_bounds`, `translate`/`resize` (for group W/H edits), `session.update_shapes`, `ShapeDef.params`, existing numeric-field and colour-picker helpers. **Produces:** `pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement` used by the properties panel when `selection` is non-empty (Canvas section otherwise).
- Sections: **Shape** (library name · shape name; "Missing library" for placeholders), **Transform** X, Y, W, H, Rotation (°) — single shape edits its frame; several edit the group bounds (W/H scale via `resize` semantics with the SE handle math, X/Y translate), **Style** Fill (colour + none toggle), Stroke (colour + none), Stroke width (0–100, step 0.5), Opacity (0–100 %), **Shape properties** from `ParamDef`s (number fields with min/max/step) shown only when all selected shapes share library+shape. Mixed values show placeholder "Mixed"; editing sets all. Each commit is one undo step.
- `commit_pending_edits` commits any focused field of this panel.

- [ ] **Step 1:** Unit-test the pure helpers (common value / Mixed resolution, percent↔opacity, group W/H → frames) → FAIL → implement → PASS.
- [ ] **Step 2:** Implement the panel.
- [ ] **Step 3:** tests/clippy/launch. Manual: change each field for one and several shapes; corner radius/sides/points/inner ratio update live; "Mixed" shows correctly; Cmd+Z per change.
- [ ] **Step 4: Commit** `feat(desktop): shape properties panel`

---

### Task 11: Edit actions and menu (desktop)

**Files:** Modify `actions.rs`, `menu_bar.rs`, `editor/mod.rs`.

**Interfaces — Consumes:** `session.undo/redo/can_undo/can_redo/remove_shapes/add_shapes`, `clipboard::{to_clipboard, from_clipboard, prepare_paste, duplicate}`, `selection`. **Produces:** actions `Undo` (cmd-z), `Redo` (cmd-shift-z), `Cut` (cmd-x), `Copy` (cmd-c), `Paste` (cmd-v), `Duplicate` (cmd-d), `Delete` (backspace, delete), `SelectAll` (cmd-a) — editor context, not intercepting text inputs (inputs keep their own copy/paste/undo). Clipboard via gpui `cx.write_to_clipboard(ClipboardItem::new_string(..))` / `cx.read_from_clipboard()`. Paste targets the active layer and selects pasted shapes; the view rect for `prepare_paste` is the visible canvas in canvas units.
- Edit menu: Undo, Redo, separator, Cut, Copy, Paste, Duplicate, Delete, separator, Select All — enabled in editor windows (Undo/Redo enabled state may stay always-enabled if gpui menus can't update live; document which). Update `menu_bar.rs` tests (`unbuilt_menus_are_present_but_disabled` no longer includes Edit).
- After undo/redo, prune `selection` to ids that still exist on the active layer.

- [ ] **Step 1:** Update/extend `menu_bar.rs` tests (Edit items present and enabled in Editor, disabled in Dashboard) → FAIL → implement → PASS.
- [ ] **Step 2:** Implement actions.
- [ ] **Step 3:** tests/clippy/launch. Manual: undo/redo every kind of edit; copy in one window, paste in another; paste off-screen centres; Cmd+D; Delete; Cmd+A; Cmd+Z inside a text field still edits the field.
- [ ] **Step 4: Commit** `feat(desktop): undo, redo, clipboard and edit menu`

---

### Task 12: Final verification

- [ ] **Step 1:** `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build -p mindweave-desktop --release` → all succeed.
- [ ] **Step 2:** Walk the spec's "Success looks like" list and the Review Focus items in the release build (hand to the user if no pointer automation is available).
- [ ] **Step 3:** Confirm `docs/libraries.md` lets a contributor create a library: create a sample library in a tempdir from the doc alone and load it with `LibraryRegistry::new` in a test (`docs_example_library_loads`, added to Task 2's test module if not already there).
- [ ] **Step 4: Commit** any polish `chore: shapes milestone polish`

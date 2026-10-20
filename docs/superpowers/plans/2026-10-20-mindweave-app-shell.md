# Mindweave App Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A macOS Rust/gpui desktop app, Mindweave, that opens a Figma-style dashboard and edits `.mindweave` files in Zed-style editor windows (layers, infinite canvas with pages, properties, menu bar, status bar, themes) — no drawable objects yet.

**Architecture:** Cargo workspace with two crates. `mindweave-core` is pure Rust (no gpui): document model and JSON format, edit operations with dirty tracking, viewport/grid maths, settings and recent files — this is where almost all tests live. `mindweave-desktop` (binary `mindweave`) is the gpui UI: one dashboard window plus one editor window per open document, built from `gpui-component` widgets, with thin views that call into core.

**Tech Stack:** Rust 1.95 stable, `gpui-kit = "=0.8.0"` (re-exports gpui as `gpui_kit::*`, `gpui_kit::component` = gpui-component 0.8.0, bundled Lucide icons via `gpui_kit::assets`), `serde`/`serde_json`, `uuid` (v4), `anyhow`, `thiserror`, `dirs`, `rfd` (native file dialogs), `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-10-20-app-shell-design.md`

## Global Constraints

- Product name **Mindweave**; package `mindweave-desktop`, binary `mindweave` (CI depends on these exact names).
- Document extension **`.mindweave`**, pretty-printed JSON, top-level `"version": 1`.
- Settings at `~/Library/Application Support/Mindweave/settings.json`.
- Pin `gpui-kit = "=0.8.0"`; commit `Cargo.lock`.
- macOS only target; CI = macOS only: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build -p mindweave-desktop --release`, artifact upload.
- Zed look: UI text 12–13px, small monochrome Lucide icons (14px), 1px hairline borders, no large icons, no gradients, no heavy shadows. Default theme is One Dark–style.
- One window per document; dashboard is its own window; opening an already-open file focuses its window.
- Background pattern and grid size are per page. Viewport, active page and active layer are saved with the file but never mark it dirty.
- No drawable objects, selection, undo of content, export, tabs, cloud — out of scope.

## Review Focus

1. **Corrupt or newer-version `.mindweave` file** — opening shows an error notification and leaves the app running; never panics. (Task 2: `load_rejects_newer_version`, `load_rejects_garbage`; Task 9 shows the error.)
2. **Rename to empty / whitespace-only name** for a page or layer — rename is ignored, old name kept. (Task 3: `rename_to_blank_is_ignored`.)
3. **Grid size typed as 0, negative, huge, or non-numeric** — clamped to 4..=200, non-numeric ignored. (Task 3: `grid_size_is_clamped`; Task 8 parses input with `str::parse::<f32>().ok()`.)
4. **Extreme zoom out** — grid never draws an unbounded number of lines; spacing adapts. (Task 4: `grid_lines_bounded_at_min_zoom`.)
5. **Corrupt or missing `settings.json`** — app starts with defaults instead of crashing. (Task 5: `load_settings_corrupt_falls_back_to_default`.)

---

## File Structure

```
Cargo.toml                         workspace (members = crates/*), shared deps
.gitignore                         remove `*.mindweave`
.github/workflows/ci.yml           macOS-only matrix
crates/mindweave-core/
  src/lib.rs                       re-exports
  src/document.rs                  Document, Page, Layer, CanvasSettings, BackgroundPattern, Viewport (serde)
  src/format.rs                    load/save, FORMAT_VERSION, FormatError
  src/session.rs                   DocumentSession: edit ops + dirty tracking
  src/viewport.rs                  pan/zoom maths, screen<->canvas, grid line generation
  src/settings.rs                  AppSettings, RecentFiles, load/save
crates/mindweave-desktop/
  src/main.rs                      app entry: init, themes, actions, keymap, native menu, open dashboard
  src/actions.rs                   gpui actions + keybindings
  src/app_state.rs                 global AppState: settings, open-window registry, open/focus/close logic
  src/themes.rs                    bundled theme JSON, load + apply + persist
  src/themes/mindweave.json        3 themes: "Mindweave One Dark", "Mindweave Graphite", "Mindweave Light"
  src/theme_picker.rs              Cmd+K Cmd+T fuzzy picker with live preview
  src/menu_bar.rs                  in-window menu bar (shared by both windows)
  src/dashboard.rs                 Dashboard view
  src/editor/mod.rs                EditorWindow root view (layout + save/close)
  src/editor/layers_panel.rs       left sidebar
  src/editor/properties_panel.rs   right sidebar
  src/editor/page_tabs.rs          bottom page tabs
  src/editor/canvas.rs             canvas element: pattern paint + pan/zoom input
  src/editor/status_bar.rs         status bar
```

---

### Task 1: Workspace scaffold, CI, empty window

**Files:**
- Create: `Cargo.toml`, `crates/mindweave-core/Cargo.toml`, `crates/mindweave-core/src/lib.rs`, `crates/mindweave-desktop/Cargo.toml`, `crates/mindweave-desktop/src/main.rs`, `rust-toolchain.toml` (channel `1.95`)
- Modify: `.gitignore` (delete line `*.mindweave`), `.github/workflows/ci.yml`

**Interfaces:**
- Produces: workspace `[workspace.dependencies]` entries for every crate listed in Tech Stack; `mindweave-desktop` has `[[bin]] name = "mindweave"`.

- [ ] **Step 1:** Create the workspace. `mindweave-desktop` depends on `gpui-kit = { version = "=0.8.0" }` and `mindweave-core = { path = "../mindweave-core" }`. `main.rs` follows the gpui-kit hello world: `gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| { gpui_kit::init(cx); gpui_kit::open_window(...) })`, wrapping the view in `gpui_kit::component::Root::new(view, window, cx)`; window title "Mindweave", 1200×800, the view renders an empty `div().size_full().bg(cx.theme().background)`.
- [ ] **Step 2:** Edit `ci.yml`: matrix `os: [macos-latest]`, delete the "Linux native build dependencies" step; leave every other step unchanged. Remove `*.mindweave` from `.gitignore`.
- [ ] **Step 3:** Run `cargo build -p mindweave-desktop && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check`. Expected: all succeed. Run `cargo run -p mindweave-desktop`; expected: a blank themed window titled Mindweave. (If `with_assets`/`Root` paths differ in 0.8.0, find the right ones in `~/.cargo/registry/src/*/gpui-component-0.8.0/src` and `gpui-kit-0.8.0/src`.)
- [ ] **Step 4: Commit** — `git add -A && git commit -m "chore: scaffold Mindweave workspace, macOS-only CI"`

---

### Task 2: Document model and file format

**Files:**
- Create: `crates/mindweave-core/src/document.rs`, `crates/mindweave-core/src/format.rs`
- Test: inline `#[cfg(test)] mod tests` in each file

**Interfaces:**
- Produces:
  - `pub enum BackgroundPattern { Lines, Dots, None }` — serde `rename_all = "lowercase"`, `Default = Dots`.
  - `pub struct CanvasSettings { pub pattern: BackgroundPattern, pub grid_size: f32 }` — default `Dots`, `20.0`.
  - `pub struct Viewport { pub x: f32, pub y: f32, pub zoom: f32 }` — canvas point at window top-left of the canvas area; default `0,0,1.0`.
  - `pub struct Layer { pub id: Uuid, pub name: String, pub visible: bool, pub objects: Vec<serde_json::Value> }` (objects always empty for now; `#[serde(default)]`).
  - `pub struct Page { pub id: Uuid, pub name: String, pub canvas: CanvasSettings, pub viewport: Viewport, pub layers: Vec<Layer>, pub active_layer: usize }`
  - `pub struct Document { pub version: u32, pub pages: Vec<Page>, pub active_page: usize }`
  - `impl Document { pub fn new() -> Self }` → one page "Page 1" with one layer "Layer 1".
  - `pub const FORMAT_VERSION: u32 = 1;`
  - `pub enum FormatError { Io(std::io::Error), Parse(serde_json::Error), UnsupportedVersion(u32), Empty }` (thiserror; Display messages are user-facing, e.g. "This file was made by a newer version of Mindweave (format 2).").
  - `pub fn load(path: &Path) -> Result<Document, FormatError>` — rejects `version > FORMAT_VERSION`, rejects zero pages or a page with zero layers (`Empty`), clamps out-of-range `active_page`/`active_layer` to 0.
  - `pub fn save(doc: &Document, path: &Path) -> Result<(), FormatError>` — pretty JSON, written to `path.with_extension("mindweave.tmp")` then renamed (atomic).

- [ ] **Step 1: Write failing tests** in `format.rs`:

```rust
#[test] fn new_document_has_one_page_one_layer() {
    let d = Document::new();
    assert_eq!(d.version, 1);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].name, "Page 1");
    assert_eq!(d.pages[0].layers[0].name, "Layer 1");
    assert_eq!(d.pages[0].canvas.pattern, BackgroundPattern::Dots);
    assert_eq!(d.pages[0].canvas.grid_size, 20.0);
}
#[test] fn round_trip_save_load() { /* tempdir; save Document::new() with a 2nd page; load; assert_eq! */ }
#[test] fn saved_json_is_pretty_and_lowercase_pattern() { /* contains "\n  " and "\"pattern\": \"dots\"" and "\"version\": 1" */ }
#[test] fn load_rejects_newer_version() { /* write {"version":2,...}; matches!(err, FormatError::UnsupportedVersion(2)) */ }
#[test] fn load_rejects_garbage() { /* write "not json"; matches!(err, FormatError::Parse(_)) */ }
#[test] fn load_clamps_bad_active_indices() { /* active_page: 9 → 0 */ }
```

- [ ] **Step 2:** `cargo test -p mindweave-core` → FAIL (types not defined).
- [ ] **Step 3:** Implement the types and `load`/`save` above. Derive `Debug, Clone, PartialEq, Serialize, Deserialize`.
- [ ] **Step 4:** `cargo test -p mindweave-core` → PASS.
- [ ] **Step 5: Commit** — `feat(core): document model and .mindweave format`

---

### Task 3: Document session — edits and dirty tracking

**Files:**
- Create: `crates/mindweave-core/src/session.rs`

**Interfaces:**
- Consumes: Task 2 types, `format::{load, save}`.
- Produces: `pub struct DocumentSession { pub doc: Document, pub path: PathBuf, dirty: bool }` with:
  - `pub fn create(path: PathBuf) -> Result<Self, FormatError>` (new doc, saved immediately, not dirty), `pub fn open(path: PathBuf) -> Result<Self, FormatError>`, `pub fn save(&mut self) -> Result<(), FormatError>` (clears dirty), `pub fn is_dirty(&self) -> bool`, `pub fn title(&self) -> String` (file stem, plus `" •"` when dirty).
  - Content edits (set dirty): `add_page(&mut self) -> usize` (name "Page N" where N = smallest unused number ≥ 1, becomes active), `rename_page(&mut self, idx: usize, name: &str)`, `add_layer(&mut self) -> usize` (on active page, inserted at top i.e. index 0, name "Layer N", becomes active), `rename_layer(&mut self, idx: usize, name: &str)`, `toggle_layer_visibility(&mut self, idx: usize)`, `set_pattern(&mut self, p: BackgroundPattern)`, `set_grid_size(&mut self, px: f32)`.
  - View edits (never dirty): `set_active_page(&mut self, idx: usize)`, `set_active_layer(&mut self, idx: usize)`, `set_viewport(&mut self, v: Viewport)`.
  - `pub fn page(&self) -> &Page` (active page).
  - Constants `GRID_MIN: f32 = 4.0`, `GRID_MAX: f32 = 200.0`.
- Rules: names are trimmed; blank → ignored, no dirty. Out-of-range indices are no-ops. Grid size is rounded to whole px and clamped to `GRID_MIN..=GRID_MAX`; NaN ignored. Setting a value equal to the current one does not mark dirty.

- [ ] **Step 1: Write failing tests:** `create_writes_file_and_is_clean`, `add_page_names_and_activates` (after adding then renaming "Page 2" to "Ideas", next add is "Page 2"), `add_layer_goes_on_top_and_activates`, `rename_to_blank_is_ignored` (`"   "` → name unchanged, `!is_dirty()`), `toggle_visibility_marks_dirty`, `grid_size_is_clamped` (0→4, -5→4, 999→200, 12.6→13, NaN→unchanged), `view_changes_do_not_mark_dirty` (set_viewport/set_active_page/set_active_layer → `!is_dirty()`, but after a later `save()` the reloaded file contains the new viewport), `title_shows_dirty_marker` ("arch" → "arch •").
- [ ] **Step 2:** `cargo test -p mindweave-core session` → FAIL.
- [ ] **Step 3:** Implement `DocumentSession` per Interfaces.
- [ ] **Step 4:** `cargo test -p mindweave-core` → PASS.
- [ ] **Step 5: Commit** — `feat(core): document session with edit ops and dirty tracking`

---

### Task 4: Viewport and grid maths

**Files:**
- Create: `crates/mindweave-core/src/viewport.rs`

**Interfaces:**
- Consumes: `Viewport`, `CanvasSettings`.
- Produces (all on plain `f32` tuples `(f32, f32)` so core stays gpui-free):
  - `pub const ZOOM_MIN: f32 = 0.1; pub const ZOOM_MAX: f32 = 10.0; pub const ZOOM_STEP: f32 = 1.25;`
  - `impl Viewport { pub fn screen_to_canvas(&self, p: (f32,f32)) -> (f32,f32); pub fn canvas_to_screen(&self, p: (f32,f32)) -> (f32,f32); pub fn pan_by(&mut self, screen_dx: f32, screen_dy: f32); pub fn zoom_at(&mut self, factor: f32, screen_anchor: (f32,f32)); pub fn zoom_percent(&self) -> u32 }` — `screen = (canvas - (x,y)) * zoom`; `pan_by` moves content with the pointer; `zoom_at` clamps to `ZOOM_MIN..=ZOOM_MAX` and keeps the canvas point under `screen_anchor` fixed.
  - `pub struct GridLines { pub xs: Vec<f32>, pub ys: Vec<f32>, pub spacing_px: f32, pub alpha: f32 }` (screen coords).
  - `pub fn grid_lines(vp: &Viewport, size: (f32,f32), grid_size: f32) -> GridLines` — effective spacing = `grid_size * zoom`, doubled repeatedly while `< 8.0` screen px; `alpha` = `((spacing_px - 8.0) / 8.0).clamp(0.3, 1.0)`; lines cover `0..=size` aligned to canvas multiples of the effective canvas spacing.

- [ ] **Step 1: Write failing tests:** `round_trip_screen_canvas`, `zoom_keeps_anchor_fixed` (anchor (300,200), factor 2 → `screen_to_canvas((300,200))` unchanged within 1e-3), `zoom_is_clamped` (zoom_at(1000) → 10.0; zoom_at(0.0001) → 0.1), `pan_moves_content` (`pan_by(10,0)` then a canvas point's screen x is +10), `grid_lines_aligned_to_canvas_multiples` (vp x=5, zoom 1, grid 20, width 100 → xs == [15,35,55,75,95]), `grid_lines_bounded_at_min_zoom` (zoom 0.1, grid 4, 4000×3000 → `xs.len() <= 4000/8 + 2` and `spacing_px >= 8.0`), `zoom_percent_rounds` (1.255 → 126).
- [ ] **Step 2:** `cargo test -p mindweave-core viewport` → FAIL.
- [ ] **Step 3:** Implement per Interfaces.
- [ ] **Step 4:** `cargo test -p mindweave-core` → PASS.
- [ ] **Step 5: Commit** — `feat(core): viewport transform and adaptive grid`

---

### Task 5: Settings and recent files

**Files:**
- Create: `crates/mindweave-core/src/settings.rs`

**Interfaces:**
- Produces:
  - `pub struct RecentFile { pub path: PathBuf, pub opened_at: u64 /* unix secs */ }`
  - `pub struct WindowSize { pub width: f32, pub height: f32 }`
  - `pub struct AppSettings { pub theme: String /* default "Mindweave One Dark" */, pub recent: Vec<RecentFile>, pub dashboard_window: Option<WindowSize>, pub editor_window: Option<WindowSize>, pub show_layers: bool /* true */, pub show_properties: bool /* true */ }` — all fields `#[serde(default)]`.
  - `pub const MAX_RECENT: usize = 20;`
  - `impl AppSettings { pub fn touch_recent(&mut self, path: &Path, now: u64); pub fn remove_recent(&mut self, path: &Path); pub fn recent_existing(&self) -> Vec<&RecentFile> }` — `touch_recent` moves/inserts to front, dedupes by canonicalised path (fallback to the raw path), truncates to `MAX_RECENT`; `recent_existing` filters to paths that exist.
  - `pub fn settings_path() -> PathBuf` = `dirs::data_dir()/Mindweave/settings.json` (on macOS this is Application Support).
  - `pub fn load_settings(path: &Path) -> AppSettings` (missing or corrupt → `AppSettings::default()`; never errors), `pub fn save_settings(s: &AppSettings, path: &Path) -> std::io::Result<()>` (creates parent dir, atomic write).

- [ ] **Step 1: Write failing tests:** `touch_recent_moves_to_front_and_dedupes`, `recent_is_capped_at_20`, `recent_existing_hides_missing_files`, `load_settings_missing_gives_default`, `load_settings_corrupt_falls_back_to_default`, `settings_round_trip`, `default_theme_is_one_dark` (`== "Mindweave One Dark"`).
- [ ] **Step 2:** `cargo test -p mindweave-core settings` → FAIL.
- [ ] **Step 3:** Implement per Interfaces.
- [ ] **Step 4:** `cargo test -p mindweave-core` → PASS.
- [ ] **Step 5: Commit** — `feat(core): app settings and recent files`

---

### Task 6: Themes and theme picker

**Files:**
- Create: `crates/mindweave-desktop/src/themes.rs`, `crates/mindweave-desktop/src/themes/mindweave.json`, `crates/mindweave-desktop/src/theme_picker.rs`

**Interfaces:**
- Consumes: `gpui_kit::component::{ThemeRegistry, Theme, ThemeConfig}`, `AppSettings.theme`.
- Produces:
  - `pub const THEMES_JSON: &str = include_str!("themes/mindweave.json");` — a gpui-component `ThemeSet` (`{"themes":[...]}`, schema in `gpui-component-0.8.0/src/theme/schema.rs`) with exactly: "Mindweave One Dark" (dark, One Dark palette: background `#282c34`, panels `#21252b`, borders `#181a1f`, text `#abb2bf`, muted `#5c6370`, accent `#61afef`), "Mindweave Graphite" (dark, neutral greys, accent `#7aa2f7`), "Mindweave Light" (light, background `#fafafa`, panels `#f3f3f3`, borders `#e4e4e4`, text `#383a42`, accent `#4078f2`). Set radius 4px, font size 13px.
  - `pub fn init_themes(cx: &mut App)` — loads `THEMES_JSON` into `ThemeRegistry::global_mut(cx)`.
  - `pub fn theme_names(cx: &App) -> Vec<SharedString>`
  - `pub fn apply_theme(name: &str, cx: &mut App)` — finds config by name (falls back to "Mindweave One Dark"), `Theme::global_mut(cx).apply_config(&cfg)`, `cx.refresh_windows()`.
  - `pub struct ThemePicker` — modal opened by action `ToggleThemePicker` (Task 7); filterable list of `theme_names`; moving the highlight calls `apply_theme` (live preview); Enter confirms and calls `on_confirm: Rc<dyn Fn(&str, &mut App)>` passed to `ThemePicker::new(on_confirm, window, cx)` (Task 7 passes a closure that persists via `AppState::update_settings`); Escape restores the theme active when opened.
- [ ] **Step 1: Write failing test** in `themes.rs`: `bundled_themes_parse` — `serde_json::from_str::<ThemeSet>(THEMES_JSON)` succeeds and names == the three above, one of mode light.
- [ ] **Step 2:** `cargo test -p mindweave-desktop themes` → FAIL.
- [ ] **Step 3:** Write the JSON and `themes.rs` functions; then `ThemePicker` (use gpui-component's list/searchable list inside a dialog or popover — whichever 0.8.0 offers for a command-palette-like overlay).
- [ ] **Step 4:** `cargo test -p mindweave-desktop` → PASS; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** — `feat(desktop): bundled themes and theme picker`

---

### Task 7: App state, actions, menus, windows

**Files:**
- Create: `crates/mindweave-desktop/src/actions.rs`, `crates/mindweave-desktop/src/app_state.rs`, `crates/mindweave-desktop/src/menu_bar.rs`
- Modify: `crates/mindweave-desktop/src/main.rs`

**Interfaces:**
- Consumes: core settings (Task 5), `DocumentSession` (Task 3), themes (Task 6).
- Produces:
  - Actions in namespace `mindweave`: `NewFile, OpenFile, Save, CloseWindow, Quit, ToggleThemePicker, ToggleLayers, ToggleProperties, ZoomIn, ZoomOut, ZoomReset, ShowDashboard`.
  - Keymap: `cmd-n` NewFile, `cmd-o` OpenFile, `cmd-s` Save, `cmd-w` CloseWindow, `cmd-q` Quit, `cmd-k cmd-t` ToggleThemePicker, `cmd-b` ToggleLayers, `cmd-alt-b` ToggleProperties, `cmd-=` ZoomIn, `cmd--` ZoomOut, `cmd-0` ZoomReset.
  - `pub struct AppState { pub settings: AppSettings, editors: HashMap<PathBuf, WindowHandle<EditorWindow>>, dashboard: Option<WindowHandle<Dashboard>> }` as a gpui `Global`, with:
    - `pub fn init(cx: &mut App)` — loads settings, applies theme, registers actions and keymap, sets native macOS menu (`cx.set_menus`) with the same menus as the in-window bar, opens the dashboard.
    - `pub fn update_settings(cx: &mut App, f: impl FnOnce(&mut AppSettings))` — mutate then `save_settings`.
    - `pub fn new_file(cx: &mut App)` — `rfd` save dialog filtered to `mindweave`, forces the `.mindweave` extension, `DocumentSession::create`, then `open_editor`.
    - `pub fn open_file_dialog(cx: &mut App)` and `pub fn open_path(path: PathBuf, cx: &mut App)` — canonicalise; if already in `editors`, activate that window; else `DocumentSession::open` (error → notification in the dashboard window with the `FormatError` Display text, and `remove_recent` if the file is gone); then `open_editor`; `touch_recent`.
    - `fn open_editor(session: DocumentSession, cx: &mut App)` — new window sized from `settings.editor_window` (default 1280×820), title from `session.title()`.
    - `pub fn editor_closed(path: &Path, cx: &mut App)` — removes from registry; if no editors left, show dashboard (open it if closed).
    - Window sizes: when a dashboard or editor window closes, store its size in `settings.dashboard_window` / `settings.editor_window` via `update_settings`.
    - `pub fn show_dashboard(cx: &mut App)`.
  - `pub fn menu_bar(window_kind: WindowKind, cx: &App) -> impl IntoElement` with `pub enum WindowKind { Dashboard, Editor }` — compact in-window bar (File, Edit, View, Insert, Arrange, Help) using gpui-component dropdown menus that dispatch the actions above; items not implemented (Edit/Insert/Arrange contents, Help) are present but disabled; zoom and toggle items disabled in Dashboard.
  - Quit: for each dirty editor, prompt (Save / Don't Save / Cancel) via `window.prompt`; Cancel aborts quit.
- [ ] **Step 1:** Implement `actions.rs`, `app_state.rs`, `menu_bar.rs`; wire `main.rs` to call `gpui_kit::init`, `themes::init_themes`, `AppState::init`. Use placeholder `Dashboard`/`EditorWindow` structs that render their name until Tasks 8–9 replace them.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings` → clean; `cargo test --workspace` → PASS.
- [ ] **Step 3: Manual check** `cargo run -p mindweave-desktop`: dashboard window opens; Cmd+N → save dialog → new editor window opens and file exists on disk with `"version": 1`; Cmd+O on the same file focuses the existing window (no second window); closing the last editor brings back the dashboard; Cmd+K Cmd+T changes theme live and it is remembered after relaunch; native app menu shows the same items.
- [ ] **Step 4: Commit** — `feat(desktop): app state, actions, menus, window management`

---

### Task 8: Dashboard view

**Files:**
- Create: `crates/mindweave-desktop/src/dashboard.rs`

**Interfaces:**
- Consumes: `AppState::{settings, new_file, open_file_dialog, open_path}`, `menu_bar(WindowKind::Dashboard)`.
- Produces: `pub struct Dashboard` (`Render`), `pub fn format_relative(opened_at: u64, now: u64) -> String` ("Just now" < 60s, "N min ago", "N h ago", "Yesterday", "N days ago", then "D Mon YYYY").
- Layout per the mockup: menu bar top; left nav 200px (Recent selected; "Open…" button below); main area grid of tiles 180px wide: first tile "New file" (dashed 1px border, `+` icon 14px, accent text), then one tile per `recent_existing()` — placeholder thumbnail (dot pattern in a 100px box), file name (stem), muted relative time, path as tooltip. Click opens. Empty state: muted "No recent files" next to the New file tile. Status bar: left "Ready", right theme name (click → ToggleThemePicker).
- [ ] **Step 1: Write failing test** `format_relative_buckets` (0→"Just now", 120→"2 min ago", 7200→"2 h ago", 86_400+10→"Yesterday", 3*86_400→"3 days ago").
- [ ] **Step 2:** `cargo test -p mindweave-desktop dashboard` → FAIL.
- [ ] **Step 3:** Implement `format_relative` and the view.
- [ ] **Step 4:** `cargo test --workspace` PASS, clippy clean; manual: tiles show, clicking a tile opens it, a deleted file disappears from the grid.
- [ ] **Step 5: Commit** — `feat(desktop): dashboard`

---

### Task 9: Editor window shell (panels, tabs, status bar)

**Files:**
- Create: `crates/mindweave-desktop/src/editor/mod.rs`, `layers_panel.rs`, `properties_panel.rs`, `page_tabs.rs`, `status_bar.rs`

**Interfaces:**
- Consumes: `DocumentSession` (all edit ops), `AppState::{settings, update_settings, editor_closed}`, `menu_bar(WindowKind::Editor)`, `Canvas` (Task 10 — render a plain `div` placeholder until then).
- Produces: `pub struct EditorWindow { session: DocumentSession, cursor_canvas: Option<(f32,f32)>, .. }` with `pub fn session(&self) -> &DocumentSession`, `pub fn update_session(&mut self, f: impl FnOnce(&mut DocumentSession), cx: &mut Context<Self>)` (applies, `cx.notify()`, refreshes window title from `session.title()`); `pub fn set_cursor(&mut self, p: Option<(f32,f32)>, cx)`.
- Layout (heights/widths): menu bar 30px; left Layers panel 220px (resizable via gpui-component `resizable`, hidden by `ToggleLayers`, persisted to `settings.show_layers`); right Properties 240px (same, `ToggleProperties`, `show_properties`); canvas fills centre; page tabs 28px under the canvas; status bar 24px at the bottom.
- **Layers panel:** header "LAYERS" (10px, uppercase, muted, letter-spaced) with a 14px `plus` icon button → `add_layer`. Rows 26px: name, eye / eye-off icon button on the right → `toggle_layer_visibility`; hidden rows muted; active row uses `theme.list_active` background; click row → `set_active_layer`; double-click → inline input, Enter/blur → `rename_layer`, Escape cancels.
- **Page tabs:** one tab per page, active uses canvas background colour; click → `set_active_page`; double-click → inline rename → `rename_page`; trailing `plus` icon → `add_page`.
- **Properties panel:** section "CANVAS": "Background pattern" segmented control Lines / Dots / None → `set_pattern`; "Grid size" number input with "px" suffix, hidden when pattern is None, commits on Enter/blur via `text.trim().parse::<f32>().ok()` → `set_grid_size` (invalid → revert display to current value). Section "SELECTION" with muted text "Nothing selected".
- **Status bar:** left: "Page {i} of {n}", "Layer: {name}"; right: cursor "x {X} · y {Y}" (rounded ints, hidden when cursor is off-canvas), "{zoom}%" (click → ZoomReset), theme name (click → ToggleThemePicker).
- **Save/close:** `Save` action → `session.save()` (error → notification); `CloseWindow` / window close button when dirty → prompt Save / Don't Save / Cancel; on close call `AppState::editor_closed`. Window title = `session.title()`.
- [ ] **Step 1:** Implement the panels, tabs, status bar and `EditorWindow` per Interfaces; replace the Task 7 placeholder.
- [ ] **Step 2:** `cargo test --workspace` PASS; clippy clean.
- [ ] **Step 3: Manual check:** add/rename pages and layers, toggle eye, empty rename keeps the old name, switch pattern (grid size row hides for None), type `0` / `abc` in grid size → 4 / reverted; title gains " •" after content edits but not after switching pages; Cmd+S clears it; Cmd+W on dirty asks; Cmd+B / Cmd+Alt+B toggle sidebars and the state survives relaunch.
- [ ] **Step 4: Commit** — `feat(desktop): editor shell with layers, properties, pages, status bar`

---

### Task 10: Infinite canvas

**Files:**
- Create: `crates/mindweave-desktop/src/editor/canvas.rs`
- Modify: `crates/mindweave-desktop/src/editor/mod.rs`

**Interfaces:**
- Consumes: `viewport::{grid_lines, ZOOM_STEP}`, `Viewport::{pan_by, zoom_at, screen_to_canvas}`, `EditorWindow::{update_session, set_cursor}`.
- Produces: `pub fn canvas(editor: Entity<EditorWindow>, page: &Page) -> impl IntoElement` — a gpui `canvas(prepaint, paint)` element filling its bounds.
- Painting: background = theme background. `Lines` → 1px hairlines (`theme.border` at `alpha`) from `grid_lines`; `Dots` → 1.5px squares/quads at each (x,y) intersection with the same colour at `alpha`; `None` → nothing. Coordinates from `grid_lines` are relative to the element's bounds origin.
- Input: two-finger scroll (no modifier) → `pan_by(-dx, -dy)`; Cmd+scroll or pinch → `zoom_at(1.0 + dy*0.01 clamped to [0.5, 2.0], cursor)`; Space held + left drag, or middle drag → pan (cursor becomes grab hand); mouse move → `set_cursor(Some(screen_to_canvas(local)))`, mouse leave → `set_cursor(None)`. Actions `ZoomIn/ZoomOut` → `zoom_at(ZOOM_STEP or 1/ZOOM_STEP, canvas centre)`, `ZoomReset` → zoom 1.0 around centre. All via `set_viewport` (never dirty).
- [ ] **Step 1:** Implement the element and wire it into `EditorWindow` in place of the placeholder.
- [ ] **Step 2:** `cargo test --workspace` PASS; clippy clean.
- [ ] **Step 3: Manual check:** smooth pan/zoom; zoom anchors under the cursor; zooming to 10% keeps the pattern readable (no moiré/solid fill) and frame rate smooth; status bar coords and zoom % update live; each page keeps its own viewport and pattern; viewport restored after save + relaunch; title has no " •" from panning alone.
- [ ] **Step 4: Commit** — `feat(desktop): infinite canvas with adaptive pattern, pan and zoom`

---

### Task 11: Final verification

- [ ] **Step 1:** Run exactly the CI commands: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build -p mindweave-desktop --release` → all succeed; `target/release/mindweave` exists.
- [ ] **Step 2:** Walk the spec's "Success looks like" list end-to-end in the release build, and the five Review Focus items (corrupt file → error not crash; delete `settings.json` contents to `{{` → app starts with defaults).
- [ ] **Step 3:** Put a Mindweave window beside Zed; fix anything that reads louder than Zed (icon size, padding, borders, text size).
- [ ] **Step 4: Commit** — `chore: final polish for app shell`

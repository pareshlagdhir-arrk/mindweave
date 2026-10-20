# Mindweave — Shapes & Libraries (Milestone 1) — Design

Date: 2026-10-20
Status: Draft for approval
Builds on: `2026-10-20-app-shell-design.md` (shipped on `main`)

## Intent

Mindweave needs drawable content. The user wants shapes to come from **libraries** so that, over time, anyone can contribute new libraries (flowchart, cloud icons, UML, …) without touching the app's code. This milestone delivers the library system, the first two libraries, and everything needed to place, select, transform and style shapes.

Later milestones (agreed order, each designed separately):
2. **Paths** — Pen tool (straight + curves), point edit (drag / add / delete points, curve handles), lasso to select many points, converting a shape to a free path.
3. **Text** — Text tool, inline single-style text on the canvas, labels inside shapes, text-style properties.
4. **Snapping** — snap to grid, smart alignment guides.

## User decisions (verbatim intent)

- Libraries are contributed as **data first** (SVG + TOML manifest; no Rust needed); a Rust "smart shape" extension may come later.
- **Toolbar**: a thin Zed-like strip **under the title bar**, library tabs on the left, the active library's tools/shapes on the right.
- **Home** is the default library and holds **tools only, no shapes**: Pointer, Hand, point edit + lasso, Pen/Polygon, Text.
- A separate **Basic Shapes** library holds the basic shapes.
- Place a shape: **pick it in the toolbar, then drag on the canvas** (a click without drag places it at its default size).
- Selected shape → right sidebar shows: **position, size, rotation; fill & stroke; shape-specific properties; text style** (text style arrives with Milestone 3).
- Canvas editing: **move, resize, rotate; multi-select; undo/redo; copy/paste/delete**.
- Point editing turns a library shape into a **free path**; paths support **curves** (Milestone 2).
- Text: **inline, single style** per box (Milestone 3).
- Snapping: **grid + smart guides** (Milestone 4).
- User libraries are installed by **dropping a folder into `~/Library/Application Support/Mindweave/libraries/`**.

## Goals (Milestone 1)

1. **Library format and loader**
   - A library is a folder:
     ```
     <library-id>/
       library.toml     # id, name, version, author, description, icon (Lucide name or svg file), order
       shapes.toml      # list of shapes
       shapes/*.svg     # optional outlines for path shapes
     ```
   - Each shape entry in `shapes.toml` has: `id`, `name`, `icon` (Lucide name or svg file), `default_size = [w, h]`, `style` defaults (fill, stroke, stroke_width, opacity), and **one geometry source**:
     - a built-in **primitive** with parameters — `rect { corner_radius }`, `ellipse`, `line`, `arrow { head_size }`, `polygon { sides }`, `star { points, inner_ratio }`; the parameters become the shape-specific properties in the sidebar (with declared label, min, max, step, default);
     - or a static **SVG outline** (`svg = "shapes/process.svg"`): the path data (`<path d>`, plus `rect/circle/ellipse/polygon/polyline/line` converted to paths) is read once at load, normalised to a 0..1 box and scaled to the shape's bounds. SVG shapes have no shape-specific properties in this milestone.
   - Libraries load at startup: **built-in** libraries are compiled into the app (embedded files), **user** libraries come from the App Support folder. A broken library is skipped with a notification naming the library and the problem; it never prevents startup. Duplicate ids: built-in wins, user duplicate is skipped with a notification.
   - All parsing happens once at load into Rust structures; drawing never re-reads files (performance).
   - A short `docs/libraries.md` explains the format with the Basic Shapes library as the worked example (this is the contributor guide).
2. **Built-in libraries**
   - **Home**: Pointer (V), Hand (H). Pen, Point edit/lasso and Text appear in Home in their own milestones (not shown before they work).
   - **Basic Shapes**: Rectangle (R, with corner radius), Ellipse (O), Line (L), Arrow (A), Triangle (polygon 3), Diamond (polygon 4 rotated, or SVG), Star (5 points). Written in the public library format.
3. **Toolbar**
   - A 32px strip under the title bar spanning the editor width. Left: library tabs (Home first, Basic Shapes, then user libraries by `order` then name). Right of a hairline divider: the active tab's tools/shapes as 28px icon buttons (16px monochrome icons) with tooltips showing name + shortcut. The active tool is highlighted.
   - The active tool persists per window; after placing a shape the tool returns to Pointer (Figma behaviour). **User's decision:** double-clicking a tool button **locks** it (small lock dot on the button) so several shapes can be drawn in a row, until Escape or another tool is picked. Shift during a placement drag **constrains** (square/circle bounds, lines/arrows at 45° steps); Alt draws from the centre.
   - Escape returns to Pointer. Space-drag still pans with any tool (existing behaviour).
4. **Shapes in the document**
   - Shapes live in their layer's `objects` list (reserved in the current format). Each shape stores: `id` (uuid), `library` id, `shape` id, `x, y, w, h` (canvas units, top-left, non-negative), `rotation` (degrees), `flip_x`, `flip_y`, `style` (fill, stroke, stroke_width, opacity), `params` (shape-specific values), `name` (optional), `locked` (false).
   - The file keeps the library + shape id, not the geometry. If a document references a library that is not installed, the shape is still kept and drawn as a dashed placeholder box with the shape name; saving preserves it unchanged.
   - Format version bumps to **2**; version-1 files load unchanged (no objects). A version-2 file opened by an older build is already rejected with a clear message (existing behaviour).
   - Hidden layers' shapes are not drawn and not selectable. New shapes go on the active layer. Z-order = order within the layer list (later = on top); layers stack as in the Layers panel (top row = front).
5. **Selection and transforms (Pointer tool)**
   - **User's decision — active layer only:** click, marquee, Cmd+A and keyboard nudges act only on shapes of the **active layer** (visible, unlocked). Shapes on other layers are drawn but cannot be selected; to edit them, make their layer active in the Layers panel. Switching the active layer (or page) clears the selection. Paste and Duplicate put shapes on the active layer.
   - Click selects the topmost active-layer shape under the cursor (hit-testing the actual outline with a few px tolerance for lines; filled interior counts for closed shapes). Shift-click toggles. Drag on empty canvas draws a marquee; shapes fully inside are selected. Escape / click empty clears.
   - Selection shows a 1px accent bounding box with 8 square resize handles and a rotate handle above the top edge (Zed restraint: small, crisp).
   - Drag a selected shape to move (all selected move together). Handles resize (Shift keeps aspect ratio, Alt resizes from centre). Rotate handle rotates around the selection centre (Shift snaps to 15°). Multiple selection resizes/rotates as a group.
   - Arrow keys nudge 1 canvas unit, Shift+arrow 10.
   - Locked shapes can't be selected on canvas (lock UI arrives later; the field exists now).
6. **Properties sidebar**
   - Nothing selected: today's Canvas properties.
   - One shape selected: sections **Shape** (library · shape name), **Transform** (X, Y, W, H, Rotation), **Style** (Fill colour + none, Stroke colour + none, Stroke width, Opacity %), **Shape properties** (the primitive's declared parameters, e.g. Corner radius, Sides, Points, Inner ratio, Head size).
   - Several selected: Transform shows the group's bounds (editing moves/scales the group); Style fields show the common value or "Mixed", and editing sets all. Shape properties show only when all selected shapes share the same shape type.
   - Reuses the existing number-field and colour-picker components and commit-on-Enter/blur behaviour; `commit_pending_edits` also commits these fields.
7. **Edit operations**
   - **Undo/redo** (Cmd+Z / Shift+Cmd+Z): every document change made through the editor (shape create/delete/transform/style/params, and the existing page/layer/canvas edits) is undoable, per window, with a bounded history (e.g. 200 steps). A drag is one step. View changes (pan/zoom, active page/layer) are not in history. The document is dirty iff its state differs from the last save (undoing back to the saved state makes it clean).
   - **Delete / Backspace** deletes selection. **Cmd+D** duplicates (offset +10,+10). **Cmd+C / Cmd+X / Cmd+V** copy/cut/paste via the system clipboard as JSON with a Mindweave MIME/marker; paste places at the same position if visible, otherwise centred in view; pasting into another document/window works. **Cmd+A** selects all on the active layer.
   - The Edit menu items (Undo, Redo, Cut, Copy, Paste, Select All) become enabled; add Duplicate and Delete.

## Non-goals (this milestone)

- Pen, point edit, lasso, curves editing, convert-to-path (Milestone 2).
- Text tool, labels, text style (Milestone 3).
- Snapping and guides (Milestone 4).
- Connectors/smart arrows, grouping, alignment/distribution commands, Arrange menu actions, lock UI, layers panel listing individual shapes.
- Rust or scripted "smart shape" libraries; library gallery/marketplace; installing from zip.
- Gradients, shadows, blend modes, images.

## Anti-goals

- Libraries that can execute code or reach the network.
- A file format that embeds library geometry so documents bloat or drift from libraries (store references).
- Any frame-rate drop while panning/zooming a page with ~2,000 basic shapes.
- A noisy, icon-heavy toolbar that breaks the Zed look.

## Constraints

- Rust + gpui-kit `=0.8.0` (gpui-component 0.8.0); macOS first; CI macOS only.
- Keep `mindweave-core` gpui-free: library parsing, geometry, hit-testing, transforms, undo history and clipboard serialisation live in core and are unit-tested; the desktop crate draws and wires input.
- Drawing uses gpui `PathBuilder` (fill + stroke, curves, dashes). Geometry for each shape is built from its primitive/normalised path; cache tessellated geometry per shape and invalidate on change if needed for the 2,000-shape target.
- Library files are untrusted input: size limits (e.g. 1 MB per file, 500 shapes per library), no external references, SVG parsing limited to geometry (no scripts, images, styles); parse errors are reported, never panic.

## Decisions made by Claude (reviewable)

- Two geometry sources (primitives with parameters, static SVG outlines) instead of a template/expression language — fast, safe, and covers the Basic Shapes; expressions can be added later.
- Tool returns to Pointer after placing a shape (double-click lock is the user's decision above).
- **Flips and backwards lines:** each shape stores `flip_x` and `flip_y` booleans (default false) in addition to `x, y, w, h` (always non-negative) and `rotation`. Dragging a line/arrow from bottom-right to top-left, or dragging a resize handle past the opposite edge, toggles the flip flags instead of producing negative sizes. Geometry is built in the unit box, flipped, scaled, then rotated about the box centre. Milestone 2's convert-to-path bakes flips and rotation into the path points.
- **Group resize with rotated shapes:** Figma-style — each shape keeps its own rotation; only its box (position and size) is scaled relative to the group bounds, so a non-uniform stretch of rotated shapes is approximate (no skew). No per-shape affine matrix in the format.
- **Placeholders and unknown data:** a shape whose library or shape id isn't installed is drawn as a dashed placeholder box with its name and is fully editable as a box (select, move, resize, rotate, restyle, delete, copy/paste); its `params` are kept untouched. For installed shapes, unknown params in the file are preserved as opaque values; params the library newly declares take the library default. Unknown fields are preserved on save where practical (serde flatten into an `extra` map).
- **Shortcuts:** library manifests cannot declare shortcuts in this milestone. Built-in shortcuts are part of the app keymap: V Pointer, H Hand, R Rectangle, O Ellipse, L Line, A Arrow. A shape shortcut works whichever tab is active and switches the toolbar to that shape's library.
- **Library format compatibility:** `library.toml` must declare `format = 1`. A library with a higher `format` is skipped with a notification ("needs a newer Mindweave"). Unknown TOML fields are ignored (logged once). An unknown primitive skips only that shape (logged), the rest of the library loads. These rules are documented in `docs/libraries.md`.
- File format version 2; missing-library shapes kept as placeholders.
- Undo covers all document edits, including the existing page/layer/canvas ones, and drives the dirty flag.
- Clipboard uses the system clipboard with JSON.
- Pen / point edit / Text buttons are not shown until their milestone ships.

## Success looks like

- Basic Shapes tab → pick Rectangle → drag on canvas → a rectangle appears and the Pointer is active again; sidebar shows its transform, style and Corner radius; changing corner radius updates it live.
- Shapes on a non-active layer cannot be selected; making that layer active lets you select them.
- Double-click the Rectangle tool, draw three rectangles in a row (Shift draws squares), Escape returns to Pointer.
- Select several shapes with a marquee, rotate and resize them as a group, Cmd+Z restores each step, saving and reopening restores everything.
- Copy shapes from one document window and paste into another.
- Dropping a valid library folder into App Support and relaunching shows a new tab; a broken one shows a notification and the app still starts.
- `docs/libraries.md` is enough for a contributor to write a new library without reading Rust.
- CI green (fmt, clippy -D warnings, tests, release build).

## Left to the builder

- Exact TOML schema field names beyond those listed, SVG subset parser choice (e.g. `usvg`/`svgtypes` or a small hand-written path parser — pin exact versions), geometry/hit-test algorithms, undo implementation (command vs snapshot), icon choices, selection handle visuals within the Zed restraint, module layout.

# Mindweave — Library format 2 and the UML library — Design

Date: 2026-10-23
Status: Draft for approval
Builds on: the shape library format 1 (`docs/libraries.md`), lines and connectors, groups, built-in Flowchart and Wireframe libraries.

## Intent

The user needs UML 2.0 class, activity and sequence diagrams. Three things they need cannot be expressed today: class boxes with three editable compartments, hollow arrowheads (inheritance, realisation, aggregation), elements filled in the theme's ink colour (initial/final nodes, fork/join bars), and lifelines (a box with a dashed line under it). The user chose to extend the library format first, then build a correct UML library on it.

## User decisions

- Extend the format with (1) an **ink colour**, (2) **hollow triangle and hollow diamond arrowheads**, (3) **multi-part library items (templates)**, then build the UML library.
- Libraries stay data files: everything UML needs is expressed in the library format, not hard-coded.

## Goals

### 1. Ink colour
- A new style colour keyword **`"ink"`**: the theme's text colour (white on dark themes, dark on light), for fill or stroke. `"auto"` keeps its meaning (fill: canvas colour; stroke: text colour).
- Valid in library `style` tables and in documents (`Style.fill` / `Style.stroke`). Properties shows it as **Ink** in the colour control, beside Auto; the colour palette offers it.

### 2. Hollow arrowheads
- `ArrowHead` gains **`HollowTriangle`** and **`HollowDiamond`**: outlined in the line's colour, filled with the canvas colour, the line trimmed to the head's base like the filled heads. Properties lists them with the other heads (names "Hollow triangle", "Hollow diamond").

### 3. Templates (multi-part library items)
- `shapes.toml` may hold **`[[template]]`** tables alongside `[[shape]]`, appearing in the ribbon in file order among the shapes:
  - `id`, `name`, `icon` (required: a Lucide name or library SVG), `default_size = [w, h]`.
  - One or more **`[[template.part]]`**, each:
    - `frame = [x, y, w, h]` in the template's own units (0..default_size),
    - **either** a shape — `primitive` or `svg` (as `[[shape]]`), optional `param` values as `params = { corner_radius = 4 }` — **or** a line — `line = { from = [x, y], to = [x, y], start_head = "none", end_head = "hollow-triangle" }`,
    - optional `style` (as `[[shape]]`, plus `dash = "solid" | "dashed" | "dotted"`),
    - optional `label = { text = "…", align = "left" | "center" | "right", valign = "top" | "middle" | "bottom", bold = bool, italic = bool, underline = bool, size = number }`.
- **Placing** a template (click, or drag to size it) adds its parts as ordinary shapes and lines, scaled from `default_size` to the placed size, **grouped together** (one group, so they select and move as one; ungroup to separate). Labels are ordinary editable labels. One undo step. Template parts that are lines are normal lines (their ends can later be attached).
- A part shape belongs to the library with a hidden internal shape id derived from the template (`<template-id>.<part-index>`), so documents keep drawing it if the library is present, and fall back to the placeholder box otherwise (existing behaviour). *(Builder may choose another stable scheme.)*
- **Library format 2**: `format = 2` is required for libraries that use `"ink"`, hollow heads or templates; format 1 libraries load unchanged. The loader validates templates like shapes (limits: at most 20 parts per template, the same 500-item limit counting templates).
- `docs/libraries.md` documents format 2.

### 4. Document format
- **File format version 7** (new arrowheads and the `"ink"` colour must not be silently dropped by older builds); clipboard payload version raised likewise.

### 5. The UML library (built-in, format 2)
One library, **UML**, tab icon `component`, after Wireframe. Items in this order:

- **Class diagrams:** Class (template: name compartment bold centred, attributes and operations compartments left/top aligned, e.g. `+ attribute: Type`, `+ operation()`), Interface (template: «interface» + name, operations), Abstract Class (as Class, name italic), Enumeration (template: «enumeration» + name, literals), Object (template: underlined `object : Class`, attributes), Package (folder-tab outline), Note (dog-eared outline), Component (rect with component glyph), Actor (stick figure, strokes).
- **Relationships (line templates, one line part each):** Association (plain), Directed Association (open arrow), Generalization (hollow triangle), Realization (dashed, hollow triangle), Dependency (dashed, open arrow), Aggregation (hollow diamond at start), Composition (filled diamond at start).
- **Activity diagrams:** Action (rounded rect), Initial Node (ink-filled circle), Activity Final (ring + ink-filled dot), Flow Final (circle with an X), Decision / Merge (diamond), Fork / Join (ink-filled bar), Send Signal (pentagon arrow), Accept Event (notched rect), Object Node (rect), Swimlane (template: header + body), Control Flow (line template, open arrow).
- **Sequence diagrams:** Lifeline (template: head box + dashed vertical line), Actor Lifeline (template: actor + dashed line), Activation (narrow bar), Combined Fragment (template: frame + operator tag "alt", left/top), Destruction (X, strokes), Synchronous Message (line, filled triangle), Asynchronous Message (line, open arrow), Reply Message (dashed line, open arrow).

## Non-goals
- Smart UML behaviour (auto-sized compartments, compartments growing with text, stereotypes as data, code generation).
- Templates that bind to each other or connect automatically.
- User-saved components (a later feature can reuse templates).

## Anti-goals
- Format 1 libraries (including user libraries) failing to load or changing appearance.
- An "ink" shape invisible on either theme.
- Older builds silently dropping hollow heads or ink colours.

## Constraints
- Core in `mindweave-core` with tests (loader, scaling of parts, new heads' geometry); desktop changes minimal (placement of templates, colour control, head list).
- The UML library is pure data in `crates/mindweave-core/libraries/uml/`.

## Left to the builder
- Internal ids of template part shapes; exact head geometry; how the ribbon icon of a template is drawn when `icon` is an SVG; colour palette placement of Ink.

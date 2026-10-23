# Library Format 2 and UML Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `"ink"` colour, hollow arrowheads and multi-part templates (library format 2), then a built-in UML library using them.

**Architecture:** Core gains the colour keyword, two `ArrowHead` variants, format 7, and template parsing plus a pure `instantiate_template` that turns a template into grouped shapes and lines. The desktop resolves ink when painting, offers Ink and the new heads in Properties, and places templates from the ribbon. The UML library is data.

**Tech Stack:** Rust, gpui-kit 0.8.

**Spec:** `docs/superpowers/specs/2026-10-23-library-format-2-and-uml-design.md` (authority; this plan fixes names and tests).

## Global Constraints

- Colour keyword `INK_COLOR = "ink"` (core `color.rs`), accepted wherever `"auto"` is; painting resolves it to the theme foreground for fill and stroke.
- `ArrowHead::HollowTriangle`, `ArrowHead::HollowDiamond`; serde names `"hollow-triangle"`, `"hollow-diamond"` (match the existing serde naming of `ArrowHead`); display names "Hollow triangle", "Hollow diamond".
- `FORMAT_VERSION = 7`; `CLIPBOARD_VERSION` and `TEXT_PAYLOAD_VERSION` each +1; versions 1–6 load.
- `LIBRARY_FORMAT = 2`; format 1 libraries load unchanged; format-2-only features in a `format = 1` library give a warning and are skipped (that item only).
- Templates: at most 20 parts each; templates count toward the 500-item limit.
- Every edit one undo step; doc comment on every item; core logic unit-tested; match surrounding code.

## Review Focus

1. A format 1 user library loads exactly as before (Task 2 test with an existing fixture).
2. Ink fill/stroke is visible on both a dark and a light theme (Task 1 test of the resolver for both).
3. Placing a template at a non-default size scales every part's frame and line ends proportionally (Task 2 test).
4. Undo after placing a template removes all its parts in one step (Task 3 test).
5. A document using a template part whose library is missing still opens and shows placeholders (Task 2 test).

---

### Task 1: Ink colour, hollow arrowheads, format 7

**Files:** core `color.rs`, `line.rs`, `format.rs`, `clipboard.rs`; desktop `editor/shape_paint.rs` (colour resolution), `editor/line_properties.rs` (head list), the colour control/palette (`color_palette.rs`, Properties colour rows), `editor/text_edit.rs` (payload version).

**Produces:** `INK_COLOR`, `is_ink_color(Option<&str>) -> bool`; `normalize_style_color` accepts `"ink"`; desktop `style_color` resolves ink to `theme.foreground`; Properties colour control shows "Ink" and the palette offers it beside Auto; `ArrowHead::ALL` includes the hollow heads; `head_paths` draws them outlined (fill = canvas/background colour, stroke = line colour) with the same inset as their filled forms; format 7.

- [ ] Tests first: `ink_is_a_valid_style_colour`; `ink_resolves_to_the_theme_foreground` (dark and light themes — or the pure resolver given both foregrounds); `hollow_heads_trim_like_filled_ones`; `hollow_heads_round_trip`; `v6_files_load_and_save_as_v7`. Then implement; `cargo test --workspace`, clippy; commit `feat: ink colour, hollow arrowheads, format 7`.

### Task 2: Library format 2 templates (core)

**Files:** core `library.rs` (+ a new `template.rs` if it keeps `library.rs` smaller), `lib.rs`; `docs/libraries.md`.

**Produces:**
- `LIBRARY_FORMAT = 2`; `Library.items: Vec<LibraryItem>` in ribbon order, `enum LibraryItem { Shape(ShapeDef-id or index), Template(TemplateDef) }` *(or an equivalent that keeps `Library.shapes` working for existing callers)*.
- `TemplateDef { id, name, icon: Icon, default_size: (f32, f32), parts: Vec<TemplatePart> }`; `TemplatePart { frame: [f32; 4], kind: PartKind, style: Option<StyleDefaults + dash>, params, label: Option<PartLabel> }`; `PartKind::Shape(ShapeSource)` / `PartKind::Line { from, to, start_head, end_head }`; `PartLabel { text, align, valign, bold, italic, underline, size }`.
- Part shapes get hidden `ShapeDef`s registered in the library (not in the ribbon) so documents draw them and `Library::shape(id)` finds them; ids stable (`<template>.<n>`).
- `pub fn instantiate_template(lib: &Library, template: &TemplateDef, frame: Frame) -> Vec<Shape>` — shapes and lines scaled from `default_size` to `frame`, labels set, all in one new group.
- `docs/libraries.md`: a "Format 2" section (ink, hollow heads, templates, with a complete example).

- [ ] Tests first: `format_1_library_loads_unchanged` (Focus 1); `template_parses_with_shape_and_line_parts`; `template_part_limits_warn`; `format_2_features_in_format_1_warn_and_skip`; `instantiate_scales_frames_and_line_ends` (Focus 3, placed at 2× size); `instantiated_parts_share_one_group`; `missing_template_library_shows_placeholders` (Focus 5). Then implement; tests, clippy; commit `feat(core): library format 2 templates`.

### Task 3: Placing templates (desktop)

**Files:** `editor/tool.rs`, `editor/toolbar.rs`, `editor/shape_gallery.rs`, `editor/interaction.rs`, `libraries.rs` (icon notices include templates).

**Produces:** `Tool::Template { library, template }`; library tabs and the More gallery list templates among shapes in file order (icon, name tooltip); click places at `default_size` centred on the click, drag sizes it (Shift keeps proportion); placement uses `instantiate_template`, adds the parts with membership (artboards) in one undo step, selects the new group; the tool returns to Pointer unless locked.

- [ ] Tests first: `template_tool_round_trips_tab`; `placing_a_template_is_one_undo_step` (Focus 4, session level); `templates_appear_in_ribbon_order`. Then implement; tests, clippy; visual check; commit `feat: place library templates`.

### Task 4: The UML library

**Files:** `crates/mindweave-core/libraries/uml/` (library.toml, shapes.toml, shapes/*.svg), `library.rs` (embed it after Wireframe; update built-in id lists in tests).

**Produces:** the spec's UML items in its order (class, relationships, activity, sequence sections), `format = 2`, tab icon `component`.

- [ ] Tests: `uml_loads_without_warnings`; `uml_has_class_activity_and_sequence_items` (spot-check Class has 3 parts with the name part bold; Generalization's line ends in `HollowTriangle`; Lifeline's line part is dashed; Initial Node fill is `"ink"`). Visual check of a sheet of every item on dark and light themes; commit `feat: built-in UML library`.

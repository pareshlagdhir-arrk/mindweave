# Artboards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Figma-style artboards on the canvas: named frames that hold, move and clip their member shapes, placed from an Artboard ribbon tab of presets or drawn with a Custom Artboard tool.

**Architecture:** Core gains the artboard shape kind, membership (`Shape.artboard`), the membership rule, member-aware move/delete/copy, presets and format v6. The desktop paints artboards beneath members with a rectangular clip and a fixed-size name label, adds artboard hit-testing and gestures, and an Artboard ribbon tab plus a Properties section.

**Tech Stack:** Rust, gpui-kit 0.8.

**Spec:** `docs/superpowers/specs/2026-10-23-artboards-design.md` (the authority; read it — this plan only fixes names and test expectations).

## Global Constraints

- Artboard = `library: TEXT_LIBRARY`, `shape: ARTBOARD_SHAPE = "artboard"`, `artboard_props: Some(ArtboardProps { clip: true })`; fill default `#FFFFFF`, no stroke; never rotated or flipped; never a member.
- Membership: `Shape.artboard: Option<Uuid>` (serde default, skipped when `None`).
- `FORMAT_VERSION = 6`; `CLIPBOARD_VERSION` and `TEXT_PAYLOAD_VERSION` each +1; every save writes 6; versions 1–5 load.
- Name label: 11 px screen size, muted colour, accent colour when selected; not scaled by zoom.
- Every edit one undo step; membership changes ride in the step of the move/place that caused them. Creating/moving/resizing an artboard never changes other shapes' membership.
- Match surrounding code; doc comment on every item; core logic unit-tested.

## Review Focus

1. Moving a selection containing an artboard and some of its members moves each shape exactly once (Task 1 test).
2. Undo after moving an artboard restores the artboard and all members in one step (Task 1 test).
3. Deleting an artboard deletes members only — never non-members overlapping it (Task 1 test).
4. A mind map subtopic dragged out of an artboard stays a member (Task 1 test).
5. Copy/paste of an artboard keeps members attached to the *new* artboard id (Task 1 test).

---

### Task 1: Core model, membership and format v6

**Files:** `crates/mindweave-core/src/shape.rs`, new `crates/mindweave-core/src/artboard.rs` (re-exported from `lib.rs`), `session.rs`, `clipboard.rs`, `format.rs`, `text.rs` (payload version only).

**Produces:**
- `ArtboardProps { clip: bool }`, `Shape.artboard_props: Option<ArtboardProps>`, `Shape.artboard: Option<Uuid>`, `Shape::new_artboard(frame: Frame, name: &str) -> Shape`, `Shape::is_artboard(&self) -> bool`.
- `pub struct ArtboardPreset { pub name: &'static str, pub size: (f32, f32), pub category: PresetCategory, pub icon: &'static str }`, `pub enum PresetCategory { Device, Desktop, Paper, Social }`, `pub const ARTBOARD_PRESETS: &[ArtboardPreset]` — the 19 presets of the spec's table, in order, with icons `smartphone`/`tablet`/`monitor`/`laptop`/`file-text`/`presentation`/`image`/`square-play`/`rectangle-horizontal`.
- `pub fn membership(shapes: &[Shape], moved: &[Uuid]) -> Vec<(Uuid, Option<Uuid>)>` — the new membership of `moved` (skipping artboards) by the spec's rule: bounding-box centre (line: midpoint of ends) inside the topmost artboard; groups decided as a unit by the whole group's box; mind map shapes take their central topic's membership.
- `DocumentSession::assign_membership(&mut self, ids: &[Uuid])` — applies `membership` within the current edit (no separate undo step).
- Member-aware session operations: moving a set of shapes (find the existing translate/move path) expands it with members of any selected artboard, de-duplicated; `remove_shapes`/`delete_with_branches` remove members of removed artboards; `clipboard::duplicate` and copy/paste remap `artboard` ids to the new artboards (members of a non-copied artboard keep their old id only if that artboard still exists, else `None`).
- `artboard_placement(shapes: &[Shape], size: (f32, f32), view_center: Pt) -> Pt` — top-left for a new preset: right of the rightmost artboard (gap 80, tops aligned), else centred on `view_center`.
- Format v6, clipboard/text payload versions +1.

- [ ] **Step 1: Tests first** in `artboard.rs`/`session.rs`/`format.rs`/`clipboard.rs`: `centre_inside_joins_topmost_artboard`; `group_joins_as_a_unit`; `line_uses_midpoint`; `subtopic_follows_central_topic` (Review Focus 4); `artboard_move_carries_members_once` (Focus 1); `undo_restores_artboard_and_members` (Focus 2); `deleting_artboard_spares_non_members` (Focus 3); `pasted_artboard_keeps_its_members` (Focus 5); `creating_artboard_captures_nothing`; `placement_right_of_last_or_centred`; `presets_table_has_19_in_spec_order` (spot-check iPhone 16 = 393 × 852, A4 = 595 × 842); `v5_files_load_and_save_as_v6`; `artboard_round_trips`.
- [ ] **Step 2: Run** `cargo test -p mindweave-core` — FAIL. **Step 3: Implement.** **Step 4:** `cargo test --workspace` and clippy — PASS.
- [ ] **Step 5: Commit** `feat(core): artboards, membership, format v6`

---

### Task 2: Painting artboards, clipping and names

**Files:** `crates/mindweave-desktop/src/editor/shape_paint.rs`, `text_paint.rs`, `canvas.rs` as needed.

**Consumes:** Task 1 types.

**Produces:** per layer, artboards paint first (fill/stroke/radius as a rectangle), then the other objects; members of a `clip` artboard (shape fill/stroke, pictures, equations, badges and their text) paint inside `with_content_mask` of the artboard's screen rect intersected with the canvas bounds; the name label paints above the top-left corner (11 px UI font, muted; accent when selected), after all shapes so it is never covered. Text of members in the separate text pass is clipped the same way. A pure helper `pub(super) fn name_label_rect(frame_on_screen: Rect, text_width: f32) -> Rect` (label sits 4 px above the frame) is shared with Task 3's hit-testing.

- [ ] **Step 1: Test** `name_label_rect` (above, left-aligned, fixed height 16). **Step 2–4:** implement; `cargo test --workspace`, clippy.
- [ ] **Step 5: Visual check** with a document holding an artboard, a member overflowing its edge (clipped), and a non-member overlapping it (not clipped). Commit `feat: paint artboards with clipping and names`.

---

### Task 3: Artboard interaction

**Files:** `editor/interaction.rs`, `editor/canvas.rs`, `editor/tool.rs`, `editor/text_edit.rs` or a small `editor/artboard_edit.rs`, `actions.rs`.

**Consumes:** Tasks 1–2 (`assign_membership`, member-aware move, `name_label_rect`).

**Produces:**
- Hit-testing: a click on an artboard's name label or empty area (no member hit there) selects the artboard; clipped-away parts of members are not hittable; plain drag on the empty area moves the artboard with its members; **⌘-drag** on the empty area starts a marquee.
- `assign_membership` called after place, move/drop, resize, paste and duplicate of non-artboard shapes (same undo step).
- No rotation handle for artboards; resize keeps members in place.
- Double-click on the name label renames in place (single-line field over the label; Enter applies, Escape cancels; one undo step).
- `Tool::Artboard` with key **A**: drag draws an artboard (Shift: square), click places 400 × 400, named "Artboard"; Tool goes back to Pointer unless locked.

- [ ] **Step 1: Tests** for the pure decisions you extract (e.g. `press_on_artboard(…) -> ArtboardPress { Select, Marquee, Member(Uuid) }` for empty-area/⌘/member cases; `a` resolves to the Artboard tool in the editor context).
- [ ] **Step 2–4:** implement; `cargo test --workspace`, clippy.
- [ ] **Step 5: Visual check** of select, move-with-members, ⌘-marquee, rename, Custom Artboard tool. Commit `feat: artboard selection, moving, renaming and tool`.

---

### Task 4: Artboard ribbon tab and Properties

**Files:** `editor/toolbar.rs`, `editor/tool.rs` (`ARTBOARD_TAB`), `assets.rs` (partial accents), `editor/properties_panel.rs` (+ a small `artboard_properties.rs`).

**Consumes:** `ARTBOARD_PRESETS`, `artboard_placement`, `Tool::Artboard`.

**Produces:**
- Tab "Artboard" after "Mind Map" (`tab_order` updated and its test extended).
- Row: Custom Artboard tool button (`frame`), divider, then one button per preset grouped by category with dividers; tooltip "iPhone 16 — 393 × 852"; presets that do not fit go behind a ⌄ dropdown menu at the end. Click places via `artboard_placement` (refused with the usual warning on a hidden layer), selects it, scrolls it into view.
- Properties for a selected artboard: Name, Preset (dropdown; changes size, keeps top-left), W/H, Fill, Stroke, Corner radius, Clip content; no Rotation.
- Accents: partial (outline stays readable), as in `assets.rs`.

- [ ] **Step 1: Tests:** `tab_order` puts Artboard after Mind Map; a pure `preset_tooltip(&ArtboardPreset) -> String`. **Step 2–4:** implement; `cargo test --workspace`, clippy.
- [ ] **Step 5: Visual check** of the tab and Properties; commit `feat: artboard ribbon tab and properties`.

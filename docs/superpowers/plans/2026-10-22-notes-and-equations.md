# Topic Notes and Equations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mind map topics can carry a rich text note (opened from an XMind-style indicator into a floating, editable card) and one LaTeX equation (maths and mhchem chemistry) drawn inside the topic.

**Architecture:** The core model (`mindweave-core`, gpui-free) gains the note and equation fields on `TopicProps`, list items and links in `RichText`, the topic layout parts for both, and file format version 5. The desktop crate wraps RaTeX in one rendering module with a cache, paints equations and the note badge on topics, adds an equation card, and generalises the on-canvas text editor so the same engine edits a note inside a floating card.

**Tech Stack:** Rust, gpui-kit 0.8 (gpui + gpui-component), RaTeX 0.1.15 (`ratex-parser`, `ratex-layout`, `ratex-types`, `ratex-svg` with `standalone` + `embed-fonts`), resvg through gpui's SVG renderer.

**Spec:** `docs/superpowers/specs/2026-10-22-notes-and-equations-design.md`

## Global Constraints

- RaTeX crates pinned exactly: `ratex-parser = "=0.1.15"`, `ratex-layout = "=0.1.15"`, `ratex-types = "=0.1.15"`, `ratex-svg = { version = "=0.1.15", features = ["standalone", "embed-fonts"] }`, declared in `[workspace.dependencies]` and used with `.workspace = true` (the repo's convention).
- SVG output must use `SvgOptions { embed_glyphs: true, .. }`; the default `<text>` output does not render in resvg.
- `FORMAT_VERSION` becomes **5**; versions 1–4 still load unchanged; anything above 5 is refused with the existing `FormatError::UnsupportedVersion`.
- Core stays gpui-free; every new core function is unit-tested.
- Shortcuts: **⌘⇧N** (Ctrl+Shift+N) opens the note card; **⌘K** link; **⌘-click** (Ctrl+click) opens a link in a note; Escape closes cards.
- Equation rendering never runs per frame for an unchanged equation (cache by source, colour, size).
- Ship the KaTeX fonts' SIL OFL 1.1 notice and RaTeX's MIT notice in `THIRD_PARTY_NOTICES.md` at the repo root.
- Match surrounding code: doc comment on every item, small pure functions with tests, `update_session` for document edits.

## Review Focus

1. **Shortcut clashes while editing a note:** ⌘B/⌘I/⌘U in the note card must style the note, not toggle the left sidebar (⌘B is bound to `ToggleLeftSidebar` outside text editing). Test in Task 7.
2. **Topic deleted or undone away while its card is open:** the note card and equation card must close, not panic or write into a missing topic. Test in Task 9 (and Task 6 for the equation card).
3. **Huge or pathological LaTeX** (very long input, `\\` repeated hundreds of times, unknown macros): rendering returns an error or a size clamped to `TOPIC_PICTURE_RANGE.1` (800) on its longest side, never hangs the UI or makes a 10,000-unit topic. Test in Task 4.
4. **Theme switch with equations on screen:** the equation must re-render in the new text colour (colour is part of the cache key) — test in Task 4.
5. **Pasting a very large note** must respect the existing `MAX_PASTE_CHARS` cap, and an emptied note must become `None` (indicator disappears). Test in Task 7.

---

### Task 1: Topic note and equation fields, file format version 5

**Files:**
- Modify: `crates/mindweave-core/src/mindmap.rs` (`TopicProps`, `central()`)
- Modify: `crates/mindweave-core/src/format.rs` (version, docs, tests)
- Modify: `crates/mindweave-core/src/session.rs` (two setters)

**Interfaces:**
- Produces:
  - `TopicProps.note: Option<RichText>` — `#[serde(default, skip_serializing_if = "Option::is_none")]`
  - `TopicProps.equation: Option<Equation>` — same serde attributes
  - `pub struct Equation { pub latex: String, pub size: (f32, f32) }` in `mindmap.rs`, `Serialize/Deserialize/Clone/Debug/PartialEq`, re-exported from `lib.rs`. `size` is the rendered size in canvas units, kept current by the desktop fit pass (Task 5); `(0., 0.)` until measured.
  - `DocumentSession::set_topic_note(&mut self, id: Uuid, note: Option<RichText>)` — stores `None` when the note's plain text is empty (whitespace counts as content only if not all whitespace — use `text.plain().trim().is_empty()`).
  - `DocumentSession::set_topic_equation(&mut self, id: Uuid, latex: Option<String>)` — trims; empty → `None`; a changed source resets `size` to `(0., 0.)`.
  - `DocumentSession::set_topic_equation_size(&mut self, id: Uuid, size: (f32, f32))` — a layout update like `set_topic_size` (no undo step, bumps the layout generation).
  - `FORMAT_VERSION = 5`; every save writes 5.

- [ ] **Step 1: Write failing tests** in `format.rs` tests:
  - `notes_and_equations_round_trip`: a central topic with `note = Some(RichText::plain("Remember"))` and `equation = Some(Equation { latex: r"\ce{H2O}".into(), size: (40., 18.) })` saves and loads equal; the saved JSON contains `"version": 5`.
  - `version_4_files_load_without_notes`: an existing v4 fixture loads with `note == None` and `equation == None`, and re-saves as version 5.
  - `load_rejects_newer_version`: change the fixture to `"version":6`.
  - Update the existing assertions that expect `4` to expect `5`.
- In `session.rs` tests: `emptied_note_becomes_none` (set `RichText::plain("  ")` → `note == None`), `equation_source_change_resets_size`, `clearing_equation_removes_it`.
- [ ] **Step 2: Run** `cargo test -p mindweave-core` — expect the new tests to FAIL (missing fields/functions).
- [ ] **Step 3: Implement** the fields, `Equation`, the three session functions (the first two through the session's normal undoable edit path, as `update_shapes` does; the size setter like `set_topic_size`), bump `FORMAT_VERSION` to 5 and extend the module doc: "Version 5 adds topic notes, equations, list items and links."
- [ ] **Step 4: Run** `cargo test -p mindweave-core` — PASS.
- [ ] **Step 5: Commit** `feat(core): topic notes and equations, format version 5`

---

### Task 2: List items and links in rich text

**Files:**
- Modify: `crates/mindweave-core/src/text.rs`

**Interfaces:**
- Produces:
  - `pub enum ListKind { Bullet, Numbered }` (`serde(rename_all = "lowercase")`)
  - `pub struct ListItem { pub kind: ListKind, pub level: u8 }` — `level` 0..=`MAX_LIST_LEVEL` where `pub const MAX_LIST_LEVEL: u8 = 2` (three levels).
  - `Paragraph.list: Option<ListItem>` — `#[serde(default, skip_serializing_if = "Option::is_none")]`
  - `RunStyle.link: Option<String>` — same serde attributes; `sanitize` drops empty/whitespace links. Runs differing only in `link` are not merged by `normalize`.
  - `StyleChange::Link(Option<String>)` handled by the existing apply-style path; `StyleSummary.link: Mixed<Option<String>>`.
  - `RichText::set_list(&mut self, range: Range<usize>, kind: Option<ListKind>)` — sets (or clears) the list kind of every paragraph the range touches; keeps an existing level.
  - `RichText::shift_list_level(&mut self, range: Range<usize>, delta: i8)` — clamps to `0..=MAX_LIST_LEVEL`; no-op on non-list paragraphs.
  - `RichText::list_number(&self, paragraph: usize) -> Option<u32>` — 1-based number of a `Numbered` item: counts back over consecutive list paragraphs, counting `Numbered` ones at the same level and skipping deeper ones; a shallower item or a non-list paragraph restarts.
  - `RichText::link_at(&self, offset: usize) -> Option<&str>`
  - Splitting a paragraph (Enter) copies `list` to the new paragraph.

- [ ] **Step 1: Write failing tests:**
  - `list_numbers_count_items_at_their_level`: paragraphs [N0, N0, N1, N1, N0, plain, N0] → numbers [1, 2, 1, 2, 3, None, 1].
  - `set_list_and_shift_level`: set Bullet over two paragraphs; shift +1 twice and +1 again → level stays 2; shift −5 → 0; `set_list(.., None)` clears.
  - `enter_continues_the_list`: splitting a bulleted paragraph gives two bulleted paragraphs at the same level.
  - `links_are_run_styles`: apply `StyleChange::Link(Some("https://a.b".into()))` to "b" of "abc" → `link_at(1) == Some("https://a.b")`, `link_at(0) == None`; summary of 0..3 is `Mixed`.
  - `old_text_without_lists_or_links_deserialises`: JSON from a v4 label loads with `list == None`, `link == None`.
- [ ] **Step 2: Run** `cargo test -p mindweave-core text::` — FAIL.
- [ ] **Step 3: Implement** the types and functions above.
- [ ] **Step 4: Run** `cargo test -p mindweave-core` — PASS (existing text tests unchanged).
- [ ] **Step 5: Commit** `feat(core): list items and links in rich text`

---

### Task 3: Topic layout parts for the note badge and the equation

**Files:**
- Modify: `crates/mindweave-core/src/mindmap.rs` (`TopicParts`, `parts`, `size_for_text`, tests)

**Interfaces:**
- Consumes: `TopicProps.note`, `TopicProps.equation` (Task 1).
- Produces:
  - `TopicParts.note: Option<Rect>` and `TopicParts.equation: Option<Rect>` (topic-local, like `badge`).
  - Badges sit in one row at the right of the text row, link rightmost: `badge` (link) at `x = w − TOPIC_PADDING − TOPIC_BADGE`; `note` at `x = w − TOPIC_PADDING − TOPIC_BADGE` when there is no link, else `TOPIC_BADGE + TOPIC_GAP` further left. Both vertically centred on the text row. The text's right margin reserves `n·TOPIC_BADGE + (n−1)·TOPIC_GAP + TOPIC_GAP + TOPIC_PADDING − text_padding.0` for `n` badges.
  - The equation is below the text row: centred horizontally, `y = text_row_bottom − text_padding.1 + TOPIC_GAP`, size `equation.size`; the topic height adds `equation.h + TOPIC_GAP + TOPIC_PADDING − text_padding.1` (mirrors how the picture adds height above). An equation with size `(0., 0.)` (not measured yet) adds nothing.
  - Width: `max(text row width, equation.w + 2·TOPIC_PADDING, picture width + 2·TOPIC_PADDING, 60)`.

- [ ] **Step 1: Write failing tests** next to `topics_fit_their_text_picture_and_link_snugly`:
  - `note_badge_sits_left_of_the_link_badge`: child topic with link and note, `size_for_text((100., 20.))` → width `100 + 32 + 2·14 + 4 + 4 + 6 − 16 = 158`; `parts.note.x == parts.badge.x − 14 − 4`; with note only, `parts.note.x == w − 6 − 14`.
  - `equation_sits_below_the_text_and_widens_the_topic`: child topic, equation size `(200., 40.)`, text `(100., 20.)` → `w = 212`, `h = 38 + 40 + 4 + 6 − 9 = 79`; `parts.equation == Rect { x: 6., y: 38. − 9. + 4., w: 200., h: 40. }`.
  - `unmeasured_equation_adds_nothing`: size `(0., 0.)` → same size as without an equation.
- [ ] **Step 2: Run** `cargo test -p mindweave-core mindmap::` — FAIL.
- [ ] **Step 3: Implement** in `parts` and `size_for_text`.
- [ ] **Step 4: Run** `cargo test -p mindweave-core` — PASS.
- [ ] **Step 5: Commit** `feat(core): topic layout for note badge and equation`

---

### Task 4: Equation rendering module and cache

**Files:**
- Create: `crates/mindweave-desktop/src/equation.rs` (register `mod equation;` in `main.rs`)
- Modify: `Cargo.toml` (workspace deps), `crates/mindweave-desktop/Cargo.toml`
- Create: `THIRD_PARTY_NOTICES.md` (KaTeX fonts OFL 1.1 text and copyright; RaTeX MIT)

**Interfaces:**
- Produces:
  - `pub struct RenderedEquation { pub svg: Arc<[u8]>, pub size: (f32, f32) }` — `size` in canvas units at zoom 1.
  - `pub fn render_equation(latex: &str, color: Hsla, font_size: f32) -> Result<RenderedEquation, String>` — `parse` → `layout` with `MathStyle::Display` and the colour → `to_display_list` → `render_to_svg` with `SvgOptions { font_size: font_size as f64, padding: 0.0, embed_glyphs: true, ..Default::default() }`. Reads `size` from the SVG's width/height (scale to keep the longest side ≤ 800 by lowering `font_size` and re-rendering once). Errors are RaTeX's parse message. Inputs longer than `MAX_EQUATION_CHARS = 4_000` are refused with "Equation is too long".
  - `pub struct EquationCache` (`Default`) with `pub fn get(&mut self, latex: &str, color: Hsla, font_size: f32) -> Option<&Result<RenderedEquation, String>>`-style access keyed by `(latex, rgba u32, font_size bits)`, plus `begin_frame`/`end_frame`/`keep` releasing entries unused for a frame, mirroring `ImageCache` (Task-5 painting rasterises `svg` through `cx.svg_renderer().render_single_frame(&svg, scale)` and caches the `RenderImage` per scale the same way `ImageCache::get_scaled` does — reuse `images::svg_scale`).
  - `pub fn equation_font_size(label: Option<&RichText>) -> f32` — the topic text's `max_font_size`, default `DEFAULT_FONT_SIZE` when no label.

- [ ] **Step 1: Write failing tests** in `equation.rs`:
  - `renders_maths_and_chemistry`: `\frac{a}{b}` and `\ce{2H2 + O2 -> 2H2O}` both `Ok`, sizes > 0, SVG contains no `<text`.
  - `bad_latex_is_an_error`: `\frac{a` → `Err`.
  - `size_follows_font_size`: font 28 is about twice the width of font 14 (ratio within 1.8..2.2).
  - `huge_input_is_refused_or_clamped`: 4,001 chars → `Err("Equation is too long")`; `x\\` ×300 → `Ok` with longest side ≤ 800.
  - `colour_is_part_of_the_cache_key`: two `get` calls with different colours give different SVG bytes; same colour twice reuses (pointer-equal `Arc`).
- [ ] **Step 2: Run** `cargo test -p mindweave-desktop equation::` — FAIL.
- [ ] **Step 3: Add dependencies, implement, write the notices file.**
- [ ] **Step 4: Run** `cargo test -p mindweave-desktop` — PASS.
- [ ] **Step 5: Commit** `feat: equation rendering with RaTeX`

---

### Task 5: Equations and note badges on the canvas

**Files:**
- Modify: `crates/mindweave-desktop/src/editor/canvas.rs` (topic fit pass ~line 538)
- Modify: `crates/mindweave-desktop/src/editor/shape_paint.rs` (`paint_topic_extras`, culled branch)
- Modify: `crates/mindweave-desktop/src/editor/mod.rs` (field `equation_cache: EquationCache`)
- Modify: `crates/mindweave-desktop/src/editor/mindmap_edit.rs` (hit tests)
- Modify: `crates/mindweave-desktop/src/assets.rs` (accent layers for the note icon `sticky-note` and the equation icon `sigma`)

**Interfaces:**
- Consumes: Tasks 1, 3, 4.
- Produces:
  - Fit pass: for each topic with an equation, render at `equation_font_size(label)` in the topic's auto text colour; when the rendered size differs from `equation.size`, call `session.set_topic_equation_size` (before the topic size check, so the same pass also refits the topic).
  - Paint: equation picture in `parts.equation`; the note badge (`sticky-note` Lucide icon, stroke colour as the link badge) in `parts.note`. Culled topics call `equation_cache.keep`.
  - `EditorWindow::note_badge_at(&self, at: Pt) -> Option<Uuid>` and `EditorWindow::equation_at(&self, at: Pt) -> Option<Uuid>` in `mindmap_edit.rs`, sharing a private helper with `link_badge_at` (`fn topic_part_at(&self, at: Pt, part: impl Fn(&TopicParts) -> Option<Rect>, slack: f32) -> Option<&Shape>`; slack 3 for badges, 0 for the equation).

- [ ] **Step 1: Write failing tests** in `mindmap_edit.rs` (session-backed, like existing editor tests): `note_badge_and_equation_are_hit` — a topic with note, link and a measured equation: a point in `parts.note` returns the topic for `note_badge_at` and not `link_badge_at`; a point in `parts.equation` returns it for `equation_at`; a point in the text returns none.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p mindweave-desktop` — PASS.
- [ ] **Step 5: Manual check:** a topic given `\ce{2H2 + O2 -> 2H2O}` by a temporary test file shows the equation below its text, sharp at 400% zoom, and in light text on the One Dark theme; switching to Mindweave Light re-renders it dark.
- [ ] **Step 6: Commit** `feat: show equations and note badges on topics`

---

### Task 6: Equation card

**Files:**
- Create: `crates/mindweave-desktop/src/editor/equation_card.rs`
- Modify: `crates/mindweave-desktop/src/actions.rs` (action `EditEquation`, no default shortcut), `editor/toolbar.rs` (Mind Map commands: add `("mm-equation", "sigma", "Equation", ...)` and `("mm-note", "sticky-note", "Note (⌘⇧N)", ...)` as a new group after the topic commands), `editor/canvas.rs` (double click: `equation_at` before `edit_text_at`), `editor/mod.rs` (state field)

**Interfaces:**
- Consumes: Task 1 `set_topic_equation`, Task 4 `render_equation`, Task 5 `equation_at`.
- Produces:
  - `pub(super) struct EquationCard { pub topic: Uuid, pub input: Entity<InputState> }`, `EditorWindow.equation_card: Option<EquationCard>`.
  - `EditorWindow::open_equation_card(&mut self, topic: Uuid, window, cx)` — needs exactly one selected topic when opened from the ribbon (else a warning notification "Select one topic to add an equation."); pre-fills the input with the current LaTeX.
  - `EditorWindow::apply_equation_card(&mut self, cx)` / `cancel_equation_card(&mut self, window, cx)`.
  - `pub(super) fn equation_to_apply(input: &str, preview: &Result<RenderedEquation, String>) -> Option<Option<String>>` — `Some(None)` when the trimmed input is empty (remove), `Some(Some(latex))` when the preview is `Ok`, `None` (Done unavailable) when it is `Err`.
  - Card UI: anchored beside the topic (same placement rule as the note card, Task 9's `card_origin`), monospace multi-line `Input`, live preview rendered with `render_equation` in the theme foreground on each input change, error text in `theme.danger`, snippet buttons inserting at the cursor: `\frac{}{}`, `\sqrt{}`, `^{}`, `_{}`, `\sum_{}^{}`, `\int_{}^{}`, `\begin{pmatrix}  \end{pmatrix}`, `\alpha`, `\ce{ -> }`. Done (⌘Return) and Cancel (Escape). The card closes when its topic is no longer on the page.

- [ ] **Step 1: Write failing tests:** `equation_to_apply` cases (empty → remove; valid → apply; invalid → none); `card_closes_when_its_topic_goes` (open the card on a topic, delete it through the session, run the editor's sync, assert `equation_card.is_none()`).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p mindweave-desktop` — PASS.
- [ ] **Step 5: Manual check:** ribbon Equation opens the card; typing updates only the preview; Done updates the topic (one ⌘Z undoes it); double-click the equation reopens it; Cancel discards.
- [ ] **Step 6: Commit** `feat: equation card`

---

### Task 7: Text editor targets labels or notes; list and link editing

**Files:**
- Modify: `crates/mindweave-desktop/src/editor/text_edit.rs`, `text_layout.rs`, `text_paint.rs`, `actions.rs`

**Interfaces:**
- Consumes: Task 1 `set_topic_note`, Task 2 list/link API.
- Produces:
  - `pub enum EditTarget { Label(Uuid), Note(Uuid) }` with `fn id(&self) -> Uuid`; `TextEditor.target: EditTarget` (all existing call sites use `EditTarget::Label`).
  - `with_text`, `edit_layout`, `end_text_edit`, `start_text_session`/`finish_text_session` dispatch on the target. For `Note`: text read from `topic.note` (empty `RichText` when `None`), written with `set_topic_note` inside the session gesture; wrap width `NOTE_TEXT_WIDTH = 296.0`; geometry supplied by the note card (Task 9) through `EditorWindow.note_geometry: Option<EditGeometry>` instead of `text_frame`.
  - New `TextCommand`s: `ToggleBullets`, `ToggleNumbered`, `Indent`, `Outdent`; Tab/Shift+Tab map to Indent/Outdent only inside a list item; Enter on an empty list item clears its `list` instead of splitting.
  - `TextLayout` indents list paragraphs by `LIST_INDENT = 18.0 × (level + 1)` and exposes `fn list_markers(&self) -> Vec<(f32, f32, String)>` (x, baseline, marker text: "•", "◦", "▪" by level for bullets; `"{n}."` for numbered via `RichText::list_number`); `text_paint` draws them. Link runs paint underlined in `theme.link` (fallback `theme.primary`).
  - Key context: while a note is being edited the editor root gets the same `TextEditing` context the canvas editor uses, so `NOT_EDITING_TEXT` bindings (⌘B sidebar) are inactive.

- [ ] **Step 1: Write failing tests** (pure `TextEditor` tests in `text_edit.rs`, using the existing `editor()` helper):
  - `bullets_toggle_and_enter_on_empty_item_ends_the_list`.
  - `tab_indents_only_inside_lists`: Tab in a plain paragraph inserts nothing list-related (existing behaviour), Tab in a bullet raises level.
  - `note_sessions_store_and_clear_notes`: begin a `Note` session on a topic, type "Hi", end → `note` has "Hi"; begin, select all, delete, end → `note == None`; the session is one undo step.
  - `cmd_b_while_editing_a_note_is_bold`: with the `TextEditing` context, `cmd-b` resolves to `TextBold`, not `ToggleLeftSidebar` (extend the existing keymap test).
  - `huge_paste_into_a_note_is_capped`: paste of `MAX_PASTE_CHARS + 10` chars keeps `MAX_PASTE_CHARS`.
  - In `text_layout.rs`: `list_markers_number_and_indent` (two numbered items → markers "1." and "2.", second-level bullet at x = 36).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test --workspace` — PASS, including all existing text tests.
- [ ] **Step 5: Commit** `feat: text editor edits notes; lists and links`

---

### Task 8: Link editing (⌘K) and opening (⌘-click)

**Files:**
- Modify: `crates/mindweave-desktop/src/editor/text_edit.rs`, `actions.rs`
- Create: `crates/mindweave-desktop/src/editor/link_popover.rs`

**Interfaces:**
- Consumes: Task 2 `StyleChange::Link`, `link_at`; Task 7 editor.
- Produces:
  - Action `TextLink` bound to `cmd-k` in the `TextEditing` context: opens a small popover with one field pre-filled with the link under the caret/selection; Enter applies `StyleChange::Link(Some(normalized_link(value)))` to the selection, or to the word at the caret when nothing is selected; an empty value applies `Link(None)`.
  - In `text_press`: with the platform modifier held, a press on a link run opens it via `cx.open_url` instead of moving the caret. Hovering a link shows a tooltip "⌘-click to open".
- [ ] **Step 1: Write failing tests:** `link_applies_to_word_at_caret` (caret inside "hello", apply → whole word linked); `empty_link_removes_it`.
- [ ] **Step 2–4:** Run (FAIL), implement, run `cargo test -p mindweave-desktop` (PASS).
- [ ] **Step 5: Commit** `feat: add, edit and open links in text`

---

### Task 9: Note card

**Files:**
- Create: `crates/mindweave-desktop/src/editor/note_card.rs`
- Modify: `editor/mod.rs` (state, render overlay), `editor/interaction.rs` (badge press), `actions.rs` (`EditNote` bound to `cmd-shift-n` / `ctrl-shift-n`), `editor/toolbar.rs` (Note button from Task 6 wired to `EditNote`), `menu_bar.rs` (Edit/Insert entry if the menu has a natural place; else skip)

**Interfaces:**
- Consumes: Task 5 `note_badge_at`; Task 7 `EditTarget::Note`, `note_geometry`; Task 8 link behaviour.
- Produces:
  - `EditorWindow.note_card: Option<Uuid>` (the topic).
  - `EditorWindow::open_note_card(&mut self, topic: Uuid, window, cx)` — selects the topic, begins a `Note` text session, places the caret at the end; `close_note_card(&mut self, window, cx)` ends it.
  - `pub(super) fn card_origin(topic: Rect, card: (f32, f32), window: (f32, f32), gap: f32) -> (f32, f32)` — screen coordinates: right of the topic if it fits, else left, else clamped inside the window with a 8 px margin; vertical: aligned to the topic top, clamped inside the window. Used by both cards.
  - `pub(super) fn note_preview(note: &RichText) -> String` — the first non-empty paragraph, cut at 80 chars with "…", for the badge tooltip.
  - Card: 320 px wide, height grows with the text up to 360 px then scrolls; formatting bar (bold, italic, underline, strike, colour, bullets, numbers, link) dispatching the existing text actions; the text area paints the note's `TextLayout` at zoom 1 and registers the platform input handler for its bounds; sets `note_geometry` each frame. Follows its topic when the canvas pans/zooms (recomputed per frame from the viewport).
  - Opening: press on the note badge (in `begin_pointer_press`, before the link badge check), the ribbon Note button or ⌘⇧N with one topic selected. Closing: Escape, a press outside the card, the badge again, or the topic leaving the page.
- [ ] **Step 1: Write failing tests:** `card_origin` (fits right; flips left near the right edge; clamped in a small window); `note_preview` (skips empty paragraphs, truncates with "…"); `note_card_closes_when_its_topic_goes`; `closing_a_new_empty_note_leaves_no_note` (open on a topic without a note, close → `note == None`, undo stack unchanged).
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets` — PASS, no warnings.
- [ ] **Step 5: Manual check** in the running app: add a note with ⌘⇧N, type with bold, a bulleted list and a link; close; the badge appears and its tooltip shows the first line; click the badge to reopen; ⌘-click the link opens the browser; ⌘Z after closing undoes the whole session; pan the canvas with the card open — it follows the topic.
- [ ] **Step 6: Commit** `feat: note card`

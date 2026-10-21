# Mindweave — Text (Milestone 2) — Design

Date: 2026-10-21
Status: Draft for approval
Builds on: `2026-10-20-app-shell-design.md`, `2026-10-20-shapes-and-libraries-design.md` (both shipped on `main`)

Milestone order (updated): **2 Text** (this document) → 3 Paths & library authoring (Pen, point edit + lasso, convert to path, "Add to Library", resize pins/stretch zones) → 4 Snapping. Connectors: not yet placed — the user will decide.

## Intent

Diagrams need words. Users must be able to put text anywhere on the canvas and inside any shape, and style it the way they would in PowerPoint or Word: per-word bold/italic/underline/strikethrough, any installed font, size and colour.

## User decisions (verbatim intent)

- A **Text tool** lives in the **Home** tab of the ribbon (next to Pointer and Hand).
- **Double-click empty canvas** → a text box appears there and you type immediately.
- **Double-click a shape** → you type that shape's **label**: centred, wraps inside the shape, moves/resizes/rotates with it, deleted with it.
- **Lines and arrows have labels too**, centred on the line's midpoint, with a small background so the line doesn't cross the text.
- Styles apply to **selected words (rich text)**: bold, italic, underline, strikethrough, font, size, colour.
- Font controls live **in the Properties sidebar only** (not in the ribbon).
- **Font dropdown** lists **all fonts installed on the Mac**, each name drawn in its own font.
- **Font size**: type a number; mouse wheel over it or ↑/↓ steps by 1; a ▾ opens presets (8, 10, 12, 14, 18, 24, 32, 48, 72).
- **Colour**: a palette with ~20 preset swatches, a row of recently used colours, and the full picker/hex.
- **Free text boxes grow as you type** (auto width) until the user drags a side handle; from then on they wrap at that width.
- Paragraph options: **horizontal align left/centre/right**; **vertical align top/middle/bottom** for shape labels.
- **Default text colour = auto contrast**: text with no colour set (`None`) is drawn black or white, whichever has more contrast against what is behind it — the shape's fill for labels (canvas background if the shape has no fill), the text box's own fill or else the page's canvas background for text boxes. It updates automatically when the fill/background changes. The Colour control shows "Auto" for `None`.
- **Line/arrow labels follow the line's actual direction** (the angle from start to end point, from frame diagonal + flips + rotation), flipped by 180° when needed so text never reads upside-down; the label turns as an endpoint is dragged. The background patch, caret, selection and IME rectangle use the same transform.
- **Resizing a text box only reflows**: every handle (side, corner, top/bottom), Shift and Alt only change the box *width* (text re-wraps, `auto_width` turns off); font sizes never change; the height always fits the content. The sidebar Transform **H is read-only** for text boxes. In a group resize, text boxes scale their position and width only (height follows content). Shape labels never scale with the shape; they re-wrap.
- Milestone 3's point editing will start with **Cmd+click** on a path (Ctrl+click on Windows/Linux) — double-click always means text. (Recorded here because it changes the earlier plan.)

## Goals

1. **Text model (core, gpui-free)**
   - A rich-text value: a list of paragraphs; each paragraph = alignment + runs; each run = text + style `{ font_family: Option<String>, size: f32, bold, italic, underline, strikethrough, color: Option<String> }` (`None` family = app default font; `None` colour = Auto contrast, see User decisions).
   - Editing operations on it, all pure and unit-tested: insert text at a position, delete a range, split/merge paragraphs (Enter/Backspace at boundaries), apply a style change to a range (toggle bold etc.; "mixed" detection for a range), set alignment for the paragraphs a range touches, query the style at a caret (what typing will use), normalise (merge adjacent identical runs, drop empty runs).
   - Positions are character offsets on grapheme boundaries (no splitting emoji/combining marks).
2. **Text on the canvas**
   - **Text box**: a new kind of document object alongside library shapes (`kind: text`), with a frame (x, y, w, h, rotation, flips not used), `auto_width: bool` (true until the user drags a side handle), the rich text, and vertical alignment ignored. Selectable, movable, rotatable, resizable like shapes (side handles set width → `auto_width = false`; height always follows content), deletable, copy/paste, undo.
   - **Shape labels**: every library shape (and placeholder) can carry an optional `label` (rich text + vertical alignment, default middle). Laid out inside the shape's frame with a small inner padding (8 canvas units), wrapping at the frame width, clipped to the frame; rotates with the shape. For line/arrow shapes the label is laid out unwrapped (single line unless the user types Enter) centred on the line midpoint, upright relative to the line's rotation, over a background in the canvas background colour with 2-unit padding.
   - Rendering via gpui text shaping with per-run fonts and decorations; scaled with zoom; cached per object (invalidate on text/style/frame/zoom change) so the 2,000-object performance goals still hold (text-heavy pages: target 500 text boxes without frame drops while panning).
3. **Editing (desktop)**
   - Enter edit mode by: Text tool click on canvas (new box), double-click empty canvas (new box, any tool), double-click a shape/line (edit/create its label), double-click an existing text box, or Enter with one text box/labelled shape selected.
   - In edit mode: caret blinks; type, Backspace/Delete, arrows (+Alt word, +Cmd line/doc), Shift-extend selection, click/drag to place caret/select, double-click word, triple-click paragraph, Cmd+A selects all text, Return = new paragraph, Shift+Return = line break within paragraph, Cmd+B/I/U and Cmd+Shift+X (strikethrough) toggle styles, Cmd+C/X/V copy/cut/paste (rich within Mindweave, plain text from other apps), Cmd+Z/Shift+Cmd+Z undo/redo typing (grouped into word-sized steps) — while editing, these keys act on the text, not on shapes.
   - Leave edit mode: Escape, clicking outside, or switching tool/page/layer. An empty new text box is removed (no undo step left behind); an emptied shape label is cleared.
   - The whole editing session is one document undo step when committed (plus fine-grained in-editor undo while editing).
   - Input methods (dead keys, accented characters, CJK IME) must work through gpui's platform text input (`InputHandler`/`EntityInputHandler`), including marked (composing) text.
4. **Properties sidebar — Text section**
   - Shown when editing text (applies to the selection, or the caret's typing style when nothing is selected), and when one or more text boxes / labelled shapes are selected without editing (applies to all their text).
   - Controls: **Font** dropdown (searchable; each family name rendered in that family; recently used at top; current font highlighted; "Mixed" when mixed), **Size** (number + wheel/↑↓ + ▾ presets; "Mixed"), **B / I / U / S** toggles (on / off / mixed state), **Colour** (swatch opening the palette: ~20 presets, recent colours (last 8, remembered in settings), full picker + hex, "theme default"), **Align** left/centre/right, and for shape labels **Vertical** top/middle/bottom.
   - Each change while not editing is one undo step; while editing it joins the edit session.
5. **Ribbon**: Home row = Pointer (V), Hand (H), **Text (T)**.
6. **File format**: version **3** (v1/v2 load). Text boxes and labels are stored structurally (paragraphs/runs/styles), not as HTML/markdown. Font families are stored by name; when a family isn't installed the app draws with the default font and the sidebar shows the stored name with a "missing" mark; the stored name is preserved on save.
7. **Clipboard**: Mindweave payload gains text boxes and labels (version 2 of the payload; version 1 still accepted). Pasting plain text from another app while *not* editing creates a new text box at the view centre with default style.

## Non-goals (this milestone)

- Bullets/numbered lists, indentation, tabs, letter spacing, line-spacing control, superscript/subscript, highlight colour, text on a path, links, spell-check, find & replace, importing rich text (RTF/HTML) from other apps, per-character rotation, vertical text, auto-fit text size to shape.
- Bundling fonts with the app (system fonts only).
- Connectors, Paths, Snapping (other milestones).

## Anti-goals

- Typing lag: a keystroke must update on the next frame even with long text (target ≤ 2 ms layout per keystroke for 2,000 characters).
- Losing text: crashes or lost input through IME, paste, or undo; leaving behind invisible empty text objects.
- Font list that freezes the app (enumerating and rendering hundreds of font previews must be lazy/virtualised).
- A sidebar that looks unlike the rest of Mindweave (keep Zed density: 26–28px rows, 13px labels).

## Constraints

- Rust + gpui-kit `=0.8.0` (gpui-pre 0.3.9 text system: `TextRun` with `font`, `color`, `underline`, `strikethrough`; `shape_text` wrapping; `all_font_names()`); gpui-component Input is plain-text only, so the canvas text editor is custom, built on gpui's `InputHandler`.
- Rich-text model, editing operations and layout-independent logic live in `mindweave-core` (unit-tested); shaping/painting/caret/IME in `mindweave-desktop`.
- macOS first; CI macOS only.

## Decisions made by Claude (reviewable)

- Text boxes carry the same `Style` as shapes (fill, stroke, stroke width, opacity; default no fill, no stroke, opacity 100%), drawn as the box background/border behind the text with 4-unit padding, so the Style section works for mixed selections.
- Sidebar changes applied to whole selected objects (not editing) change only that one property in every run of their text (e.g. setting the font keeps per-word bold); selected shapes without a label are skipped. No separate object-level default style.
- Text boxes are stored as a built-in object kind inside the existing shape list (a `Shape` with the reserved library id `mindweave` and shape id `text`, its text in the same `label` field shapes use, plus `auto_width`). They size from content and have no outline geometry, but reusing the shape list means selection, transforms, layers, clipboard and undo already work for them. Their `frame.h` is derived from layout and is not part of the dirty comparison.
- Rich text stored as paragraphs → runs (structural, not HTML).
- Default text style: app UI font family (system default via gpui), 16 canvas units, Auto colour (contrast), left aligned; shape labels default centre/middle, 14 units.
- Typing undo inside an edit session groups by word; the committed session is one document undo step.
- Recently used colours: last 8, stored in app settings.
- Plain-text paste from other apps creates a text box when not editing.
- Missing fonts fall back to the default and keep their stored name.
- Line labels: single line by default, upright along the line, background patch in canvas background colour.
- Format version 3; clipboard payload version 2.

## Success looks like

- Double-click empty canvas → type "Hello" → select "ell" → Cmd+B → it's bold; change its font to Georgia from the dropdown (names shown in their own fonts) and size via mouse wheel; pick a red swatch; click away → one undo step removes the whole edit.
- Double-click a rectangle → type a two-line label → it stays centred and wraps; rotate and resize the rectangle → the label follows.
- Double-click an arrow → label sits at its midpoint with a background.
- Japanese/accented input via IME works.
- Save, reopen: all text and styles are back; a file using a font not installed shows the fallback with the font name kept.
- 500 text boxes pan smoothly; typing in a 2,000-character box stays instant.
- CI green.

## Left to the builder

- Exact data structure inside the rich-text model (rope vs Vec<run>), caret/selection rendering details, word-boundary rules (use unicode segmentation), font-list virtualisation approach, layout caching strategy, module layout, exact preset colours (a balanced 20-colour palette that works on light and dark canvases).

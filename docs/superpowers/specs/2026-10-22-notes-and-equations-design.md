# Mindweave — Topic Notes and Equations — Design

Date: 2026-10-22
Status: Draft for approval
Builds on: the mind map work on `main` (topics, `TopicProps`, link badge) and the Text milestone (`2026-10-21-text-design.md`: `RichText`, the on-canvas text editor).

## Intent

Mind maps are used for study and technical notes. A topic's short text is not enough: users need to attach longer, formatted notes to a topic, and to show mathematical and chemical equations on topics. This should feel like XMind, the tool the user knows: notes sit behind a small indicator on the topic and open in a floating card; equations are typed as LaTeX and shown rendered inside the topic.

## User decisions

- **Notes are rich text** at "XMind level": bold, italic, underline, strikethrough, text colour, bulleted lists, numbered lists, and links.
- **A topic with a note shows an XMind-style note indicator** on the topic. **Clicking the indicator opens the note** in a **floating note card** beside the topic, where it can be read and edited in place. Clicking away or pressing Escape closes it.
- **Equations are typed as LaTeX**, like XMind, in a box with a **live preview**. Maths and chemistry matter equally: chemistry uses mhchem syntax (`\ce{2H2 + O2 -> 2H2O}`, `\ce{N2 + 3H2 <=> 2NH3}`, charges, states, isotopes, arrows with conditions) and is used as regularly as maths.
- **The rendered equation is shown inside the topic box, below its text; the topic grows to fit it.**
- **One equation per topic.** (Several lines are still possible with an `aligned` block.)
- **Adding and editing an equation:** an **Equation** button on the **Mind Map** ribbon tab opens an editor card for the selected topic; **double-clicking the equation** on a topic reopens it.
- **Rendering engine: RaTeX** (pure Rust, MIT; KaTeX fonts under SIL OFL 1.1). The user accepted its size cost (measured below). Shrinking the app through release build settings is a **separate, later task**, not part of this work.

## Evidence behind the engine choice

A throwaway spike (not kept) rendered 5 maths and 6 chemistry samples with `ratex-parser`/`ratex-layout`/`ratex-svg` 0.1.15 (`standalone` + `embed-fonts`, `SvgOptions { embed_glyphs: true, .. }`), then parsed and rasterised the SVG with resvg:

- All 11 rendered correctly: quadratic formula, Gaussian integral, Basel sum, matrix inverse, `aligned` Maxwell equations, `\ce` combustion, equilibrium `<=>`, ionic precipitation with charges and `v`, nuclear decay with `^{14}_{6}C`, `->[\Delta]` with hydrate dot and `^` gas, states `(aq)`/`(s)`.
- Output has no `<text>` elements (glyphs are embedded as paths), so it renders with no font set-up.
- Release-mode time per equation: layout + SVG 0.12–0.53 ms; usvg parse 0.09–0.43 ms. Comfortable for a live preview.
- Size, measured on the real app (release, stripped): 23.4 MB → 25.9 MB, **+2.5 MB**. About 0.7 MB is data (≈560 KB KaTeX fonts, 170 KB mhchem state machine); the rest is code (parser, layout, mhchem engine, `fancy-regex`, and newer copies of font crates gpui also uses).
- Runner-up was Typst + mitex (LaTeX → Typst): better typesetting but ~15–25 MB, two translation layers, and no mhchem. MathJax via embedded V8 was ruled out (size, non-Rust engine).

## Goals

### 1. Notes

1. **Model.** `TopicProps` gains an optional note: `note: Option<RichText>` (serialised, skipped when `None`). A note whose text is empty after editing is stored as `None`.
2. **Rich text grows two features, usable by notes** (and harmless to existing text, which never sets them):
   - **Lists:** a paragraph can be a bulleted or numbered list item, with a nesting level (at least 3 levels). Numbering counts consecutive numbered items at the same level and restarts after a non-list paragraph.
   - **Links:** a run can carry a link (a web address or file path, using the same rules as the topic link, `normalized_link`). *(My call, since the card is always editable:)* a plain click in a link places the caret so links stay editable; **⌘-click** (Ctrl+click elsewhere) opens the link, and hovering a link shows a tooltip saying so. ⌘K adds, edits or removes the link on the selection or the link under the caret.
3. **File format (user decision):** the format moves from version 4 to **version 5**, so an older build **refuses** a file containing notes, equations, list items or links with its existing "made by a newer version" message, instead of opening it, dropping that content and losing it on save. This version still opens version 1–4 files (they load unchanged). The builder decides whether every save writes version 5, or only files that use the new fields; either way the anti-goal below holds. Update the format documentation and add load/round-trip tests.
4. **Indicator.** A topic with a note shows a small note icon inside the topic box, in the badge area beside the existing link badge (both can be present). The topic's size accounts for it the way it already accounts for the link badge (`TopicProps::parts`, `size_for_text`). Hovering the indicator shows the first line or so of the note as a tooltip.
5. **Opening.** Clicking the note indicator opens the **note card** beside the topic (to the right if there is room on screen, else the left; never off-window). Clicking the indicator does not start a drag or change the selection beyond selecting that topic.
6. **The note card:**
   - A floating card in the editor window (not a separate OS window), roughly 320 px wide, growing in height with its content up to a limit, then scrolling.
   - A compact formatting bar along the top: bold, italic, underline, strikethrough, colour, bulleted list, numbered list, link. Standard shortcuts work (⌘B/⌘I/⌘U, ⌘K for link).
   - The note is editable directly; typing, selection, clipboard (rich copy/paste within the app, plain text from outside), and IME work as they do in canvas text.
   - The card stays attached to its topic while the canvas pans or zooms, and closes if the topic is deleted.
   - Closes on Escape, on a click outside it, or on the indicator again.
7. **Adding a note:** a **Note** button on the Mind Map ribbon tab (next to the existing topic commands) and the shortcut **⌘⇧N** (Ctrl+Shift+N elsewhere) open the card for the selected topic, creating an empty note. If the user closes it without typing anything, no note is created.
8. **Undo:** each note-editing session (open → close) is one undo step, like the canvas text editor's session. Adding or removing a note is undoable.

### 2. Equations

1. **Model.** `TopicProps` gains `equation: Option<String>`: the LaTeX source only. The picture is never stored; it is rendered on load and cached.
2. **Rendering.** A small module of our own wraps RaTeX behind one function (shape left to the builder, e.g. `render_equation(latex, colour, size) -> Result<Svg, Error>`), so the engine can be swapped later. Rendered results are cached by (source, colour, size) and reused while panning; re-rendered only when one of those changes.
3. **Display.** The equation is drawn inside the topic box, centred, below the topic's text (and below its picture, if any), with the usual topic padding. The topic's size grows to fit it. It is drawn in the topic's effective text colour (so it follows Auto contrast on dark and light topics) at a size that matches the topic's font size. It scales with zoom and stays sharp (re-rasterise per zoom level as SVG icons already do).
4. **Equation card** (opened by the ribbon **Equation** button with one topic selected, or by double-clicking the equation):
   - A multi-line LaTeX input (monospace), a live preview under it, and a row of insert snippets: fraction, square root, superscript/subscript, sum, integral, matrix, Greek letter, and a `\ce{}` reaction.
   - *(My call:)* only the card's preview updates while typing; **the topic on the canvas changes once, when the edit is applied**, so the map does not re-arrange on every keystroke and no temporary state is needed.
   - Invalid LaTeX shows RaTeX's error message in the preview area in an error colour. While the input is invalid, Done is unavailable and the topic keeps its previous equation.
   - **Done** (or ⌘Return) applies; **Escape** or **Cancel** discards changes. Clearing the input and applying removes the equation.
   - Applying is one undo step.
5. **Double-click** on the equation area of a topic opens the equation card; double-click elsewhere on the topic keeps today's behaviour (edit the topic text).

### 3. Ribbon

The Mind Map tab gains **Note** and **Equation** buttons in the topic commands group, with two-tone accent icons in the same style as the rest of the ribbon (`assets.rs` accent layers). Both are disabled-looking (no effect) when no single topic is selected.

### 4. Licensing

Ship the KaTeX fonts' SIL OFL 1.1 notice with the app (wherever third-party notices live; create one if none exists), and RaTeX's MIT notice.

## Non-goals

- A visual (palette-driven, WYSIWYG) equation editor. LaTeX only.
- More than one equation per topic, or equations on non-topic shapes, text boxes or inside notes.
- Images, tables, headings, or attachments inside notes.
- A notes panel in the sidebar, or a separate notes window.
- Exporting notes or equations to other formats (no export exists yet).
- Shrinking the app size (separate task, agreed).
- Full TeX fidelity: KaTeX-level coverage is the target.

## Anti-goals (would be a failure even if it technically works)

- Panning or zooming a map with many equations becomes jerky: rendering must be cached and never run per frame for unchanged equations.
- Equations unreadable on the dark themes (black on dark), or blurry when zoomed in.
- Losing a note's or an equation's content through a misclick (closing the card must keep what was typed for notes; only the explicit Cancel/Escape of the equation card discards).
- Older `.mindweave` files failing to open, or saving a file this version can't reopen.
- A note card that falls off-screen or hides the topic it belongs to.

## Constraints

- Pure Rust, offline. RaTeX crates pinned to exact versions (`=0.1.15` or the latest at build time, pinned), with the `standalone` + `embed-fonts` features and `embed_glyphs: true` (the default `<text>` output does not render in resvg).
- The existing app structure: core model in `mindweave-core` (gpui-free, unit-tested), UI in `mindweave-desktop`. Notes' list/link additions to `RichText` belong in core with tests.
- Follow the code's existing conventions (comment density, naming, small pure functions with tests, the badge hit-testing pattern in `mindmap_edit.rs`).
- Performance: the existing goal of smooth panning holds; equation rendering only on change, cached.

## Left to the builder

- How the note card's editor is built: reuse the canvas text editor's engine (`text_edit.rs`, `text_layout.rs`, `text_paint.rs`) inside a floating element, or a separate editor over the same `RichText` model. Reuse is preferred where it does not distort the canvas editor.
- The note and equation icons, exact indicator size and placement within the badge row, card styling, snippet list details.
- Cache structure and eviction for rendered equations; the exact equation font-size mapping.
- How list items are drawn (bullet glyphs, indentation per level, number format) and edited (Tab/Shift+Tab to change level, Return on an empty item ends the list — conventional behaviour).
- Whether the equation card is a popover anchored to the topic or a small floating panel, as long as it behaves as described.

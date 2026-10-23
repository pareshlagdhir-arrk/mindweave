# Mindweave

Mindweave is a native desktop app for mind maps and diagrams, written in Rust on [gpui](https://github.com/zed-industries/zed/tree/main/crates/gpui). It combines an XMind-style mind map editor with a free-form diagram canvas: shapes, connectors, rich text, images, icon packs, topic notes, and LaTeX equations for both mathematics and chemistry.

It runs fully offline and stores each document as a single `.mindweave` file.

> **Status:** early development (version 0.1.0). macOS is the primary platform; it is the only platform built in CI.

---

## Contents

- [For users](#for-users)
  - [Getting started](#getting-started)
  - [The window](#the-window)
  - [Mind maps](#mind-maps)
  - [Notes on topics](#notes-on-topics)
  - [Tasks](#tasks)
  - [Equations and chemical formulae](#equations-and-chemical-formulae)
  - [Shapes, lines and text](#shapes-lines-and-text)
  - [Images and icon packs](#images-and-icon-packs)
  - [Canvas, pages and layers](#canvas-pages-and-layers)
  - [Themes](#themes)
  - [Keyboard shortcuts](#keyboard-shortcuts)
  - [Where Mindweave keeps its data](#where-mindweave-keeps-its-data)
  - [AI agents](#ai-agents)
- [For developers](#for-developers)
  - [Prerequisites](#prerequisites)
  - [Build, run and test](#build-run-and-test)
  - [Repository layout](#repository-layout)
  - [Architecture](#architecture)
  - [File format](#file-format)
  - [Conventions](#conventions)
  - [Continuous integration](#continuous-integration)
  - [Documentation](#documentation)
- [Licence](#licence)

---

## For users

### Getting started

1. Build and launch the app (see [Build, run and test](#build-run-and-test)), or run a release build from CI.
2. The **dashboard** opens first. Choose **New file**, open a recent document from its tile, or use **Open…**.
3. Each document opens in its own editor window. Changes are saved with **⌘S**.

### The window

| Area | What it holds |
|---|---|
| **Ribbon** (top) | Tabs: **Home** (Pointer, Hand, Text, Image), **Basic Shapes**, **Mind Map**, then one tab per installed shape library. A library's shapes sit in a gallery strip; its **⌄** expander opens the full, searchable gallery. |
| **Left sidebar** | **Layers** and **Icons** panes. Hide or show the whole sidebar with **⌘B** or the button at the left of the status bar; the View menu chooses which panes it holds. |
| **Canvas** (centre) | The infinite drawing area, with page tabs below it. |
| **Properties** (right) | Settings for the canvas or for the current selection: transform, text, style, topic options. Toggle with **⌥⌘B**. |
| **Status bar** (bottom) | Page and layer, pointer position, zoom (click to reset), and the theme name (click to change). |

### Mind maps

- Open the **Mind Map** tab and click **New Mind Map** to place a central topic.
- With a topic selected, press **Tab** to add a subtopic, **Enter** to add a topic after it, and **Shift+Enter** to add one before it.
- **Summary** and **Boundary** group selected topics; **Relationship** draws a curved link from one topic to another.
- Double-click a topic (or press **F2**) to edit its text. Topics resize to fit their text, picture, badges and equation.
- A topic can carry a picture (drop an icon or image on it) and a web or file link (set in Properties; open it from the link badge).

### Notes on topics

Any topic can hold a rich text note, in the style of XMind.

- **Add a note:** select one topic and click **Note** on the Mind Map tab, or press **⌘⇧N**.
- **Read or edit a note:** a topic with a note shows a small three-line note badge. Hover it to see the first line; click it to open the **note card** beside the topic.
- **Formatting:** the card's toolbar offers bold (**⌘B**), italic (**⌘I**), underline (**⌘U**), strikethrough, text colour, bulleted lists, numbered lists, and links. Inside a list, **Tab** and **Shift+Tab** change the level (three levels); **Enter** on an empty item ends the list.
- **Links:** select text and press **⌘K** to add, edit or remove a link. **⌘-click** a link to open it; a plain click places the caret so that the link stays editable.
- **Closing:** press **Escape**, click outside the card, or click the badge again. What you typed is kept. A note left empty is removed. Each editing session is a single undo step.

### Tasks

Any topic can be a task, in the style of XMind's tasks and to-dos combined.

- **Make a task:** select one or more topics and click **Task** on the Mind Map tab, or turn on **Task** in the Properties panel. Click it again to make them plain topics.
- **What a task holds:** a rich-text description, a start and an end date, progress (0 to 100 %) and a status (Not Started, In Progress, Blocked, Done). Edit them in the Task section of the Properties panel when one task is selected; **Description → Add… / Edit…** opens the description in a card beside the topic, with the same formatting as notes (bold, italic, underline, colour, lists and links). Progress and status stay in step: 100 % is Done, Done is 100 %.
- **The badge:** a task shows a badge left of its text: a ring that fills clockwise with its progress, coloured by status, or a green tick when done. Click the badge to tick a task off, or back to Not Started.
- **Subtasks:** a topic added under a task is a task too. A task with subtasks is a parent task: its progress is the average of its subtasks', it is Done when they all are, Blocked when one is, and its dates run from the earliest start to the latest end. Its progress, status and dates are shown, not edited; only its description is its own. Plain topics among the subtopics are not counted.

### Equations and chemical formulae

Each topic can show one equation, written in LaTeX and drawn inside the topic below its text. Rendering happens inside the app (no internet connection is needed) and stays sharp at any zoom level.

1. Select one topic and click **Σ Equation** on the Mind Map tab.
2. Type LaTeX in the card. The preview updates as you type; snippet buttons insert common structures (fraction, root, powers, sum, integral, matrix, Greek letter, reaction arrow). Errors are shown in the preview.
3. Click **Done** or press **⌘Return** to apply it (one undo step). **Escape** or **Cancel** discards the change.
4. Double-click the equation on a topic to edit it again. Clear the text and click Done to remove it.

**Mathematics** uses standard LaTeX:

```latex
x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}
\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}
\begin{pmatrix} a & b \\ c & d \end{pmatrix}
```

**Chemistry** uses the mhchem notation inside `\ce{...}`:

| You type | You get |
|---|---|
| `\ce{H2O}` | H₂O (subscripts are automatic) |
| `\ce{2H2 + O2 -> 2H2O}` | a reaction arrow |
| `\ce{N2 + 3H2 <=> 2NH3}` | equilibrium arrows |
| `\ce{SO4^2-}`, `\ce{Fe^3+}` | ion charges |
| `\ce{NaCl(aq)}`, `\ce{AgCl(s)}` | states |
| `\ce{^{14}_{6}C}` | an isotope |
| `\ce{CaCO3 ->[\Delta] CaO + CO2 ^}` | a condition above the arrow; `^` marks a gas given off |
| `\ce{BaSO4 v}` | `v` marks a precipitate |
| `\ce{CuSO4.5H2O}` | a hydrate dot |

Mathematics and chemistry can be combined, for example `K = \frac{[\ce{NH3}]^2}{[\ce{N2}][\ce{H2}]^3}`. For several lines, use `\begin{aligned} ... \\ ... \end{aligned}`. An equation may be up to 1,500 characters long.

### Shapes, lines and text

- Pick a shape from **Basic Shapes** (or another library tab) and click or drag on the canvas. Double-click a tool to lock it for repeated placement.
- Lines connect to shapes and follow them when they move; their outline style, arrowheads and line jumps are set in Properties.
- **Text:** use the **Text** tool (**T**), or double-click empty canvas to create a text box. Double-click any shape to type its label. Styling (font, size, colour, bold, italic, underline, strikethrough, alignment) applies to the selected words.
- New shapes remember the style you last gave a shape of their kind.
- **Arrange:** group (**⌘G**), ungroup (**⌘⇧G**), and change stacking order (**⌘]**, **⌘[**, with Shift for front and back).

### Images and icon packs

- **Images:** click **Image** on the Home tab or paste a picture. PNG, JPEG, GIF, WebP, BMP, TIFF and SVG are supported. Pictures scale in proportion.
- **Icon packs:** the **Icons** pane (**⌘⇧I**) lists imported icon sets. Click **+** to import a folder or a `.zip` of SVG icons; click an icon to place it, or drag it onto the canvas or onto a topic.

### Canvas, pages and layers

- Pan with the **Hand** tool (**H**) or by holding **Space**; zoom with the trackpad, the mouse wheel, or **⌘=** and **⌘-**; **⌘0** resets the zoom.
- The canvas background can be plain, lined or dotted, with configurable spacing and colours (Properties, with nothing selected).
- A document has several **pages** (tabs under the canvas), and each page has its own **layers**. Hidden layers cannot be edited.

### Themes

Three themes are bundled: **Mindweave One Dark** (default), **Mindweave Graphite** and **Mindweave Light**. Open the picker with **⌘K ⌘T** or by clicking the theme name in the status bar.

### Keyboard shortcuts

On Windows and Linux builds, read **⌘** as **Ctrl**.

**Files and view**

| Shortcut | Action |
|---|---|
| ⌘N / ⌘O / ⌘S / ⌘W / ⌘Q | New / Open / Save / Close window / Quit |
| ⌘B | Show or hide the left sidebar |
| ⌥⌘B | Show or hide Properties |
| ⌘⇧I | Show or hide the Icons pane |
| ⌘= / ⌘- / ⌘0 | Zoom in / out / actual size |
| ⌘K ⌘T | Theme picker |

**Canvas**

| Shortcut | Action |
|---|---|
| V / H / T | Pointer / Hand / Text tool |
| R / O / L | Rectangle / Ellipse / Line |
| Arrow keys (Shift for further) | Nudge the selection |
| ⌘Z / ⌘⇧Z | Undo / Redo |
| ⌘X / ⌘C / ⌘V / ⌘D | Cut / Copy / Paste / Duplicate |
| ⌘A | Select all |
| Delete / Backspace | Delete |
| ⌘G / ⌘⇧G | Group / Ungroup |
| ⌘] / ⌘[ / ⌘⇧] / ⌘⇧[ | Bring forward / Send backward / Bring to front / Send to back |
| Enter / F2 | Edit the selected text or label |
| Escape | Cancel the current action |

**Mind maps**

| Shortcut | Action |
|---|---|
| Tab | Add a subtopic |
| Enter / Shift+Enter | Add a topic after / before |
| ⌘⇧N | Open the note of the selected topic |
| ⌘Return (in the equation card) | Apply the equation |

**While editing text** (labels, text boxes and notes)

| Shortcut | Action |
|---|---|
| ⌘B / ⌘I / ⌘U / ⌘⇧X | Bold / Italic / Underline / Strikethrough |
| ⌘K | Add, edit or remove a link |
| Tab / Shift+Tab (in a list) | Indent / outdent the list item |
| Enter / Shift+Enter | New paragraph / line break |
| ⌥← ⌥→, ⌘← ⌘→, ⌘↑ ⌘↓ | Move by word, line, document (add Shift to select) |

### Where Mindweave keeps its data

On macOS, under `~/Library/Application Support/Mindweave/`:

| Path | Contents |
|---|---|
| `settings.json` | Theme, recent files, window sizes, panel visibility, recent colours and fonts, remembered styles |
| `libraries/` | Your own shape libraries (see [docs/libraries.md](docs/libraries.md)) |
| `icons/` | Imported icon packs |

Documents themselves are saved wherever you choose, as `.mindweave` files.

### AI agents

AI agents can build and edit Mindweave documents (mind maps, diagrams and artboards) and look at the result as a picture through `mindweave-mcp`, a Model Context Protocol server that works with Claude Code, Claude Desktop, Codex, opencode, Cursor and other MCP clients. A short skill in [`skills/mindweave`](skills/mindweave/SKILL.md) explains the workflow to agents. Agents can do what the app does with topics, including notes, equations, links and [tasks](#tasks) (with rich-text descriptions, dates, progress, status and parent tasks that roll up their subtasks).

Quick start (details in [docs/agents.md](docs/agents.md)):

1. **Install** the server: `cargo install --path crates/mindweave-mcp` (installs `~/.cargo/bin/mindweave-mcp`).
2. **Register** it once with your client, for example `claude mcp add --scope user mindweave -- ~/.cargo/bin/mindweave-mcp` or `codex mcp add mindweave -- ~/.cargo/bin/mindweave-mcp`.
3. **Use** it: open a new client session and ask, for example, "Create ~/notes/launch.mindweave with a project plan as tasks".

You never start or stop the server yourself: the client starts it with a session and stops it when the session ends. See [Starting, stopping and updating](docs/agents.md#starting-stopping-and-updating) for turning it off, removing it and picking up a new build.

---

## For developers

### Prerequisites

- **Rust 1.95**, pinned in `rust-toolchain.toml` (rustup installs it automatically, with `rustfmt` and `clippy`).
- **macOS** with the Xcode command-line tools (`xcode-select --install`). gpui renders with Metal.
- No other system libraries are required: the LaTeX renderer, fonts and icons are compiled in.

### Build, run and test

```sh
cargo run -p mindweave-desktop              # debug build and launch
cargo run -p mindweave-desktop --release    # optimised build (much faster rendering)

cargo test --workspace                      # all unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The binary is named `mindweave` (`target/debug/mindweave` or `target/release/mindweave`). Debug builds are noticeably slower when drawing; judge performance with `--release`.

### Repository layout

```
crates/
  mindweave-core/        Document model and logic. No UI dependency; fully unit-tested.
    src/document.rs        Document, pages, layers, canvas settings
    src/shape.rs           Shapes, labels, styles
    src/text.rs            Rich text: runs, paragraphs, lists, links, editing operations
    src/mindmap.rs         Topics, map layout, topic parts (picture, badges, equation)
    src/task.rs            Tasks on topics and how parent tasks roll up
    src/session.rs         DocumentSession: edits, undo/redo, dirty state, saving
    src/format.rs          .mindweave load/save and format versioning
    src/library.rs         Shape library loading (built-in and user)
    src/icons.rs           Icon pack import
    src/settings.rs        App settings (settings.json)
    src/ops/               Command layer for agents and tools (validated edits, reading, validate)
    src/export.rs          SVG export of a page, artboard or objects
    src/text_measure.rs    Approximate text layout without fonts
    libraries/basic-shapes Built-in shape library (data files)
  mindweave-mcp/         MCP server for AI agents (stdio JSON-RPC, tools, PNG rendering).
  mindweave-desktop/     The gpui application.
    src/main.rs            Entry point
    src/app_state.rs       Global state: open windows, settings, recent files
    src/actions.rs         Actions and default key bindings
    src/assets.rs          Asset source: Lucide icons, the app's own icons, ribbon accent layers
    src/equation.rs        LaTeX rendering (RaTeX) and its cache
    src/dashboard.rs       Launch window
    src/menu_bar.rs        Menus
    src/editor/            The editor window:
      mod.rs                 EditorWindow and layout
      toolbar.rs             Ribbon
      canvas.rs              Canvas element, input, background (dot_tile.rs)
      interaction.rs         Pointer gestures
      shape_paint.rs         Shape, topic, badge and equation painting
      text_edit.rs           Rich text editor engine (labels, text boxes, notes)
      text_layout.rs         Text shaping and layout cache
      note_card.rs           Floating note card
      equation_card.rs       Floating equation editor
      link_popover.rs        ⌘K link popover
      *_properties.rs        Properties panel sections
docs/
  agents.md              The MCP server and AI agent setup
  libraries.md           Writing shape libraries
  superpowers/specs/     Design documents for each milestone
  superpowers/plans/     Implementation plans
skills/mindweave/        Agent skill for working with Mindweave files
THIRD_PARTY_NOTICES.md   Licences of bundled third-party components
```

### Architecture

- **Two crates, one rule:** everything that can be expressed without a window lives in `mindweave-core` and is tested there. The desktop crate turns that model into gpui elements and input handling.
- **`DocumentSession`** owns a document and is the only way to change it. Each user action becomes one undo step; multi-event gestures (dragging, typing in a text session, editing a note) are grouped into one step. Layout-derived values (topic sizes, measured equation sizes) are updated without creating undo steps or marking the document dirty.
- **Rendering** is immediate-mode on a gpui `canvas`: per-frame work is limited to lookups in caches (`GeometryCache`, `TextLayouts`, `ImageCache`, `EquationCache`, the dot pattern tile). Anything expensive is computed once and keyed by its inputs.
- **Equations** are rendered by [RaTeX](https://github.com/erweixin/RaTeX), a pure-Rust KaTeX-compatible engine with an mhchem port, to SVG with embedded glyph outlines. The SVG is rasterised per zoom scale through gpui's SVG renderer. The engine is wrapped behind `render_equation` in `equation.rs` so that it can be replaced.
- **Text editing** uses one engine (`TextEditor`) for every target: shape labels, text boxes and topic notes (`EditTarget`). It handles IME, word-grouped undo, rich copy and paste, lists and links.
- **Icons** are Lucide icons from `gpui-kit-assets`, plus the app's own icons and two-tone accent layers served by `assets.rs`.

### File format

A `.mindweave` file is pretty-printed JSON with a top-level `version` field. The current version is **8**:

| Version | Added |
|---|---|
| 1–3 | The document model as it grew: pages, layers, shapes, lines, labels and text boxes |
| 4 | Stored pictures (`images`, each stored once by id) |
| 5 | Topic notes, equations, list items and links in rich text |
| 6 | Artboards and the artboard each shape belongs to |
| 7 | The `"ink"` style colour and hollow arrowheads |
| 8 | Tasks on topics |

Files with an older version load unchanged and are saved as the current version. A file with a newer version is refused with the message "This file was made by a newer version of Mindweave", so that content an older build does not understand is never silently dropped. Clipboard payloads follow the same rule with their own version numbers.

When you add a field to the format: make it optional with a serde default, raise `FORMAT_VERSION` in `format.rs` if older builds would lose data, and add load and round-trip tests.

### Conventions

- Every item has a doc comment; comments explain why, in full sentences.
- Prefer small pure functions with unit tests; UI code calls them. Editor windows cannot be built in tests, so decision logic is extracted where it can be tested.
- Document changes go through `update_session` so that undo, dirty state and redraws stay consistent.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` must pass.
- New dependencies are declared in `[workspace.dependencies]` and pinned where their output affects rendering.
- Bundled third-party fonts and assets must be listed in `THIRD_PARTY_NOTICES.md`.

### Continuous integration

`.github/workflows/ci.yml` runs on every push and pull request on macOS: format check, clippy with warnings as errors, the full test suite, and a release build. The unsigned release binary is uploaded as a build artefact.

### Documentation

- [docs/libraries.md](docs/libraries.md): how to write and install shape libraries (no Rust required).
- [docs/agents.md](docs/agents.md): the MCP server for AI agents and how to set it up in each client.
- [docs/superpowers/specs/](docs/superpowers/specs/): design documents, one per milestone.
- [docs/superpowers/plans/](docs/superpowers/plans/): the implementation plans that followed them.

---

## Licence

Mindweave is released under the [MIT Licence](LICENSE). The notices for RaTeX and the KaTeX fonts, which are compiled into the app, are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Other dependencies, including the Lucide icons and gpui, are covered by their own licences; a complete notices list should be generated (for example with `cargo about`) before a public release.

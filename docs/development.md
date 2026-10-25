# Developing Mindweave

Setting up a development build, how the code is organised, and the rules it follows. For installing a release build, see the [README](../README.md#install-from-source).

## Prerequisites

On every system: **Rust 1.95**, pinned in `rust-toolchain.toml` (install [rustup](https://rustup.rs); it fetches the pinned toolchain with `rustfmt` and `clippy` on the first `cargo` command). The LaTeX renderer, fonts and icons are compiled in.

**macOS**

- The Xcode command-line tools: `xcode-select --install`. gpui renders with Metal.

**Windows** (10 or 11, x64)

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022) (or Visual Studio) with the **Desktop development with C++** workload, which provides the MSVC linker and the Windows SDK.
- rustup with its default `x86_64-pc-windows-msvc` toolchain (not the GNU one).
- gpui renders with DirectX 11; no other libraries are needed.
- Long paths: if a build fails on a path length error, enable long paths (`git config --global core.longpaths true`, and the Windows "Enable Win32 long paths" policy) or clone into a short folder such as `C:\src\mindweave`.

**Linux** (x64; Ubuntu 22.04 or later, or the equivalent packages elsewhere)

- A C toolchain and the libraries gpui (X11, Wayland, Vulkan, fonts) and the file dialogs (GTK 3) build against. On Debian or Ubuntu:

  ```sh
  sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev \
    libwayland-dev libvulkan-dev libx11-xcb-dev libxcb1-dev \
    libfontconfig1-dev libfreetype6-dev libssl-dev libgtk-3-dev
  ```

  On Fedora: `sudo dnf install gcc pkgconf libxkbcommon-devel libxkbcommon-x11-devel wayland-devel vulkan-loader-devel libxcb-devel fontconfig-devel freetype-devel openssl-devel gtk3-devel`.
- A GPU driver with Vulkan support (Mesa is fine). gpui runs on both Wayland and X11.

CI builds and tests on all three systems (see [Continuous integration](#continuous-integration)); that list of Linux packages is the one CI installs.

## Build, run and test

```sh
cargo run -p mindweave-desktop              # debug build and launch
cargo run -p mindweave-desktop --release    # optimised build (much faster rendering)

cargo test --workspace                      # all unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The binary is named `mindweave` (`target/debug/mindweave` or `target/release/mindweave`; `mindweave.exe` on Windows, in `target\debug\` or `target\release\`). Debug builds are noticeably slower when drawing; judge performance with `--release`. The commands are the same in PowerShell. To build and install a release for yourself, use the [install scripts](../README.md#install-from-source).

## Windows and Linux status

Mindweave is developed on macOS. It builds and passes its tests on Windows and Linux, but has not yet been tried there by hand, and these parts still need porting:

- **Shortcuts** are bound to **⌘** (`cmd-…`), which Windows and Linux read as the Windows or Super key, not **Ctrl**.
- **Menus**: the app uses the macOS menu bar; Windows and Linux have none, so menu commands are reachable only from the ribbon and shortcuts.
- **Rotated text** is drawn from glyph outlines read on macOS only; elsewhere rotated labels are not drawn.
- **App icon**: set at run time on macOS only; elsewhere it comes from packaging, which is not set up yet.

Help with any of these is welcome; check the result by running the app on that system, not only CI.

## Repository layout

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
  user-guide.md          Using the app
  development.md         This file
  agents.md              The MCP server and AI agent setup
  libraries.md           Writing shape libraries
  superpowers/specs/     Design documents for each milestone
  superpowers/plans/     Implementation plans
scripts/                 install.sh (macOS, Linux) and install.ps1 (Windows): build and install from source
skills/mindweave/        Agent skill for working with Mindweave files
THIRD_PARTY_NOTICES.md   Licences of bundled third-party components
```

## Architecture

- **Two crates, one rule:** everything that can be expressed without a window lives in `mindweave-core` and is tested there. The desktop crate turns that model into gpui elements and input handling.
- **`DocumentSession`** owns a document and is the only way to change it. Each user action becomes one undo step; multi-event gestures (dragging, typing in a text session, editing a note) are grouped into one step. Layout-derived values (topic sizes, measured equation sizes) are updated without creating undo steps or marking the document dirty.
- **Rendering** is immediate-mode on a gpui `canvas`: per-frame work is limited to lookups in caches (`GeometryCache`, `TextLayouts`, `ImageCache`, `EquationCache`, the dot pattern tile). Anything expensive is computed once and keyed by its inputs.
- **Equations** are rendered by [RaTeX](https://github.com/erweixin/RaTeX), a pure-Rust KaTeX-compatible engine with an mhchem port, to SVG with embedded glyph outlines. The SVG is rasterised per zoom scale through gpui's SVG renderer. The engine is wrapped behind `render_equation` in `equation.rs` so that it can be replaced.
- **Text editing** uses one engine (`TextEditor`) for every target: shape labels, text boxes and topic notes (`EditTarget`). It handles IME, word-grouped undo, rich copy and paste, lists and links.
- **Icons** are Lucide icons from `gpui-kit-assets`, plus the app's own icons and two-tone accent layers served by `assets.rs`.

## File format

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

## Conventions

- Every item has a doc comment; comments explain why, in full sentences.
- Prefer small pure functions with unit tests; UI code calls them. Editor windows cannot be built in tests, so decision logic is extracted where it can be tested.
- Document changes go through `update_session` so that undo, dirty state and redraws stay consistent.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` must pass.
- New dependencies are declared in `[workspace.dependencies]` and pinned where their output affects rendering.
- Bundled third-party fonts and assets must be listed in `THIRD_PARTY_NOTICES.md`.

## Continuous integration

`.github/workflows/ci.yml` runs on every push and pull request on macOS, Windows and Linux (Ubuntu): format check, clippy with warnings as errors, the full test suite, and a release build of the app and the MCP server, and a run of the install script into a temporary folder. The unsigned binaries are uploaded as build artefacts, one per system. You do not need a Windows or Linux machine for CI; GitHub provides them.

## Documentation

- [user-guide.md](user-guide.md): using the app.
- [libraries.md](libraries.md): writing and installing shape libraries (no Rust required).
- [agents.md](agents.md): the MCP server for AI agents and how to set it up in each client.
- [superpowers/specs/](superpowers/specs/) and [superpowers/plans/](superpowers/plans/): design documents and implementation plans, one per milestone.

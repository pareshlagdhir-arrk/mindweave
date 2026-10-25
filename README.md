# Mindweave

A desktop app for mind maps and diagrams, written in Rust on [gpui](https://github.com/zed-industries/zed/tree/main/crates/gpui). It works offline and saves each document as one `.mindweave` file.

- **Mind maps** in the style of XMind: map, logic and org layouts, summaries, boundaries, relationships.
- **On topics:** rich-text notes, LaTeX equations (including chemistry with mhchem), pictures, links, and **tasks** with dates, progress and status that roll up from subtasks.
- **Diagrams** on the same canvas: shapes, connectors, text, images, icon packs, artboards, SVG import.
- **AI agents** can create and edit documents through an MCP server (Claude, Codex, opencode, Cursor and others).

> **Status: early development (0.1.0).** There are no prebuilt downloads yet; you build it from source (below).
> macOS is the system it is developed and used on. Windows and Linux builds compile and pass the tests in CI, but have hardly been used: shortcuts still use the Windows/Super key instead of Ctrl, there is no menu bar, and rotated text is not drawn. Expect rough edges, and keep backups of documents you care about.

## Install from source

You need [Rust](https://rustup.rs) and your system's C build tools. The scripts check both, build a release version (the first build takes several minutes), and install for your user only, without administrator rights.

**macOS** (needs the Xcode command-line tools: `xcode-select --install`)

```sh
git clone https://github.com/pareshlagdhir-arrk/mindweave.git
cd mindweave
scripts/install.sh
```

Installs `~/Applications/Mindweave.app`, which you open from Spotlight or Launchpad.

**Linux** (needs a C compiler and the X11, Wayland, Vulkan, font and GTK development packages; the script lists the exact `apt` or `dnf` command if any are missing)

```sh
git clone https://github.com/pareshlagdhir-arrk/mindweave.git
cd mindweave
scripts/install.sh
```

Installs `~/.local/bin/mindweave` with an entry in your app menu.

**Windows** (needs [Build Tools for Visual Studio](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022) with "Desktop development with C++")

```powershell
git clone https://github.com/pareshlagdhir-arrk/mindweave.git
cd mindweave
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

Installs `%LOCALAPPDATA%\Programs\Mindweave\mindweave.exe` with a Start menu entry.

On every system the scripts also install the MCP server for AI agents (`~/.cargo/bin/mindweave-mcp`). Use `--skip-mcp` (`-SkipMcp` on Windows) to leave it out, or `--skip-app` (`-SkipApp`) to install only the server. To update, pull the latest changes and run the script again. To uninstall, delete what it installed (listed above) and the MCP server.

## First steps

1. Open Mindweave and choose **New file** on the dashboard.
2. On the **Mind Map** tab, click **New Mind Map**. Press **Tab** to add a subtopic and **Enter** to add a sibling; double-click a topic to edit its text.
3. Select a topic and use **Note**, **Equation** or **Task** on the same tab to add detail.
4. Draw shapes from the **Basic Shapes** tab; lines attach to shapes and follow them.
5. Save with **⌘S** (Ctrl+S is not mapped on Windows and Linux yet).

The [user guide](docs/user-guide.md) covers everything else, including the keyboard shortcuts.

## AI agents

`mindweave-mcp` lets an agent build and edit `.mindweave` files (topics, notes, equations, tasks, shapes, artboards) and look at the result as a picture. Register it once with your client, then ask in plain words, for example "Create ~/notes/plan.mindweave with a launch plan as tasks":

```sh
claude mcp add --scope user mindweave -- ~/.cargo/bin/mindweave-mcp   # Claude Code (run in a terminal)
codex mcp add mindweave -- ~/.cargo/bin/mindweave-mcp                 # Codex
```

The client starts and stops the server itself. The Claude desktop app, opencode, Cursor and other clients are configured with a short JSON entry instead; see [docs/agents.md](docs/agents.md). Review what an agent produces before you rely on it.

## Documentation

- [User guide](docs/user-guide.md): using the app.
- [AI agents](docs/agents.md): MCP server setup for each client, the tools, starting, stopping and updating.
- [Shape libraries](docs/libraries.md): making and installing your own shapes.
- [Development](docs/development.md): building for development, code layout, architecture, file format, conventions.

## Contributing

Issues and pull requests are welcome. Before a pull request, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks on macOS, Windows and Linux. Help with the Windows and Linux ports is especially useful; see [Windows and Linux status](docs/development.md#windows-and-linux-status).

## Licence

[MIT](LICENSE). The notices for RaTeX and the KaTeX fonts, which are compiled into the app, are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Other dependencies, including gpui and the Lucide icons, are under their own licences; a complete notices list will be generated before the first public release.

# AI agents

Mindweave ships an MCP server, `mindweave-mcp`, that lets AI agents create,
read and edit `.mindweave` documents (mind maps, diagrams and artboards)
and look at the result as a picture. It works with any client that speaks
the [Model Context Protocol](https://modelcontextprotocol.io) over stdio:
Claude Code, Claude Desktop, Codex, opencode, Cursor and others.

A short skill, [`skills/mindweave/SKILL.md`](../skills/mindweave/SKILL.md),
tells agents how to use it well.

## What it does

- **Headless and stateless.** Every tool takes `path`, the absolute path of
  a `.mindweave` file. Each call opens the file, makes one change, saves it
  (to a temporary file renamed over the original, as the app saves) and
  returns compact JSON with the ids it created and a short summary.
- **Tools.** Documents: `create_document`, `document_summary`, `add_page`,
  `add_layer`, `set_active`. Reading: `list_objects`, `get_object`,
  `validate`, `list_libraries`, `list_artboard_presets`. Mind maps:
  `add_mind_map` (with a whole topic tree), `add_topics`, `set_topic`
  (text, note, equation, link), `add_summary`, `add_boundary`,
  `add_relationship`, `set_structure`. Diagrams: `add_shape`, `add_text`,
  `connect`, `add_line`, `group`, `ungroup`, `set_style`, `set_text`,
  `set_name`, `move_objects`, `set_frame`, `delete_objects`, `arrange`.
  Artboards: `add_artboard`. Pictures: `export_png` (returned as an image,
  optionally written to a `.png`), `export_svg`.
- **Same rules as the app.** Edits go through the same document model:
  mind maps are laid out automatically, connectors stay attached to the
  shapes they join, objects placed inside an artboard join it, and the file
  format version is the app's.
- **Text sizes are estimated.** Without the app's fonts, text boxes and
  topics are sized from an average character width. When the app opens the
  file it measures the text and refits them, so sizes may shift slightly.
- **Pictures are close, not identical.** Exported pictures follow the app
  closely enough to check layout; equations are rendered with the app's
  LaTeX engine, link and note badges are not drawn, and fonts come from
  the system.
- **Safety.** The server only touches the paths it is given, reads and
  writes only files ending in `.mindweave` (exports only `.png` or `.svg`),
  requires absolute paths, never replaces an existing document with
  `create_document` and never deletes files. Texts, batches and picture
  sizes are limited. Logs go to stderr; stdout carries only protocol
  messages.

## Build and install

```sh
cargo install --path crates/mindweave-mcp     # installs ~/.cargo/bin/mindweave-mcp
# or
cargo build --release -p mindweave-mcp        # target/release/mindweave-mcp
```

Options:

| Option | Meaning |
|---|---|
| `--libraries-dir DIR` | Folder of user shape libraries. Default: the app's (`~/Library/Application Support/Mindweave/libraries` on macOS), or `$MINDWEAVE_LIBRARIES_DIR`. |
| `--help`, `--version` | Show help or the version. |

In the snippets below, replace `/path/to/mindweave-mcp` with the installed
binary (for example `/Users/me/.cargo/bin/mindweave-mcp`).

## Client setup

> The snippets follow each client's documentation as we understood it and
> were not all tested against current releases; check your client's MCP
> documentation if one does not work.

### Claude Code

```sh
claude mcp add mindweave -- /path/to/mindweave-mcp
# available in every project:
claude mcp add --scope user mindweave -- /path/to/mindweave-mcp
```

### Claude Desktop

In `claude_desktop_config.json` (Settings → Developer → Edit Config):

```json
{
  "mcpServers": {
    "mindweave": {
      "command": "/path/to/mindweave-mcp",
      "args": []
    }
  }
}
```

### Codex

In `~/.codex/config.toml`:

```toml
[mcp_servers.mindweave]
command = "/path/to/mindweave-mcp"
args = []
```

### opencode

In `opencode.json` (project) or `~/.config/opencode/opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "mindweave": {
      "type": "local",
      "command": ["/path/to/mindweave-mcp"],
      "enabled": true
    }
  }
}
```

### Cursor

In `.cursor/mcp.json` (project) or `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "mindweave": {
      "command": "/path/to/mindweave-mcp",
      "args": []
    }
  }
}
```

### Any other MCP client

Register a **stdio** server whose command is the `mindweave-mcp` binary,
with no arguments (or `--libraries-dir DIR`). It speaks MCP protocol
versions 2025-06-18, 2025-03-26 and 2024-11-05 with newline-delimited
JSON-RPC 2.0.

## Using the skill

The skill is a plain folder in the open Agent Skills format
(`SKILL.md` with `name` and `description` frontmatter):

- **Claude Code:** copy `skills/mindweave` to `~/.claude/skills/mindweave`
  (all projects) or `.claude/skills/mindweave` (one project).
- **Clients with skills folders** (for example Codex and opencode): copy
  the folder where the client looks for skills; see its documentation.
- **Any client:** add a line to the project's `AGENTS.md` (or `CLAUDE.md`)
  such as "When working with .mindweave files, follow
  skills/mindweave/SKILL.md", or paste the skill's body into the client's
  custom instructions.

## Example

> Create ~/notes/launch.mindweave with a mind map of a product launch:
> research, build, marketing and support, two or three subtopics each, and
> a note on the riskiest topic. Then show me the picture.

The agent calls `create_document`, then `add_mind_map` with the whole
tree, then `export_png`, looks at the image and adjusts.

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
  (text, note, equation, link, task), `add_summary`, `add_boundary`,
  `add_relationship`, `set_structure`. Diagrams: `add_shape`, `add_text`,
  `connect`, `add_line`, `group`, `ungroup`, `set_style`, `set_text`,
  `set_name`, `move_objects`, `set_frame`, `delete_objects`, `arrange`.
  Artboards: `add_artboard`. Pictures: `export_png` (returned as an image
  of at most 2048 px a side, optionally written to a `.png` at full size),
  `export_svg`.
- **Same rules as the app.** Edits go through the same document model:
  mind maps are laid out automatically, connectors stay attached to the
  shapes they join, objects placed inside an artboard join it, and the file
  format version is the app's.
- **Text sizes are estimated.** Without the app's fonts, text boxes and
  topics are sized from an average character width. When the app opens the
  file it measures the text and refits them, so sizes may shift slightly.
- **Pictures are close, not identical.** Exported pictures follow the app
  closely enough to check layout; equations are rendered with the app's
  LaTeX engine, task badges are drawn but link and note badges are not,
  and fonts come from the system.
- **Safety.** The server only touches the paths it is given, reads and
  writes only files ending in `.mindweave` (exports only `.png` or `.svg`;
  an existing file at an export's `out_path` is replaced, written through a
  temporary file), requires absolute paths, never replaces an existing document with
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

> Each snippet says whether it was checked against the client's current
> documentation. None was run against a live client release; if one does
> not work, check the client's MCP documentation.

### Claude Code

*Verified against the Claude Code MCP documentation.* stdio is the
default transport; options such as `--scope` go before the server name.

```sh
claude mcp add mindweave -- /path/to/mindweave-mcp
# available in every project:
claude mcp add --scope user mindweave -- /path/to/mindweave-mcp
```

### Claude Desktop

*The `mcpServers` form is the documented one; the menu path to the file
is unverified and may differ between versions.*

In `claude_desktop_config.json` (on macOS in
`~/Library/Application Support/Claude/`; Settings → Developer → Edit
Config):

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

*Verified against developers.openai.com/codex/mcp.* With the Codex CLI:

```sh
codex mcp add mindweave -- /path/to/mindweave-mcp
```

Or in `~/.codex/config.toml`:

```toml
[mcp_servers.mindweave]
command = "/path/to/mindweave-mcp"
args = []
```

### opencode

*Verified against opencode.ai/docs/mcp-servers.* Newer ("v2")
documentation nests servers one level deeper, under `"mcp": { "servers":
{ "mindweave": { ... } } }`; use that form if your version's documentation
shows it.

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

*Verified against cursor.com/docs/context/mcp.*

In `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` (all workspaces).
For the same server name, the project file wins over the global one:

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

*Follows the MCP specification (stdio transport), which the server's tests
exercise.* Register a **stdio** server whose command is the `mindweave-mcp` binary,
with no arguments (or `--libraries-dir DIR`). It speaks MCP protocol
versions 2025-06-18, 2025-03-26 and 2024-11-05 with newline-delimited
JSON-RPC 2.0.

## Starting, stopping and updating

The server is not run by hand. Once registered, the client starts
`mindweave-mcp` when a session begins, talks to it over stdin and
stdout, and stops it when the session ends; between calls it holds no
files open. Run in a terminal, it waits silently for JSON-RPC input.

| To | Do |
|---|---|
| Start it | Register it (see [Client setup](#client-setup)), then open a new client session. |
| Check it | `claude mcp list` or `codex mcp list`; in Claude Code, `/mcp` shows it and its tools. |
| Stop it | Close the client session or quit the client. |
| Turn it off but keep it | Claude Code: disable it in `/mcp`. opencode: `"enabled": false`. Other clients: remove or comment out its entry. |
| Remove it | `claude mcp remove mindweave`, `codex mcp remove mindweave`, or delete its entry from the client's configuration file. |
| Update it | `cargo install --path crates/mindweave-mcp` after pulling changes, then start a new session (in Claude Code, `/mcp` can reconnect it). The installed binary is a copy: new features, such as tasks, reach agents only after this. |

To check the binary by hand:

```sh
echo '{"jsonrpc":"2.0","id":1,"method":"ping"}' | ~/.cargo/bin/mindweave-mcp
```

It prints `{"jsonrpc":"2.0","id":1,"result":{}}`.

## Tasks

Any topic can be a task. In `add_mind_map` and `add_topics` tree nodes,
and in `set_topic`, pass `task`:

```json
{"text": "Build", "task": {"description": "First **release**", "start": "2026-11-02", "end": "2026-11-27"},
 "children": [{"text": "API", "task": {"status": "done"}}, {"text": "UI", "task": {"progress": 40}}]}
```

- `description` is simple Markdown, like `note`; `start` and `end` are
  `YYYY-MM-DD`; `progress` is 0 to 100; `status` is `not-started`,
  `in-progress`, `blocked` or `done` (done sets progress to 100,
  not-started to 0). An empty description or date clears it;
  `{"remove": true}` makes the topic a plain topic again.
- Topics added under a task are tasks too. A task with subtasks is a
  parent task: `get_object` reports its progress (the average of its
  subtasks'), status and dates (earliest start, latest end) rolled up,
  with `subtasks` counting them; set progress and status on the tasks
  without subtasks.
- Exported pictures show each task's badge.

## Using the skill

The skill is a plain folder in the open Agent Skills format
(`SKILL.md` with `name` and `description` frontmatter):

The folder name must match the skill's `name` (`mindweave`). One location
works for several clients: **`.agents/skills/mindweave`** in a repository
(or `~/.agents/skills/mindweave` for your user) is read by both Codex and
opencode.

- **Claude Code** *(verified against the Claude Code skills documentation)*: copy `skills/mindweave` to
  `~/.claude/skills/mindweave` (all projects) or `.claude/skills/mindweave`
  (one project).
- **Codex** *(verified against the Codex skills documentation)*:
  `$HOME/.agents/skills/mindweave` (user) or `.agents/skills/mindweave` in
  the repository (Codex scans from the folder it was started in up to the
  repository root).
- **opencode** *(verified against opencode.ai/docs/skills)*:
  `.opencode/skills/mindweave/SKILL.md` (project) or
  `~/.config/opencode/skills/mindweave/SKILL.md` (global); opencode also
  reads `.claude/skills/` and `.agents/skills/` folders.
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

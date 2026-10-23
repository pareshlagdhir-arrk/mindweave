---
name: mindweave
description: Build and edit Mindweave documents (.mindweave mind maps, diagrams and artboards) through the mindweave MCP server, and check the result as a picture. Use when asked to create or change a mind map, flowchart, diagram, wireframe or any .mindweave file.
---

# Mindweave

Mindweave documents (`.mindweave`) hold pages of mind maps, diagrams and
artboards. The `mindweave` MCP server edits them: every tool takes `path`
(an absolute path to the file), opens it, makes one change, saves it and
returns the ids of what it created.

## Workflow

1. **Start**: `create_document` for a new file, or `document_summary` and
   `list_objects` to learn an existing one (ids, kinds, frames, text).
2. **Build** in a few large calls rather than many small ones:
   - mind maps: `add_mind_map` with the whole `topics` tree;
   - diagrams: `add_shape` / `add_text`, then `connect`;
   - screens and slides: `add_artboard` first, then its content.
3. **Look**: `export_png` (whole page, one `artboard_id`, or some `ids`)
   and inspect the image. Text sizes are estimates until the Mindweave app
   opens the file, so allow a little slack.
4. **Fix** what is off (`move_objects`, `set_frame`, `set_text`,
   `set_style`, `delete_objects`) and look again.
5. **Check**: `validate` should report no errors.

## Mind maps

- One central idea; 3 to 7 main topics; short topic texts (a few words).
  Put detail in notes (`note` takes simple Markdown: `- ` bullets,
  `1. ` items, `**bold**`, `*italic*`), not in topic text.
- Topics are laid out automatically; never position them by hand. Choose
  `structure`: `map` (both sides), `logic` (to the right) or `org`
  (downwards, for hierarchies).
- `add_topics` with `position` `after`/`before` inserts siblings;
  `add_summary` brackets siblings; `add_boundary` encloses a branch;
  `add_relationship` links two topics across branches; `equation` takes
  LaTeX (mhchem works for chemistry).
- For plans and to-do lists, make topics tasks: `task` (in a tree node or
  `set_topic`) takes `description`, `start`/`end` (`YYYY-MM-DD`),
  `progress` (0 to 100) and `status` (`not-started`, `in-progress`,
  `blocked`, `done`). Topics added under a task are tasks too. Set
  progress and status on the tasks without subtasks only: a parent task
  rolls up its subtasks' (average progress, earliest start, latest end).

## Diagrams

- `list_libraries` shows shapes (Basic Shapes: rectangle, ellipse, line,
  triangle, diamond, star) and any user libraries or templates.
- Coordinates are canvas units; y grows downwards; `x`, `y` are the
  top-left corner. Lay shapes on a grid (for example 160 units apart) and
  keep sizes consistent; flowcharts usually run top to bottom or left to
  right.
- Use `connect` (not `add_line`) between shapes: connectors attach to both
  ends, meet the outlines and follow the shapes when they move. Use
  `end_head` `triangle` or `arrow` for flow, `route` `angular` for tidy
  orthogonal lines, and `text` for labels on decisions.
- `group` things that belong together; `arrange` changes stacking.

## Artboards

- Create the artboard before its content: objects whose centre falls
  inside it become members, move with it and are clipped to it.
- `list_artboard_presets` gives phone, tablet, desktop, paper, slide and
  social sizes; without `x`/`y`, each new artboard goes right of the last.
- `export_png` with `artboard_id` shows exactly one artboard.

## Style

- Colours: `#RRGGBB`, or `auto` (follows the light or dark theme: canvas
  colour for fills, text colour for outlines), `ink` (the theme's text
  colour) or `none`. Prefer `auto` for neutral shapes and a small palette
  of accent colours for meaning; keep text readable (text colour `auto`
  contrasts with the fill).
- Text: `size` 14 to 18 for labels, 24 to 40 for titles; `bold` for
  headings.

## Limits

- One file per call; at most 500 objects or topics per call and 10,000
  characters per text. Paths must be absolute and end in `.mindweave`;
  existing files are never replaced by `create_document`.
- Images and icons cannot be added yet; links and note badges are not
  drawn in exported pictures.
- Changes are written to the file. If the file is open in the Mindweave
  app at the same time, reopen it there to see them.

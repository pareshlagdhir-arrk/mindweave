# Mindweave user guide

How to use the app. For installing it, see the [README](../README.md); for AI agents, [agents.md](agents.md).

## Getting started

1. Open Mindweave. The **dashboard** opens first: choose **New file**, open a recent document from its tile, or use **Open…**.
2. Each document opens in its own window. Save with **⌘S**; documents are single `.mindweave` files, stored wherever you choose.

## The window

| Area | What it holds |
|---|---|
| **Ribbon** (top) | Tabs: **Home** (Pointer, Hand, Text, Image), **Basic Shapes**, **Mind Map**, then one tab per installed shape library. A library's shapes sit in a gallery strip; its **⌄** expander opens the full, searchable gallery. |
| **Left sidebar** | **Layers** and **Icons** panes. Hide or show the whole sidebar with **⌘B** or the button at the left of the status bar; the View menu chooses which panes it holds. |
| **Canvas** (centre) | The infinite drawing area, with page tabs below it. |
| **Properties** (right) | Settings for the canvas or for the current selection: transform, text, style, topic options. Toggle with **⌥⌘B**. |
| **Status bar** (bottom) | Page and layer, pointer position, zoom (click to reset), and the theme name (click to change). |

## Mind maps

- Open the **Mind Map** tab and click **New Mind Map** to place a central topic.
- With a topic selected, press **Tab** to add a subtopic, **Enter** to add a topic after it, and **Shift+Enter** to add one before it.
- **Summary** and **Boundary** group selected topics; **Relationship** draws a curved link from one topic to another.
- Double-click a topic (or press **F2**) to edit its text. Topics resize to fit their text, picture, badges and equation.
- A topic can carry a picture (drop an icon or image on it) and a web or file link (set in Properties; open it from the link badge).

## Notes on topics

Any topic can hold a rich text note, in the style of XMind.

- **Add a note:** select one topic and click **Note** on the Mind Map tab, or press **⌘⇧N**.
- **Read or edit a note:** a topic with a note shows a small three-line note badge. Hover it to see the first line; click it to open the **note card** beside the topic.
- **Formatting:** the card's toolbar offers bold (**⌘B**), italic (**⌘I**), underline (**⌘U**), strikethrough, text colour, bulleted lists, numbered lists, and links. Inside a list, **Tab** and **Shift+Tab** change the level (three levels); **Enter** on an empty item ends the list.
- **Links:** select text and press **⌘K** to add, edit or remove a link. **⌘-click** a link to open it; a plain click places the caret so that the link stays editable.
- **Closing:** press **Escape**, click outside the card, or click the badge again. What you typed is kept. A note left empty is removed. Each editing session is a single undo step.

## Tasks

Any topic can be a task, in the style of XMind's tasks and to-dos combined.

- **Make a task:** select one or more topics and click **Task** on the Mind Map tab, or turn on **Task** in the Properties panel. Click it again to make them plain topics.
- **What a task holds:** a rich-text description, a start and an end date, progress (0 to 100 %) and a status (Not Started, In Progress, Blocked, Done). Edit them in the Task section of the Properties panel when one task is selected; **Description → Add… / Edit…** opens the description in a card beside the topic, with the same formatting as notes (bold, italic, underline, colour, lists and links). Progress and status stay in step: 100 % is Done, Done is 100 %.
- **The badge:** a task shows a badge left of its text: a ring that fills clockwise with its progress, coloured by status, or a green tick when done. Click the badge to tick a task off, or back to Not Started.
- **Subtasks:** a topic added under a task is a task too. A task with subtasks is a parent task: its progress is the average of its subtasks', it is Done when they all are, Blocked when one is, and its dates run from the earliest start to the latest end. Its progress, status and dates are shown, not edited; only its description is its own. Plain topics among the subtopics are not counted.

## Equations and chemical formulae

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

## Shapes, lines and text

- Pick a shape from **Basic Shapes** (or another library tab) and click or drag on the canvas. Double-click a tool to lock it for repeated placement.
- **Basic Shapes** has rectangles, ellipses, lines, triangles, diamonds, pentagons, hexagons, stars, a container and the logic gates AND, OR, NOT, NAND, NOR, XOR, XNOR and Buffer. The polygon, star and container shapes have settings in Properties (sides, points, header height, corner radius).
- **Containers** group a part of a diagram under a title. Only a container's border and title band belong to it: click inside it to select the shapes it holds, or drag there to select several. Double-click inside it to add a separate text box; double-click its title band to edit its title. Moving, copying or duplicating a container takes along every shape lying wholly inside it (not mind map topics or locked shapes); deleting it leaves them. Turn **Title bar** off in Properties for a plain frame. A container has no fill by default, so it never hides what is in it.
- Lines connect to shapes and follow them when they move; their outline style, arrowheads and line jumps are set in Properties. A line jumps the lines below it that it crosses: with an arc, a pointed or square hop, or a **gap** with a short wall at each cut end, so the line crossed seems to pass between them.
- **Text:** use the **Text** tool (**T**), or double-click empty canvas to create a text box. Drag a text box's side handle to change its width (the text wraps), or a corner handle to scale it: the box and every font size in it grow or shrink together. Double-click any shape to type its label. Styling (font, size, colour, bold, italic, underline, strikethrough, alignment) applies to the selected words.
- New shapes remember the style you last gave a shape of their kind.
- **Arrange:** group (**⌘G**), ungroup (**⌘⇧G**), and change stacking order (**⌘]**, **⌘[**, with Shift for front and back).

## Images and icon packs

- **Images:** click **Image** on the Home tab or paste a picture. PNG, JPEG, GIF, WebP, BMP, TIFF and SVG are supported. Pictures scale in proportion. Paste works with picture files copied in Finder, screenshots, images copied in a browser or Preview, and SVG source copied as text. When a browser copies only a link to the picture (Chrome does this for SVG images), Mindweave downloads it from that address (https or http, at most 20 MB) using the system's `curl`; nothing is downloaded otherwise.
- **Icon packs:** the **Icons** pane (**⌘⇧I**) lists imported icon sets. Click **+** to import a folder or a `.zip` of SVG icons; click an icon to place it, or drag it onto the canvas or onto a topic.

## Canvas, pages and layers

- Pan with the **Hand** tool (**H**) or by holding **Space**; zoom with the trackpad, the mouse wheel, or **⌘=** and **⌘-**; **⌘0** resets the zoom.
- The canvas background can be plain, lined or dotted, with configurable spacing and colours (Properties, with nothing selected).
- A document has several **pages** (tabs under the canvas), and each page has its own **layers**. Hidden layers cannot be edited.

## Themes

Three themes are bundled: **Mindweave One Dark** (default), **Mindweave Graphite** and **Mindweave Light**. Open the picker with **⌘K ⌘T** or by clicking the theme name in the status bar.

## Keyboard shortcuts

The shortcuts below are the macOS ones. Windows and Linux builds do not map **⌘** to **Ctrl** yet (see [Windows and Linux status](development.md#windows-and-linux-status)).

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

## Where Mindweave keeps its data

In your user data folder, under `Mindweave/`: on macOS `~/Library/Application Support/Mindweave/`, on Linux `~/.local/share/Mindweave/`, on Windows `%APPDATA%\Mindweave\`.

| Path | Contents |
|---|---|
| `settings.json` | Theme, recent files, window sizes, panel visibility, recent colours and fonts, remembered styles |
| `libraries/` | Your own shape libraries (see [libraries.md](libraries.md)) |
| `icons/` | Imported icon packs |

Documents themselves are saved wherever you choose, as `.mindweave` files.

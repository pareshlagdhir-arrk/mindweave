# Shape libraries

Every shape in Mindweave comes from a **library**: a folder of plain data
files that describes a set of shapes. You can write a library without any
Rust; you only need a text editor and, optionally, an SVG editor.

This guide describes formats 1 and 2 of the library files. Format 2 adds
the `"ink"` colour, dashed outlines, hollow arrowheads and templates
(items made of several shapes and lines); see [Format 2](#format-2).
Format 1 libraries keep loading as before.

## Installing a library

Copy the library folder into:

```
~/Library/Application Support/Mindweave/libraries/
```

Then restart Mindweave. Each library appears as a tab in the ribbon.
Built-in libraries come first, followed by your libraries sorted by `order`
and then by `name`.

If a library cannot be loaded, Mindweave still starts. It shows a notice
that names the library folder and the problem, and skips that library.

## Folder layout

```
my-flowchart/          # any folder name; the notices use it
  library.toml         # required: describes the library
  shapes.toml          # required: lists the shapes
  shapes/*.svg         # optional: outlines for SVG shapes
```

All paths inside the files are relative to the library folder. Paths that
are absolute, contain `..`, or lead outside the folder (for example through
a symbolic link) are rejected.

## A complete example

This small library has two shapes: a "Process" box built from a primitive,
and a "Decision" outline read from an SVG file.

`my-flowchart/library.toml`:

<!-- example-file: library.toml -->
```toml
format = 1
id = "my-flowchart"
name = "My Flowchart"
version = "0.1.0"
author = "Your Name"
description = "Process and decision shapes."
icon = "workflow"
order = 10
```

`my-flowchart/shapes.toml`:

<!-- example-file: shapes.toml -->
```toml
[[shape]]
id = "process"
name = "Process"
icon = "rectangle-horizontal"
default_size = [140, 70]
primitive = "rect"
style = { fill = "#E8F0FE", stroke = "#3B6FD8", stroke_width = 1.5 }

[[shape.param]]
key = "corner_radius"
label = "Corner radius"
min = 0
max = 40
step = 1
default = 8

[[shape]]
id = "decision"
name = "Decision"
icon = "shapes/decision.svg"
default_size = [120, 90]
svg = "shapes/decision.svg"
style = { fill = "#FFF4D6", stroke = "#C99A2E" }
```

`my-flowchart/shapes/decision.svg`:

<!-- example-file: shapes/decision.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 90">
  <polygon points="60,0 120,45 60,90 0,45"/>
</svg>
```

## `library.toml`

| Field         | Type    | Required | Default    | Meaning |
|---------------|---------|----------|------------|---------|
| `format`      | integer | yes      | none       | The library format: `1`, or `2` to use the features of [Format 2](#format-2). |
| `id`          | string  | yes      | none       | A stable, unique identifier. Documents store it, so never change it after you publish the library. 1 to 64 characters: lowercase letters, digits, `-`, `_` and `.`, starting with a letter or digit. Example: `"acme.flowchart"`. |
| `name`        | string  | yes      | none       | The tab name shown in the ribbon (at most 100 characters). |
| `version`     | string  | yes      | none       | Your library's own version, for example `"1.2.0"`. |
| `author`      | string  | no       | none       | Who made the library. |
| `description` | string  | no       | none       | One sentence about the library. |
| `icon`        | string  | no       | `"shapes"` | The tab icon. See [Icons](#icons). |
| `order`       | integer | no       | `0`        | Sort key among user libraries; lower comes first. Ties are sorted by `name`. |

An optional field with the wrong type, or an `order` outside the 32-bit
integer range, gives a warning and uses its default; the library still
loads.

## `shapes.toml`

The file holds one `[[shape]]` table per shape and, in format 2, one
`[[template]]` table per [template](#templates): at most 500 of them in
all. They appear in the ribbon in the order they are written.

| Field          | Type             | Required        | Default      | Meaning |
|----------------|------------------|-----------------|--------------|---------|
| `id`           | string           | yes             | none         | Unique within the library. Documents store it, so never change it after you publish. Same character rules as the library `id`. |
| `name`         | string           | yes             | none         | Shown in tooltips and in the sidebar (at most 100 characters). |
| `icon`         | string           | no              | see below    | The ribbon icon. See [Icons](#icons). Without one, an SVG shape uses its own outline and a primitive shape uses `"shapes"`. |
| `default_size` | `[width, height]`| no              | `[120, 80]`  | The size, in canvas units, of a shape placed with a single click. Each value is from 1 to 10000. |
| `primitive`    | string           | one of these two| none         | A built-in shape. See [Primitives](#primitives). |
| `svg`          | string           | one of these two| none         | Path of an SVG outline file, for example `"shapes/cloud.svg"`. See [SVG outlines](#svg-outlines). |
| `style`        | table            | no              | see below    | The style of a newly placed shape. |
| `param`        | array of tables  | no              | none         | Adjustable properties of a primitive shape. See [Parameters](#parameters). |

A shape needs exactly one of `primitive` and `svg`.

### `style`

| Field          | Type   | Default     | Meaning |
|----------------|--------|-------------|---------|
| `fill`         | string | `"auto"` | Fill colour as `"#RRGGBB"`, `"auto"` for the canvas colour of the current theme, `"ink"` (format 2) for the theme's text colour, or `"none"` for no fill. |
| `stroke`       | string | `"auto"` | Outline colour as `"#RRGGBB"`, `"auto"` or `"ink"` (format 2) for the theme's text colour (white in dark themes, dark in light ones), or `"none"` for no outline. |
| `stroke_width` | number | `1.5`       | Outline width in canvas units, from 0 to 100. |
| `opacity`      | number | `1.0`       | From 0 (invisible) to 1 (opaque). |
| `dash`         | string | `"solid"`   | Format 2: `"solid"`, `"dashed"` or `"dotted"` outline. |

An invalid colour gives a warning and is treated as `"none"`. An invalid
width or opacity gives a warning and uses the default.

### Parameters

Each `[[shape.param]]` table below a `[[shape]]` declares one property that
users can change in the sidebar. Its `key` must be one of the parameters
of the shape's primitive.

| Field     | Type   | Required | Default            | Meaning |
|-----------|--------|----------|--------------------|---------|
| `key`     | string | yes      | none               | The primitive parameter, for example `"corner_radius"`. |
| `label`   | string | no       | the key, as words  | The sidebar label, for example `"Corner radius"`. |
| `min`     | number | yes      | none               | The smallest value allowed. |
| `max`     | number | yes      | none               | The largest value allowed; not less than `min`. |
| `step`    | number | no       | `1`                | The increment; greater than 0. |
| `default` | number | no       | `min`              | The value of a newly placed shape. It is clamped to `min..max`. |

Values in documents are clamped to `min..max` when the shape is drawn.
A `min..max` range wider than the primitive's own limits (see
[Primitives](#primitives)) is narrowed to those limits when the library
loads, with a warning, so the sidebar never offers values the shape
ignores.
A parameter that a primitive reads but the shape does not declare is not
shown in the sidebar and uses the primitive's own default. SVG shapes have
no parameters in format 1.

## Primitives

| Primitive | Parameters (own default and limits) | Drawing |
|-----------|-------------------------------------|---------|
| `rect`    | `corner_radius` (0; at most half the shorter side) | A rectangle filling the box, optionally with rounded corners. |
| `ellipse` | none | An ellipse filling the box. |
| `line`    | none | A line from the top-left to the bottom-right corner of the box. Stroke only. |
| `arrow`   | `head_size` (12; at least 0) | A line like `line`, with a filled arrowhead at its end in the stroke colour. |
| `polygon` | `sides` (3; whole numbers from 3 to 12) | A regular polygon with a vertex at the top centre. |
| `star`    | `points` (5; whole numbers from 3 to 20), `inner_ratio` (0.5; from 0.1 to 0.9) | A star; `inner_ratio` is the inner radius relative to the outer radius. |
| `container` | `header` (28; at least 0), `corner_radius` (0) | A box with a title band `header` units tall. Only its border and title band are the shape: clicks inside it reach the shapes it holds, a double click inside adds a text box, and moving, copying or duplicating it takes along the shapes wholly inside it. A `header` of 0 gives no title band. Its label sits in the title band. A container always has a `header` parameter; one is added if the library declares none. |

Mindweave also clamps every value to these limits, whatever a document
says, and narrows a parameter declaration's range to them.

## SVG outlines

An SVG shape's outline is read once, when the library loads. Only geometry
is used:

- These elements contribute geometry: `path`, `rect`, `circle`, `ellipse`,
  `line`, `polyline` and `polygon`, directly under the root `svg` or
  inside `g` (or nested `svg`) elements.
- Everything else is ignored: scripts, styles, images, text, `use`, `defs`
  and their contents. Nothing is executed or fetched.
- Colours and other paint attributes are ignored. `line` and `polyline`
  are drawn as strokes only; every other element is filled with the
  shape's fill colour. Within one element, overlapping areas use the
  even-odd rule, so an inner contour of a `path` cuts a hole. Separate
  elements are filled one over another and never cut holes in each other.
- Of `transform`, only `translate` and `scale` are applied; `rotate`,
  `skewX`, `skewY` and `matrix` are skipped. Bake other transforms into
  the path before you save.
- The `viewBox` (or, without one, `width` and `height` in px, or failing
  those, the outline's own bounds) is stretched to the shape's box.

Limits: each file is at most 1 MB; documents with a DTD or with elements
nested more than 64 deep are rejected; path numbers larger than 1,000,000
make that path contribute nothing. A path used by several shapes or icons
is read once; paths that differ only in `./` parts or repeated `/`, such
as `a.svg` and `./a.svg`, count as the same path. Every file read counts
towards the library's 16 MB, including files that cannot be used (for
example, because they are not UTF-8 text).

An SVG that cannot be used gives a warning and skips only that shape.

## Icons

An `icon` value is either:

- the name of a [Lucide](https://lucide.dev/icons) icon, such as
  `"square"` or `"arrow-up-right"` (lowercase letters, digits and `-`); or
- the path of an SVG file in the library, ending in `.svg`, such as
  `"shapes/decision.svg"`. It follows the same rules as
  [SVG outlines](#svg-outlines) and is drawn as a monochrome icon.

An invalid icon gives a warning and the default icon is used. So does a
well-formed Lucide name that this version of Mindweave does not bundle;
one notice per library names the first five such icons and counts the
rest.

## Compatibility and errors

- `format` must be `1` or `2`. A library that declares a higher format is
  skipped with the notice "needs a newer version of Mindweave". A missing
  or invalid `format` skips the library too.
- A format 1 library that uses a format 2 feature gives a warning and
  skips only that feature: an `"ink"` colour is treated as `"none"`,
  `dash` is ignored, and each `[[template]]` is skipped.
- Unknown fields are ignored, so a library written for a later Mindweave
  can still load when it only adds fields. They are written to the log
  (standard error) once per launch rather than shown as a notice.
- Problems in one shape (unknown primitive, unusable SVG, duplicate shape
  `id`, missing `id` or `name`) skip only that shape, with a warning.
  Problems in one parameter skip only that parameter.
- These problems skip the whole library: a missing `library.toml` or
  `shapes.toml`, a file larger than 1 MB, files that add up to more than
  16 MB, invalid TOML, a missing or invalid required field in
  `library.toml`, more than 500 shapes and templates, or an `id` that another loaded
  library already uses. Built-in libraries keep their ids.
- At most 100 library folders are loaded, in folder name order; one
  notice counts the folders that were skipped.
- Notices quote at most 60 characters of any value from your files, and
  notices longer than 600 characters are cut short with `…`.

## Format 2

Set `format = 2` in `library.toml` to use:

- **The ink colour.** `"ink"` as a `fill` or `stroke` is the theme's text
  colour: white in dark themes, dark in light ones. Use it for solid
  marks such as a UML initial node, which must stay visible on any
  theme. (`"auto"` keeps its meaning: the canvas colour for a fill.)
- **Dashed outlines.** `dash = "dashed"` or `"dotted"` in a `style`.
- **Templates**, which can also use hollow arrowheads on their lines.

### Templates

A `[[template]]` is one ribbon item that places several shapes and lines
at once. They are scaled from the template's `default_size` to the size
the user drags (a click uses `default_size`) and grouped, so they select
and move as one; ungrouping separates them. The placed shapes are
ordinary shapes: their labels can be edited, and the ends of their lines
attached to other shapes.

A template made only of lines (such as a UML relationship or message) is
drawn like the Line tool instead: its lines run from where the drag
starts to where it ends, at any angle, and Shift snaps them to 45°
steps. A line written from right to left (`from` to the right of `to`)
so runs back towards the start of the drag. A click places the lines at
their own length, rightwards from the click. When the template is a
single line, its ends attach to the shapes they are dropped on.

| Field          | Type             | Required | Default     | Meaning |
|----------------|------------------|----------|-------------|---------|
| `id`           | string           | yes      | none        | Unique among the library's shapes and templates. Same rules as a shape `id`, but at most 61 characters, so that part ids (`<id>.<part number>`) stay within 64. |
| `name`         | string           | yes      | none        | Shown in tooltips (at most 100 characters). |
| `icon`         | string           | yes      | none        | The ribbon icon. See [Icons](#icons). An invalid icon gives a warning and `"shapes"` is used. |
| `default_size` | `[width, height]`| no       | `[120, 80]` | The template's own units: part frames and line ends are given within this box. Each value is from 1 to 10000. |
| `part`         | array of tables  | yes      | none        | The parts, back to front: 1 to 20 `[[template.part]]` tables. |

Each `[[template.part]]` is either a shape or a line:

| Field       | Type            | Meaning |
|-------------|-----------------|---------|
| `frame`     | `[x, y, w, h]`  | A shape part's box, in template units. Required for shapes; not used by lines. |
| `primitive` | string          | A shape part built from a [primitive](#primitives). |
| `svg`       | string          | A shape part from an [SVG outline](#svg-outlines). |
| `params`    | table           | A primitive part's parameter values, such as `{ corner_radius = 4 }`. Values are clamped to the primitive's limits. |
| `line`      | table           | A line part: `{ from = [x, y], to = [x, y], start_head = "none", end_head = "hollow-triangle" }`. `from` and `to` are in template units. |
| `style`     | table           | As a shape's [`style`](#style), with `dash`. Lines have no fill, and their `stroke_width` defaults to `2`. |
| `label`     | table           | The part's text: `{ text = "Name", align = "center", valign = "middle", bold = false, italic = false, underline = false, size = 14 }`. Only `text` is required (at most 1000 characters; `\n` starts a new paragraph). `align` is `"left"`, `"center"` or `"right"`; `valign` is `"top"`, `"middle"` or `"bottom"`; `size` is from 1 to 1000. |

A part has exactly one of `primitive`, `svg` and `line`. Arrowheads are
`"none"`, `"arrow"`, `"triangle"`, `"diamond"`, `"circle"`,
`"hollow-triangle"` and `"hollow-diamond"`. The hollow heads are outlined
in the line's colour and filled with the canvas colour (not left
transparent), so they hide whatever lies beneath them, such as the line
itself or the edge of a shape the head overlaps. On a coloured shape
the inside of the head shows the canvas colour, not the shape's fill.

Documents store each placed shape part as the library's shape
`<template id>.<part number>` (parts are numbered from 1 in the order
they are written), so do not reorder the parts of a published template;
add new parts at the end. If the library is not installed, these shapes
are drawn as placeholders; lines are drawn as usual.

A template with more than 20 parts, no usable parts, an `id` longer than
61 characters, or a missing `id`, `name` or `icon` is skipped with a
warning. A part with a problem (no
`frame`, an unknown primitive, both a shape and a line) is skipped on its
own; smaller problems in a label or a parameter give a warning and use a
default.

### A complete format 2 example

A small UML library: an ink-filled initial node, a class box with three
compartments, and a dashed realization arrow with a hollow head.

`my-uml/library.toml`:

<!-- format-2-file: library.toml -->
```toml
format = 2
id = "my-uml"
name = "My UML"
version = "0.1.0"
icon = "component"
```

`my-uml/shapes.toml`:

<!-- format-2-file: shapes.toml -->
```toml
[[shape]]
id = "initial-node"
name = "Initial Node"
icon = "circle-dot"
default_size = [24, 24]
primitive = "ellipse"
style = { fill = "ink", stroke = "none" }

[[template]]
id = "class"
name = "Class"
icon = "shapes/class.svg"
default_size = [160, 100]

[[template.part]]
frame = [0, 0, 160, 28]
primitive = "rect"
label = { text = "ClassName", bold = true }

[[template.part]]
frame = [0, 28, 160, 36]
primitive = "rect"
label = { text = "+ attribute: Type", align = "left", valign = "top", size = 12 }

[[template.part]]
frame = [0, 64, 160, 36]
primitive = "rect"
label = { text = "+ operation()", align = "left", valign = "top", size = 12 }

[[template]]
id = "realization"
name = "Realization"
icon = "move-up-right"
default_size = [120, 60]

[[template.part]]
line = { from = [0, 60], to = [120, 0], end_head = "hollow-triangle" }
style = { stroke = "ink", dash = "dashed" }
```

`my-uml/shapes/class.svg`, the class icon:

<!-- format-2-file: shapes/class.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <path d="M3 3H21V21H3Z M5 5V8H19V5Z M5 10V14H19V10Z M5 16V19H19V16Z"/>
</svg>
```

## Worked example: Basic Shapes

The built-in Basic Shapes library is written in this format. Its files are
in `crates/mindweave-core/libraries/basic-shapes/`. The logic gates show
how an SVG outline mixes filled parts (the gate bodies and their output
bubbles, drawn in the shape's fill colour) with stroke-only `line` and
`polyline` parts (the input and output leads, and the extra curve of XOR
and XNOR).

<!-- basic-shapes-file: library.toml -->
```toml
format = 1
id = "basic-shapes"
name = "Basic Shapes"
version = "1.0.0"
author = "Mindweave"
description = "Rectangles, ellipses, polygons, containers, logic gates and other simple shapes."
icon = "shapes"
order = 0
```

<!-- basic-shapes-file: shapes.toml -->
```toml
[[shape]]
id = "rectangle"
name = "Rectangle"
icon = "square"
default_size = [120, 80]
primitive = "rect"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape.param]]
key = "corner_radius"
label = "Corner radius"
min = 0
max = 1000
step = 1
default = 0

[[shape]]
id = "ellipse"
name = "Ellipse"
icon = "circle"
default_size = [120, 80]
primitive = "ellipse"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "line"
name = "Line"
icon = "minus"
default_size = [120, 60]
primitive = "line"
style = { fill = "none", stroke = "auto", stroke_width = 2, opacity = 1.0 }

[[shape]]
id = "triangle"
name = "Triangle"
icon = "triangle"
default_size = [120, 80]
primitive = "polygon"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape.param]]
key = "sides"
label = "Sides"
min = 3
max = 12
step = 1
default = 3

[[shape]]
id = "diamond"
name = "Diamond"
icon = "diamond"
default_size = [120, 80]
svg = "shapes/diamond.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "pentagon"
name = "Pentagon"
icon = "pentagon"
default_size = [120, 80]
primitive = "polygon"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape.param]]
key = "sides"
label = "Sides"
min = 3
max = 12
step = 1
default = 5

[[shape]]
id = "hexagon"
name = "Hexagon"
icon = "hexagon"
default_size = [120, 80]
svg = "shapes/hexagon.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "star"
name = "Star"
icon = "star"
default_size = [120, 80]
primitive = "star"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape.param]]
key = "points"
label = "Points"
min = 3
max = 20
step = 1
default = 5

[[shape.param]]
key = "inner_ratio"
label = "Inner ratio"
min = 0.1
max = 0.9
step = 0.05
default = 0.5

[[shape]]
id = "container"
name = "Container"
icon = "panel-top"
default_size = [240, 160]
primitive = "container"
style = { fill = "none", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape.param]]
key = "header"
label = "Header height"
min = 0
max = 1000
step = 1
default = 28

[[shape.param]]
key = "corner_radius"
label = "Corner radius"
min = 0
max = 1000
step = 1
default = 6

[[shape]]
id = "gate-and"
name = "AND Gate"
icon = "shapes/gate-and.svg"
default_size = [100, 60]
svg = "shapes/gate-and.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-or"
name = "OR Gate"
icon = "shapes/gate-or.svg"
default_size = [100, 60]
svg = "shapes/gate-or.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-not"
name = "NOT Gate"
icon = "shapes/gate-not.svg"
default_size = [100, 60]
svg = "shapes/gate-not.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-nand"
name = "NAND Gate"
icon = "shapes/gate-nand.svg"
default_size = [100, 60]
svg = "shapes/gate-nand.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-nor"
name = "NOR Gate"
icon = "shapes/gate-nor.svg"
default_size = [100, 60]
svg = "shapes/gate-nor.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-xor"
name = "XOR Gate"
icon = "shapes/gate-xor.svg"
default_size = [100, 60]
svg = "shapes/gate-xor.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-xnor"
name = "XNOR Gate"
icon = "shapes/gate-xnor.svg"
default_size = [100, 60]
svg = "shapes/gate-xnor.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "gate-buffer"
name = "Buffer"
icon = "shapes/gate-buffer.svg"
default_size = [100, 60]
svg = "shapes/gate-buffer.svg"
style = { fill = "auto", stroke = "auto", stroke_width = 1.5, opacity = 1.0 }
```

<!-- basic-shapes-file: shapes/diamond.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <path d="M 50 0 L 100 50 L 50 100 L 0 50 Z"/>
</svg>
```

<!-- basic-shapes-file: shapes/hexagon.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <path d="M 25 0 H 75 L 100 50 L 75 100 H 25 L 0 50 Z"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-and.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 20 5 H 55 C 69 5 80 16 80 30 C 80 44 69 55 55 55 H 20 Z"/>
  <line x1="0" y1="20" x2="20" y2="20"/>
  <line x1="0" y1="40" x2="20" y2="40"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-or.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 20 5 Q 50 5 80 30 Q 50 55 20 55 Q 32 30 20 5 Z"/>
  <line x1="0" y1="20" x2="25" y2="20"/>
  <line x1="0" y1="40" x2="25" y2="40"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-not.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 20 8 L 70 30 L 20 52 Z"/>
  <circle cx="75" cy="30" r="5"/>
  <line x1="0" y1="30" x2="20" y2="30"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-nand.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 20 5 H 45 C 59 5 70 16 70 30 C 70 44 59 55 45 55 H 20 Z"/>
  <circle cx="75" cy="30" r="5"/>
  <line x1="0" y1="20" x2="20" y2="20"/>
  <line x1="0" y1="40" x2="20" y2="40"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-nor.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 20 5 Q 50 5 70 30 Q 50 55 20 55 Q 32 30 20 5 Z"/>
  <circle cx="75" cy="30" r="5"/>
  <line x1="0" y1="20" x2="25" y2="20"/>
  <line x1="0" y1="40" x2="25" y2="40"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-xor.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 26 5 Q 56 5 80 30 Q 56 55 26 55 Q 38 30 26 5 Z"/>
  <polyline points="14,5 16.625,11.25 18.5,17.5 19.625,23.75 20,30 19.625,36.25 18.5,42.5 16.625,48.75 14,55"/>
  <line x1="0" y1="20" x2="19" y2="20"/>
  <line x1="0" y1="40" x2="19" y2="40"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-xnor.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 26 5 Q 56 5 72 30 Q 56 55 26 55 Q 38 30 26 5 Z"/>
  <circle cx="77" cy="30" r="5"/>
  <polyline points="14,5 16.625,11.25 18.5,17.5 19.625,23.75 20,30 19.625,36.25 18.5,42.5 16.625,48.75 14,55"/>
  <line x1="0" y1="20" x2="19" y2="20"/>
  <line x1="0" y1="40" x2="19" y2="40"/>
  <line x1="82" y1="30" x2="100" y2="30"/>
</svg>
```

<!-- basic-shapes-file: shapes/gate-buffer.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
  <path d="M 20 8 L 80 30 L 20 52 Z"/>
  <line x1="0" y1="30" x2="20" y2="30"/>
  <line x1="80" y1="30" x2="100" y2="30"/>
</svg>
```

# Shape libraries

Every shape in Mindweave comes from a **library**: a folder of plain data
files that describes a set of shapes. You can write a library without any
Rust; you only need a text editor and, optionally, an SVG editor.

This guide describes format 1 of the library files.

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
| `format`      | integer | yes      | none       | The library format. It must be `1`. |
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

The file holds one `[[shape]]` table per shape, at most 500 shapes. Shapes
appear in the ribbon in the order they are written.

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
| `fill`         | string | `"#F5F5F5"` | Fill colour as `"#RRGGBB"`, or `"none"` for no fill. |
| `stroke`       | string | `"#7F8EA3"` | Outline colour as `"#RRGGBB"`, or `"none"` for no outline. |
| `stroke_width` | number | `1.5`       | Outline width in canvas units, from 0 to 100. |
| `opacity`      | number | `1.0`       | From 0 (invisible) to 1 (opaque). |

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

- `format` must be `1`. A library that declares a higher format is skipped
  with the notice "needs a newer version of Mindweave". A missing or
  invalid `format` skips the library too.
- Unknown fields are ignored, so a library written for a later Mindweave
  can still load when it only adds fields. They are written to the log
  (standard error) once per launch rather than shown as a notice.
- Problems in one shape (unknown primitive, unusable SVG, duplicate shape
  `id`, missing `id` or `name`) skip only that shape, with a warning.
  Problems in one parameter skip only that parameter.
- These problems skip the whole library: a missing `library.toml` or
  `shapes.toml`, a file larger than 1 MB, files that add up to more than
  16 MB, invalid TOML, a missing or invalid required field in
  `library.toml`, more than 500 shapes, or an `id` that another loaded
  library already uses. Built-in libraries keep their ids.
- At most 100 library folders are loaded, in folder name order; one
  notice counts the folders that were skipped.
- Notices quote at most 60 characters of any value from your files, and
  notices longer than 600 characters are cut short with `…`.

## Worked example: Basic Shapes

The built-in Basic Shapes library is written in this format. Its files are
in `crates/mindweave-core/libraries/basic-shapes/`, together with
`shapes/diamond.svg`.

<!-- basic-shapes-file: library.toml -->
```toml
format = 1
id = "basic-shapes"
name = "Basic Shapes"
version = "1.0.0"
author = "Mindweave"
description = "Rectangles, ellipses, lines, arrows and other simple shapes."
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
style = { fill = "#F5F5F5", stroke = "#7F8EA3", stroke_width = 1.5, opacity = 1.0 }

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
style = { fill = "#F5F5F5", stroke = "#7F8EA3", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "line"
name = "Line"
icon = "minus"
default_size = [120, 60]
primitive = "line"
style = { fill = "none", stroke = "#7F8EA3", stroke_width = 2, opacity = 1.0 }

[[shape]]
id = "arrow"
name = "Arrow"
icon = "arrow-up-right"
default_size = [120, 60]
primitive = "arrow"
style = { fill = "none", stroke = "#7F8EA3", stroke_width = 2, opacity = 1.0 }

[[shape.param]]
key = "head_size"
label = "Head size"
min = 4
max = 100
step = 1
default = 12

[[shape]]
id = "triangle"
name = "Triangle"
icon = "triangle"
default_size = [120, 80]
primitive = "polygon"
style = { fill = "#F5F5F5", stroke = "#7F8EA3", stroke_width = 1.5, opacity = 1.0 }

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
style = { fill = "#F5F5F5", stroke = "#7F8EA3", stroke_width = 1.5, opacity = 1.0 }

[[shape]]
id = "star"
name = "Star"
icon = "star"
default_size = [120, 80]
primitive = "star"
style = { fill = "#F5F5F5", stroke = "#7F8EA3", stroke_width = 1.5, opacity = 1.0 }

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
```

<!-- basic-shapes-file: shapes/diamond.svg -->
```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <path d="M 50 0 L 100 50 L 50 100 L 0 50 Z"/>
</svg>
```

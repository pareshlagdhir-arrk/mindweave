# Mindweave — Artboards — Design

Date: 2026-10-23
Status: Draft for approval
Builds on: shapes and libraries, groups and stacking order, mind maps, notes and equations (all on `main`).

## Intent

Users design screens and documents (phone apps, web pages, A4 sheets, slides, social posts) on Mindweave's canvas. They need Figma-style artboards: named frames of a known size placed on the infinite canvas, holding and clipping the shapes, text, images and mind maps drawn inside them, so a whole screen moves as one and nothing spills past its edge.

## User decisions

- **Artboards live on the canvas itself**, like Figma frames: several can sit side by side on the infinite canvas of a page, among other objects. They are not pages, windows or layers; pages and layers work as today.
- **Behaviour like a Figma frame:** a named frame (default fill white) with its **name drawn above its top-left corner**. Shapes placed inside belong to it, **move with it, and are clipped to its edges**. Resizing the artboard does not scale its contents.
- **An Artboard ribbon tab** with **groups of preset buttons** (device-outline icons, dividers between groups) for four categories — **phones and tablets, desktop and web, paper and slides, social media** — plus a **Custom Artboard** tool to drag any size.
- **Clicking an artboard's empty area selects the artboard** (and dragging moves it). A marquee still starts on the open canvas outside artboards. **⌘-drag on an artboard's empty area draws a marquee** that selects shapes inside it.
- **No capture, like Figma:** drawing or resizing an artboard over shapes already on the canvas does not take them in; they stay on the open canvas (drawn above the artboard, not clipped) until moved onto it.
- **No export for now** (PNG/PDF export of an artboard is a later feature).

## Goals

### 1. Model (core)

- A new object kind, the **artboard**: a shape of the built-in `mindweave` library with shape id `artboard` (like topics and boundaries), carrying:
  - `frame` (x, y, w, h; never rotated or flipped),
  - `name` (the existing `Shape.name`, shown as the title; a new artboard is named after its preset, e.g. "iPhone 16", or "Artboard" for a custom one; duplicates keep the name),
  - `style`: fill default `#FFFFFF`, no stroke by default, stroke/radius/opacity editable like any shape,
  - `artboard: ArtboardProps { clip: bool }` — `clip` defaults to true (a Properties toggle "Clip content").
- **Membership:** every other shape gains `artboard: Option<Uuid>` — the artboard it belongs to (`None`: on the open canvas). Artboards themselves never belong to an artboard (no nesting).
- **Membership rule** (one pure core function): after a shape is placed, moved, resized, pasted, duplicated or dropped, it belongs to the topmost artboard on the same layer whose frame contains the shape's **bounding-box centre**, else to none. Lines (connectors) use the midpoint of their two ends. Creating, moving or resizing an *artboard* never changes other shapes' membership (no capture — user decision).
- *(My call)* **Groups join as a unit:** a group's membership is decided by the centre of the whole group's bounding box, and all its shapes share it, so a group never splits across an artboard edge.
- *(My call)* **A mind map belongs as a whole, live,** to the artboard of its central topic: all topics, summaries, boundaries and relationships of that map always share the central topic's membership. Dragging a subtopic out does not release it; topics the layout pushes past the edge stay members and are clipped. If the central topic is deleted, the existing mind map rules apply and the remaining shapes keep their membership until moved.
- **Moving an artboard moves its members** by the same offset (one undo step). A selection that contains an artboard and some of its members moves each shape once. **Resizing** changes only the frame; members keep their positions (the ones now outside its frame are still members — clipped — until moved).
- **Deleting an artboard deletes its members**; **duplicating / copying** an artboard includes its members (the copies belong to the new artboard).
- **Stacking:** an artboard always paints beneath its members. Artboards of a layer paint before that layer's other objects; within the artboards, their own stacking order applies. Members keep their relative order among themselves. *(My call)* This is enforced at paint time: the arrange commands keep working on the layer's order, and for an artboard they reorder it only among artboards; a member sent to back still paints above its artboard.
- Artboards cannot be rotated or flipped; rotation handles are not shown for them.
- **File format version 6** (an older build must refuse files containing artboards rather than drop them, as with version 5). *(My call)* Every save writes version 6, following the existing pattern. Clipboard payload version raised likewise.

### 2. Presets (core data)

A built-in, ordered preset table (name, width, height, category, Lucide icon), in canvas units (= px at 100%):

| Category | Presets |
|---|---|
| Phones and tablets | iPhone 16 (393 × 852), iPhone 16 Pro Max (440 × 956), Android (412 × 917), iPad (820 × 1180), Android tablet (800 × 1280) |
| Desktop and web | Desktop (1440 × 1024), MacBook (1512 × 982), HD screen (1920 × 1080), Web page (1280 × 3000) |
| Paper and slides | A4 (595 × 842), A4 landscape (842 × 595), A3 (842 × 1191), Letter (612 × 792), Slide 16:9 (1920 × 1080), Slide 4:3 (1024 × 768) |
| Social media | Square post (1080 × 1080), Story (1080 × 1920), Video thumbnail (1280 × 720), Banner (1584 × 396) |

Paper sizes use points (72 per inch), as Figma does. Social presets use generic names (no brand names or brand icons).

### 3. Ribbon

- A new **Artboard** tab after **Mind Map** (order: Home, Basic Shapes, Mind Map, Artboard, other libraries).
- Groups separated by dividers: **Custom Artboard** tool (`frame` icon); phones and tablets (`smartphone`, `tablet`); desktop and web (`monitor`, `laptop`); paper and slides (`file-text`, `presentation`); social (`image`, `square-play`, `rectangle-horizontal`). Each button's tooltip shows the name and size, e.g. "iPhone 16 — 393 × 852". Icons follow the ribbon's two-tone style with partial accents (outline must stay readable).
- If the row cannot fit every preset, the presets that do not fit go behind a ⌄ menu at the end of the row (as the library gallery does).
- **Clicking a preset** places a new artboard of that size on the active layer: to the right of the rightmost artboard on the page (any layer; gap 80, tops aligned) when there is one, else centred in the view; the view then scrolls to show it if it is off-screen. The new artboard is selected. A hidden active layer refuses, as other placement does.
- **Custom Artboard** is a tool (shortcut **A**): drag on the canvas to draw an artboard of any size (Shift keeps it square); a plain click places a 400 × 400 one.

### 4. Canvas behaviour

- **Painting:** fill, outline and corner radius as a rectangle; the name is drawn above the top-left corner in the UI font at a fixed screen size (11 px, muted colour; the selected artboard's name in the accent colour), not scaled with zoom. With `clip`, members (shapes, their text, pictures, equations, badges) are painted clipped to the artboard's frame. *(My call)* The clip is rectangular (rounded corners do not round the clip); the floating note and equation cards, being UI overlays, are never clipped.
- **Selecting:** clicking an artboard's name, or its empty area (no member under the pointer), selects it; dragging moves it with its members. A plain drag starting on an artboard's empty area moves the artboard; **⌘-drag there draws a marquee** (user decision). A marquee started on the open canvas selects artboards it fully encloses, like other shapes. *(My call)* The clipped-away part of a member cannot be clicked.
- **Renaming:** double-clicking the name edits it in place (Enter/Escape); the artboard's Properties also show a Name field.
- **Members:** moving shapes in or out of an artboard updates membership on drop (live clip preview while dragging is not required). Placing a shape with a tool inside an artboard makes it a member.
- **Properties for an artboard:** Name, preset picker (changes size, keeps position), W/H, Fill, Stroke, Corner radius, Clip content. Rotation is hidden.
- **Layers panel:** unchanged (it lists layers, not objects).

## Non-goals

- Exporting artboards (PNG, PDF, SVG) — later.
- Nested artboards, auto layout, constraints / responsive resizing of members, prototyping links.
- Live re-clipping preview of a shape while it is being dragged in or out.
- User-defined presets (a later addition could read them from a library file).

## Anti-goals

- Losing shapes: deleting or moving an artboard must never leave members stranded or delete non-members; membership must survive save/load, undo/redo and copy/paste.
- Panning or zooming a page with many artboards and members becomes noticeably slower (clipping must not add per-frame allocation proportional to the number of shapes beyond what painting already does).
- An artboard's name becoming unreadable when zoomed out, or huge when zoomed in.
- Older files failing to open, or older builds silently dropping artboards.

## Constraints

- Core model and membership rules in `mindweave-core`, unit-tested and gpui-free; UI in `mindweave-desktop`.
- Reuse existing machinery where it fits: shape kinds (`TEXT_LIBRARY` + shape id, like topics), the ribbon's `action_button` / tool buttons and dividers, `update_session` for edits, the paint pipeline's content masks.
- Every new edit is one undo step; layout-like updates (membership recomputed on drop) are part of the same step as the move that caused them.

## Left to the builder

- Exact data structure for membership updates and how the move gesture carries members along.
- How clipping is threaded through shape, text and picture painting.
- Ribbon overflow details, icon accent shapes, the name label's exact placement and hit area.
- Whether the preset table lives in a Rust constant or an embedded TOML file.

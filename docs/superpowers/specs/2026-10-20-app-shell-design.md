# Mindweave App Shell — Design

Date: 2026-10-20
Status: Draft for approval (builder-checked)

## Intent

**Mindweave** is a native desktop diagramming application written in Rust on gpui (the UI framework behind the Zed editor). Long term it combines three kinds of tool in one canvas-based app: diagramming (draw.io), mind mapping (XMind) and interface design (Figma), all with a modern feel.

This document covers only the first milestone: the **application shell**. Features (shapes, connectors, mind-map nodes, selection, etc.) will be designed and built one at a time afterwards, each through its own design. The shell must therefore be a clean foundation that later features slot into, not a demo.

## Look and feel (user's requirement)

- Must feel like Zed: minimal, neat, clean, dense. Small text (~12–13px UI), small monochrome icons, hairline 1px borders between panels, subtle hover states, no decoration, no large icons, no gradients, no drop-shadow-heavy cards.
- The mockup the user approved (ignore its "Show grid" toggle and "Background" field — the Background pattern selector below supersedes them) is at `.superpowers/brainstorm/*/content/shell-layout.html` (One Dark palette). Treat it as the layout reference; the real app should be at least that restrained.

## Goals

1. **Dashboard** shown on launch (Figma-style file browser):
   - Left nav: Recent (default), plus room for later entries.
   - Main area: grid of recently opened files (name, last-edited time, simple placeholder thumbnail), and a "New file" tile.
   - "New file" opens a native save dialog and creates the file on disk, then opens it in a new editor window.
   - **User's decision — one window per document (Zed-style).** The dashboard is its own window. Each opened file gets its own editor window; opening a file that is already open focuses its window. Closing an editor window prompts if unsaved; quitting prompts for every unsaved window. When the last editor window closes, the dashboard is shown (re-opened if it was closed).
   - An "Open…" action (menu + Cmd+O) opens an existing file via native open dialog.
   - Recent list persists between launches; entries whose file no longer exists are hidden or shown as missing.
2. **Editor** shown when a file is open:
   - **Menu bar** (File, Edit, View, Arrange, Help). On macOS the menus live only in the system menu bar; the window keeps a slim title bar (traffic lights, drag area, centred file name). Other platforms draw the menus in that title bar. Only items that work in this milestone are enabled (New, Open, Save, Close, theme, toggle sidebars, zoom); the rest may be present but disabled.
   - **Left sidebar — Layers**: list of layers for the current page; "+" adds a layer; each row has an eye toggle to show/hide; one layer is active (highlighted). Renaming by double-click is desirable.
   - **Centre — Infinite canvas**: pans (trackpad scroll / space+drag / middle-drag) and zooms (pinch / Cmd+scroll / Cmd+= / Cmd+-), draws its background pattern. No shapes yet.
   - **Page tabs** at the bottom of the canvas, spreadsheet-style: switch pages, "+" adds a page, rename by double-click. Each page has its own layers and viewport.
   - **Right sidebar — Properties**: in this milestone shows *Canvas* properties:
     - **Background** colour (swatch + hex, reset to theme), always shown.
     - **Background pattern** dropdown: Lines / Dots / None, with settings for the chosen pattern — Lines: grid size, line colour, major line every N cells (0 = off); Dots: spacing, dot size, dot colour; None: nothing more. All per page and saved in the file.
     - Leave a clearly separated, empty "Selection" area — later it will show selected-object properties.
   - **Status bar**: current page (e.g. "Page 1 of 3"), active layer, cursor canvas coordinates, zoom %, and a theme selector.
   - Both sidebars can be toggled (View menu + shortcuts) as in Zed.
3. **Themes**:
   - Ship at least: a One Dark–style theme (default), one other dark theme, one light theme.
   - Selectable from the status-bar theme selector and from a Zed-style theme picker (Cmd+K Cmd+T, fuzzy list with live preview).
   - The choice persists across launches.
4. **Files on disk (user's requirement)**: each document is a real file the user can find in Finder and put in git.
   - One file per document, extension **`.mindweave`** (user's decision), human-readable pretty-printed JSON containing: format version, pages (name, viewport, active layer, canvas settings), layers per page (name, visible), active page. Leave an obviously extensible place for objects.
   - **My call:** background pattern and grid size are **per page** (like Figma's per-page background); the Properties panel edits the current page.
   - **My call:** viewport, active page and active layer are written to the file on every save but **do not** mark the document dirty on their own (no unsaved marker or close prompt from merely panning/zooming).
   - Save with Cmd+S; window title shows file name and an unsaved-changes marker.

## Non-goals (this milestone)

- Any drawable objects: shapes, text, connectors, images, mind-map nodes, frames.
- Selection, undo/redo of canvas content (undo infrastructure may be stubbed), copy/paste, export.
- Collaboration, cloud storage, accounts, file thumbnails rendered from content.
- Windows and Linux support (code should not needlessly block them, but only macOS is targeted and tested now).
- Tabs for multiple documents in one window.

## Anti-goals

- A UI that looks like a generic web/Electron app: big icons, chunky buttons, rounded "card" everywhere, bright accents.
- A throwaway demo whose structure must be rewritten when the first real feature arrives.

## Constraints and decisions

- Language: Rust. UI: gpui. **User's decision:** use `gpui-component` for widgets (buttons, inputs, menus, switches, lists, dock/resizable panels, theming) where it fits.
- **My call:** use whichever gpui version/source `gpui-component` requires, pinned exactly (git rev or crates version) so builds are reproducible.
- Product name **Mindweave** (user's decision). Keep the existing CI's names: package `mindweave-desktop`, binary `mindweave`.
- **User's decision:** CI runs on **macOS only** for this milestone. Edit `.github/workflows/ci.yml` to drop the Windows and Ubuntu matrix entries and the Linux dependency step; keep fmt, `clippy -D warnings`, tests, release build and artifact upload.
- `.gitignore` currently contains `*.mindweave`; **remove that line**, since `.mindweave` is now the user document format and must be committable.
- **My call:** app settings (theme, recent files, window sizes) stored as JSON in `~/Library/Application Support/Mindweave/`.
- **My call:** in-window menu bar as in the mockup; also register the native macOS application menu with the same actions so standard shortcuts work.
- Target: macOS first.

## Success looks like

- `cargo run -p mindweave-desktop` opens the dashboard; New file → save dialog → a new editor window opens on an empty canvas with one page and one layer.
- CI (macOS) is green: fmt, clippy with warnings denied, tests, release build.
- User can add/rename pages and layers, toggle layer visibility, change background pattern and grid size, pan and zoom smoothly, save, quit, relaunch, and find the file in Recent with everything restored.
- Theme switching is instant and remembered.
- Side by side with Zed, it looks like it belongs in the same family.

## Left to the builder

- Module/crate layout, state management pattern, action/keybinding naming.
- Exact colours of the additional themes (keep Zed's restraint).
- Icon set (small, monochrome, line icons — e.g. the Lucide set bundled with gpui-component).
- Canvas rendering approach for the pattern and pan/zoom, as long as it stays smooth at any zoom (pattern should fade or adapt density when very zoomed out).

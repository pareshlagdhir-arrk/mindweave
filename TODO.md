# TODO list

### Must have features

- [ ] reduce the app size
- [ ] shapes are not connected
- [ ] settings should be graphical when possible

### Next Milestone

---

### Claude Milestone

#### Milestone 2: Paths and library authoring

Part A: Paths (the Home tools that were deferred)

1. Path model in the core: a free path made of corner points and smooth points, with curve handles, open or closed. It is saved in the document and gets fill, stroke and hit-testing like any other shape.
2. Pen tool (P):
   - Click to place a corner point, or click and drag to place a smooth curve point.
   - Clicking the first point closes the path; Enter or Escape finishes it.
   - Shift snaps the angle to 45°.
   - Each finished path is one undo step.
3. Point edit mode (double-click a path, or the Point edit tool):
   - Shows anchor points and curve handles.
   - You can drag points and handles, click an edge to add a point, and press Delete to remove a point.
   - You can switch a point between corner and smooth.
4. Lasso inside point edit: drag a freehand loop to select many points, then move them together. Shift adds points to the selection.
5. Convert to path: point-editing a library shape (rectangle, star and so on) turns it into a free path, with rotation and flips applied permanently to its points. Its fill and stroke are kept, and its shape-specific settings are dropped. Arrange > "Convert to Path" does the same.

Part B: Library authoring, the Figma and draw.io approach (the fix)

6. Resize behaviour for library shapes, set visually:
   - Lock proportions.
   - Figma-style pins (fixed size or stretch on each axis).
   - Stretch zones (9-slice), so corners and edges keep their size and only the middle stretches.
   - These are stored in the library format, applied when shapes are drawn, and fix the distortion problem for SVG shapes.
7. "Add to Library…": select one or more shapes or paths, choose "Add to Library…", pick an existing user library or create a new one, then name the shape and choose its icon. Mindweave writes the SVG and the TOML entry to ~/Library/Application Support/Mindweave/libraries/, and the new shape appears in the toolbar straight away.
8. Library shape editor: a small panel for an added shape:
   - name;
   - default size and style;
   - lock proportions and pins;
   - stretch zones, drawn as guides you drag on the shape.
9. Manage user libraries: rename, delete or reorder shapes and libraries, and reveal the folder in Finder so it can be shared. Changes reload without restarting the app.
10. Contributor guide update: docs/libraries.md will describe the "draw it, add it, share the folder" workflow, the resize and stretch-zone fields, and the existing hand-editing reference.

Part C: Wrap-up

11. Edit menu and properties: the sidebar shows path properties such as open or closed and point count, and Edit gains Convert to Path. Undo covers every new action.
12. Final checks:
    - CI;
    - speed with 2,000 paths;
    - a hand-test list for you;
    - a whole-branch review.

Not in this milestone:

- adjustable settings on contributed shapes (they stay on the built-in shapes, as you chose);
- formulas;
- text (milestone 3);
- snapping (milestone 4).

Open questions to settle when we write the design:

- Should several shapes added together become one library shape that keeps each part's own colours, or be merged into a single outline?
- Should the toolbar get a + button for "Add to Library", or only the Edit or right-click menu?
- Should adding to a built-in library like Basic Shapes be allowed? I would suggest not: user libraries only.

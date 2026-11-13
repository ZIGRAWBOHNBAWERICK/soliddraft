# SolidDraft interface design

The release interface is Edit Suite, approved in the L00 merged prototype on
13 November 2026. It combines B3's frame, sketch mode and tool palette with A2's
right-click ring, navigation bar and scene gallery. This guide records that target.
It does not claim that every panel below has shipped.

The internal `frame_layout` setting selects `classic` or `edit_suite`. Classic
remains the default during the rebuild. It retains its ribbon, docked tree and
Properties panel, light and dark Studio palettes, and saved panel layout. New frame
work belongs behind `edit_suite` until the release switches the default.

## Frame

One frame holds parts, assemblies and drawings. At the 1440 by 900 reference size,
the title bar is 34 px tall, the options bar below it is 46 px, and the tool palette
is 250 px wide at the left. The viewport fills the remaining space. A 58 px timeline
sits above a 30 px command line at the bottom, both to the right of the palette.
These measurements exclude the prototype's guide strip, which is review furniture.

The title bar holds menus at the left, workspaces in the centre and open document
tabs at the right. It has no search field. There is no Surface workspace.

| Document | Workspaces | Bottom strip | Tree and Values pane |
|---|---|---|---|
| Part | Solid, Sketch, Assembly, Inspect, View | Feature history | Hidden by default |
| Assembly | Assembly, Inspect, View | Components and mates | Open by default |
| Drawing | Views, Annotate, Sheet, View | Sheets and the active sheet's views | Open by default |

The palette exposes named commands in two columns. Every tool remains reachable by
scrolling at shorter window heights. Keep the name visible beside its icon. Smart
Dimension, Move entities, Copy entities, Rotate entities and Scale entities are
visible tools. Do not hide sketch tools inside Modify or Transform dropdowns.
Unavailable commands say "Coming soon" in the public UI, including their tooltips.
Do not show implementation details or ticket numbers there.

## Options and forms

The options bar follows the active tool, selection or feature editor. With nothing
selected, it holds pick filters and view controls. Display style belongs on the
viewport navigation bar.

A feature form puts model picks and its main size on the bar. OK and Cancel stay at
the right. Enter accepts and Escape cancels. A More button with a count opens a
sheet below the bar for the remaining fields. The sheet never contains the only
way to finish the operation. Short forms such as Extrude and Fillet need no sheet.
Fillet asks for edges and radius; corner treatment belongs to the kernel.

Use the same bar and sheet pattern for Mate, drawing sheet properties, Repair and
plane picking. Start sketch offers the three origin planes in the viewport and on
the bar, and accepts a flat face. Choosing a sketch tool outside a sketch first
asks for its plane and then arms that tool.

Tree and Values share a 236 px pane between the palette and viewport. Open it from
the View workspace, View menu or Tree button beside the timeline. There is no
separate Properties pane in Edit Suite. The tree holds structure, origin planes,
visibility and the Material row. Values holds named inputs and expressions.

## Theme tokens

`crates/soliddraft-app/src/theme.rs` is the source of shared colours, bundled fonts
and egui styling. `theme::edit_suite` contains the approved prototype values.
`theme::palette()` and the existing accessors choose that palette for Edit Suite.
`theme::LIGHT` and `theme::DARK` remain the classic palettes.

Edit Suite's charcoal chrome and amber accent do not change with the saved light
or dark appearance preference. The preference is retained for Classic. Viewport
scenes are a separate choice; a dark frame does not imply a dark modelling space.

| Role | Token | Value |
|---|---|---|
| Options bar and command line | `PALETTE.chrome` | `#2a2c30` |
| Title bar, timeline and recessed strips | `BAR`, `PALETTE.panel_header` | `#222427` |
| Palette and tree pane | `PALETTE.panel` | `#303338` |
| Frame separators | `PALETTE.line` | `#191a1d` |
| Strong borders | `PALETTE.line_strong` | `#474b52` |
| Primary text | `PALETTE.ink` | `#e6e8ec` |
| Secondary text | `PALETTE.muted` | `#9da2ab` |
| Active tool, focus and command prompt | `PALETTE.accent` | `#f5a524` |
| Selected fill | `PALETTE.selection_bg`, `PALETTE.accent_soft` | Amber at 20% |
| Selected text | `PALETTE.selection_ink` | `#f5a524` |
| Text on filled amber actions | `PALETTE.on_accent` | `#231600` |
| Hovered amber action | `PRIMARY_HOVER` | `#ffbc4d` |
| Fields and raised chips | `PALETTE.field_bg`, `PALETTE.chip` | `#3a3d43` |
| Chip and popup borders | `CHIP_LINE` | `#4a4e55` |
| Menus and dialogs | `POPUP` | `#35383e` |
| Hover wash | `POPUP_HOVER` | White at 10% |
| Viewport toolbar | `PALETTE.hud_bg` | `#2a2c30` at 95% |
| Successful state | `PALETTE.ok` | `#63d68f` |
| Destructive action and Close sketch | `DANGER`, `ON_DANGER` | `#e03a2e`, white |
| Error text on charcoal | `PALETTE.error` | `#ffb4ad` |
| AI session | `PALETTE.terminal_bg`, `PALETTE.terminal_ink` | `#17181b`, `#e6e9ee` |
| AI links and prompt | `PALETTE.terminal_accent` | `#f5a524` |
| Drawing surround | `PALETTE.canvas_bg` | `#565a62` |

The dark red action fill and pale red error text are separate roles. Use dark ink
on amber actions and white on red actions. Keep focus visible with an amber stroke.
Hover, focus and selection must remain distinguishable without changing a label.

Inter Regular, Medium and SemiBold are bundled. Use `theme::font`,
`theme::font_medium` and `theme::font_semibold`; egui's fonts supply missing symbols.
Use `theme::mono` for entered values, measurements and the command prompt.

| Type role | Size |
|---|---|
| Body | `BODY_SIZE`, 12.5 px |
| Palette tool label | `TOOL_LABEL_SIZE`, 11.5 px |
| Secondary label | `SMALL_SIZE`, 10.5 px |
| Numeric value and prompt | `VALUE_SIZE`, 12 px |
| Palette group caption | `GROUP_LABEL_SIZE`, 9 px |

The frame constants in `edit_suite` name title, options, palette, pane, timeline and
command-line dimensions. `TOOL_HEIGHT` is 23 px, `ROW_HEIGHT` is 25 px and
`CONTROL_HEIGHT` is 28 px. `ICON_SIZE` is 16 px. Palette columns have a 2 px gap,
tools have 5 px horizontal padding, and options groups have a 12 px gap. Allow a
wrapped label more height instead of clipping its name.

Use 6 px corners for ordinary controls and popups, 4 px for value fields, 5 px for
palette tools and 10 px for the floating navigation bar. These are `RADIUS`,
`FIELD_RADIUS`, `TOOL_RADIUS` and `FLOATING_RADIUS`. The classic constants at the
module root remain unchanged. New panels should consume these tokens rather than
copying prototype literals.

## Viewport scenes and navigation

The Scene gallery opens above the Scene button on the bottom-centred navigation
bar. It shows favourites as thumbnails. Manage favourites lists all nine scenes.
There is no second backdrop grid in the palette or swatch row on the options bar.
The empty-space context menu and command line also offer scenes.

The scenes are Plain white, Studio fade, 3 point faded, Soft spotlight, Graphite,
Night grid, 3 point beige, Blueprint and Lightbox studio. Their backgrounds live in
`viewport/scenes.rs`. Plain white is the default. A scene changes the backdrop,
not the frame palette or the geometric meaning of sketch colours.

The navigation bar holds orbit, pan, zoom, fit, display style, Scene, origin axes
and all dimensions. A drawing uses pan, zoom, fit, grid and snap instead. The view
cube remains at the upper right. View changes animate over about 150 ms.
Z is up; Front looks from -Y toward +Y, Top down -Z with +Y up, Right along -X,
and Isometric from +X, -Y, +Z.

Mouse gestures follow the saved navigation preset. A fresh installation uses
Fusion 360: middle drag pans and Shift plus middle drag orbits. The wheel zooms
toward the cursor. A click without a drag selects. Grid, reference visibility,
projection and rendering quality retain their existing preferences.

## Sketch mode

An open sketch locks the workspace switch to its sketch and plane. The palette
contains sketch tools only. A red Close sketch action occupies the right end of
the options bar. It stays visible while a tool or dimension editor is active.

Sketch geometry uses its own semantic tokens, independent of the amber interface:
`UNDER_DEFINED` is blue `#1d4fe0`, `FULLY_DEFINED` is black `#0b0d11`, and conflicts
are red. A dark scene places the sketch on `SKETCH_SHEET`, `#f1f3f7`, so black
remains black on every scene. Do not derive geometry's definition colours from
`theme::ink()` or `theme::accent()` when migrating the sketch renderer.

Smart Dimension chooses the measurement from the picks and placement. A line
placed above or below measures its horizontal extent; beside it, its vertical
extent; inside its bounds, its length. Circles give diameters, arcs give radii,
and two lines give an angle or parallel distance. A new driving dimension opens
its value editor. Reference dimensions show measured values without driving the
solver. Solver results determine definition and conflict colours.

Tools preview their result before committing. Escape cancels the unfinished step
before leaving a tool or sketch. Line and Centreline retain their existing chain
rules. Relations, dimensions, construction flags and undo remain document behavior;
changing the frame must not change them. See [modelling.md](modelling.md) for the
model and persistent-reference rules.

## Right-click and history

Right-click opens eight commands around the cursor and the full grouped list below.
A face offers Face, Feature and Body columns; an edge offers Edge, Feature and Body.
The menu follows the picked context and stays inside the window near an edge.
It uses the same charcoal, amber, text and popup tokens as the rest of the frame.

The timeline is history. Clicking a chip selects; double-clicking edits;
right-clicking opens its context menu. Clicking between chips moves the rollback
marker and displays that history position. An under-defined sketch carries a
`(-)` prefix. Suppression, missing references and failed or pending proofs retain
text explanations alongside their status marks. Visibility choices remain saved
with the document.

## Command line and assistant

The bottom line holds tool chips, a command prompt and document status. Tab
completes commands, Up and Down recall history, and a typed command runs the same
registry action as its button. Status details remain available when the line is
short of room.

Typing `ai` opens a session above the timeline. Its dark console uses the terminal
tokens. Preview, Apply, Discard and Undo retain their existing proposal semantics.
After 30 seconds idle the session folds into an `ai · 1 turn` chip; its countdown
stays visible. There is no docked Assistant pane or drawing Assistant workspace.

## Shared behavior and implementation boundaries

Menus, palette entries, shortcuts and the command line use the same command
registry. Preferences keeps Navigation and Interface shortcut settings shared
across document types. Unavailable settings categories say "Coming soon".
Keyboard focus and edited bindings remain visible in menus and shortcut help.

Icons come from `assets/icons` and `icons.rs`. The catalogue generator owns the
SVG files. Use registry icons or the shared painter instead of text glyphs.
Raster treatments follow the actual chrome brightness, including Edit Suite when
the retained appearance preference is Light.

The frame does not change the kernel boundary. The app talks through
`soliddraft_kernel_api::Kernel`; geometry, proof, persistent references, undo and
file formats keep their current contracts. Picking retains vertex, edge and face
priority. Rendering quality and display style remain separate from scenes.
See [kernel-bridge.md](kernel-bridge.md), [documents.md](documents.md),
[assemblies.md](assemblies.md), [materials.md](materials.md) and [web.md](web.md)
for those contracts. Drawings retain their model references, explicit repair and
vector export behavior while adopting the shared frame.

# UI and interaction parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (custom ruler origin, #1212; Artboard tool clicks fit the topmost editable object in one undo step (#1227); scores unchanged; Pathfinder single-result source names and SVG IDs retained, #1293; DXF control-point SPLINE closure read, #1299 follow-up; Document Setup: Edit Artboards closes the modal after applying valid settings; pointer-click regressions; scores and hours unchanged; Artboards: Escape exits editing; an active drag cancels first; shortcut-routing regression; scores and hours unchanged; Simplify preview/counts and valid rewritten-path selection (#1272); parity remains partial; Texture-effect drag redraws optimized (#1257); idle Windows interaction still unverified; scores unchanged; Curvature point display improved (#1236, in part); scores unchanged; Direct Selection: no-op anchor Align preserves Undo, Redo and saved state (#1180 follow-up); Eyedropper cursor distinguished from precise crosshair (#1269); scores unchanged; Eyedropper can read visible locked and outside-isolation sources (#1251); scores unchanged; Direct Selection: independent segment picks and Delete (#1278); scores unchanged; Adding paint to an unpainted zero-width stroke restores the 1 pt default (#1253); scores unchanged; macOS save sheets offer a Format menu, #866; Attributes fill-rule controls edit the compound when a member is selected, #1197; a marquee needs to touch a path, not just lie inside its fill, #1103; earlier: Live Corners round corners between curved sides, #1144) · **Target:** Adobe Illustrator 2026 (30.x)

How VectorCraft's tools, handles, snapping and panels behave next to Illustrator's. A vector illustrator's
trust is in the details: where a handle lands, what Shift and Alt do mid-drag, what a smart guide snaps to,
how far an arrow key nudges. This checklist is the work list for the **interaction-fidelity pass**
([gaps.md](gaps.md) G1). Part of [target-app-parity.md](target-app-parity.md).

**How it is measured:** from the tool state machines' own documentation and tests (`crates/tools/src/*.rs`,
`crates/ui-egui/src/canvas.rs`, `shortcuts.rs`, the `tests_*` files) against Illustrator's public documentation
(helpx tool pages and the keyboard-shortcut reference). Never against the running app: the clean-room rule
forbids launching or measuring Illustrator, so there has been **no side-by-side session**. Every "done" below is
"behaves as the documentation describes", which is weaker than "a power user can't tell". Status:
**done** (documented behaviour implemented and tested), **partial**, **missing**.

## Summary

| Area | % | Remaining (h) | Kind |
|---|---:|---:|---|
| Pen, Curvature and anchor tools | 85% | 6–10 | estimated |
| Direct Selection, handles and anchors | 85% | 5–8 | estimated |
| Smart guides and snapping | 70% | 8–12 | estimated |
| Transform handles, bounding box, Free Transform | 80% | 5–8 | estimated |
| Nudging, numeric entry, precision, units | 85% | 3–5 | estimated |
| Modifier keys and temporary tools | 80% | 5–8 | estimated |
| Keyboard shortcuts (default set) | 70% | 6–10 | estimated |
| Cursors | 75% | 3–5 | estimated |
| Panels, Control bar, Properties, context menus | 70% | 15–25 | estimated |
| Look and feel (layout, theme, metrics) | 80% | 5–10 | measured once against public screenshots on 2026-10-02; panels added since not re-measured |
| Accessibility (screen reader, keyboard-only canvas) | 20% | 15–25 | estimated: AccessKit on by default for egui widgets; canvas and custom panels not described, never tested with a screen reader |
| **UI/UX overall** | **~55%** | **~70–110** (overlaps the feature areas) | estimated |

The overall figure is lower than the rows because what users notice first (small differences across many
tools, never checked side by side) isn't captured row by row.

## Pen, Curvature and anchor tools

| Behaviour (Illustrator, public docs) | Status | Notes |
|---|---|---|
| Click: corner anchor; drag: smooth anchor with symmetric handles | done | `pen.rs` |
| Shift constrains segments and handles to 45° | done | Illustrator's Constrain Angle preference is honoured (`constrainAngle`) |
| Alt mid-drag splits the handles (only the outgoing follows) | done | keeps the curve already shaped |
| Space mid-drag repositions the anchor, handles and all | done | |
| Click the first anchor to close; the drag shapes the closing curve | done | |
| Click the last anchor to retract its outgoing handle; drag from it to pull a new one | done | |
| Alt over a handle, anchor or segment acts as the Anchor Point tool | done | |
| Cmd/Ctrl lends the last-used selection tool | done | the path being drawn continues afterwards |
| Click an open path's end to continue it; click another path's end to join | done | #776 |
| Rubber Band preview | done | Preferences option |
| Auto Add/Delete on a selected path; Shift or the preference disables it | done | |
| Pen cursors (new path ×, continue /, close o, add +, delete −, convert ^) | partial | vector cursors exist; not every state verified |
| Each anchor snaps to Smart Guides and construction angles | done | `DrawSnap` |
| Curvature tool: click/drag points, double-click or Alt-click for a corner, temporary Direct Selection, point conversion | partial | circular point markers without direction handles or duplicate markers, Anchor Size and Hide Edges regression-tested (#1236, in part); existing-path interpolation request and modifiers requested in #975 remain |
| Add/Delete/Anchor Point tools; Anchor Point tool drags segments to reshape | done | |
| Pencil: Fidelity, Keep selected, Alt to Smooth, edit selected paths, close by proximity | partial | Keep selected and Alt to Smooth are missing |
| Join tool scrubbing, Average, Join with corner/smooth choice | done | |
| Simplify (slider, auto, corner angle threshold) | partial | Tolerance, default-on Preview and original/new point counts (#1272); slider, auto-simplify, corner-angle UI and Show Original remain missing |

## Direct Selection, anchors and handles

| Behaviour | Status | Notes |
|---|---|---|
| Click/Shift-click anchors; marquee toggles with Shift | done | #483 |
| Align selected anchors to their bounds or the artboard; repeating an alignment that moves nothing keeps Undo, Redo and saved state | done | #1180 follow-up; engine regressions include rounding-only moves and unchanged handles |
| A marquee selects the objects whose paths it touches (anchors or segments): drawn inside a filled shape, over a background, it leaves that shape (Preview and Outline) | done | #1103 |
| Click a segment independently of its endpoint anchors; only its direction handles show | done | #1278: hollow endpoint squares, selected edge emphasized; headless egui checked |
| Drag a curved segment to bend it, a straight one to move its anchors | done | |
| Drag by an anchor of a path selected whole moves the whole path; Alt-drag by an anchor or a segment copies it, Alt toggles the copy during the drag, and the copy snaps by the anchor pressed, onto the original too | done | #1104 |
| Handles shown are the handles that drag; tolerance from Selection & Anchor Display | done | #494 |
| Drag a handle with Shift (45° about its anchor), Alt (move one handle alone) | done | handles snap to smart guides |
| Show handles when multiple anchors are selected; Highlight anchors on mouse over; anchor and handle size | done | Preferences › Selection & Anchor Display |
| Convert selected anchors to corner/smooth from the Control bar and Properties | done | |
| Cut path at selected anchors, Remove Anchor Points, Delete removes segments | done | #1278: segment Delete preserves neighboring edges and endpoint anchors; a second Delete removes the remainder; explicit anchor Delete retains its behavior |
| Live Corners widgets (radius readout, Alt-click cycles kind, double-click the Corners dialog) | done | clicking a widget without pre-selecting the anchor (#908) missing |
| Corners between curved sides (#1144), Relative rounding | partial | Relative rounding missing |
| Isolation mode by double-click, breadcrumbs, Esc to exit | done | |
| "Magnetic" moves of selected anchors with a modifier (#955 request) | missing | not an Illustrator feature as documented; under discussion |

## Smart guides and snapping

| Behaviour | Status | Notes |
|---|---|---|
| Snap to anchors, centres, edges, artboards, bleed and ruler guides with magenta lines | done | `guides.rs` kinds: Anchor, Center, Edge, Artboard, Bleed, Guide |
| Anchor/Path labels ("anchor", "center", "path", "align") | done | |
| "intersect" label and snapping to path intersections | missing | |
| Construction guides at preference angles while drawing | done | #506 |
| Measurement labels while drawing, moving and resizing | partial | sizes and offsets; angle and distance readouts while drawing missing |
| Spacing guides (equal distances) | partial | for moves (#394); not for resizing or drawing |
| Snapping tolerance preference | done | |
| Snap to Point, Snap to Grid, Snap to Pixel; Pixel Preview | done | |
| Snap to Glyph | missing | menu stub |
| Tangent and perpendicular snapping for line endpoints (new in 30.0) | missing | |
| Snapping panel/popover consolidating the snap options (30.0) | missing | |
| Customisable rotation snap angle (#973) | missing | Illustrator: Constrain Angle and construction angles only |
| Ruler guides: drag out, select, move, delete, lock, Make/Release Guides | done | #414, #451 |
| Ruler origin: corner drag with preview, double-click reset to the active artboard, relative X/Y fields, native save/reopen | done | #1212; `document.setRulerOrigin`, `document.resetRulerOrigin`; geometry and command coordinates stay absolute |
| Ruler guides as Layers rows: shown, hidden, locked and deleted with their layer; rows dragged to another layer | done | `guide.setLayer`; guides from older files go on a layer |
| Alt swaps a ruler guide's orientation while dragging | missing | |
| Global/artboard ruler mode switch and video rulers | missing | menu stubs; custom document origin is done (#1212) |

## Transform handles, bounding box, Free Transform

| Behaviour | Status | Notes |
|---|---|---|
| Moving a rounded rectangle with Craquelure at high DPI | partial | Distance searches optimized; moved-viewport pixels unchanged for three texture effects; `texture_drag_perf` probe available. Idle Windows mouse-drag QA remains (#1257) |
| Bounding-box handles: Shift proportional, Alt from centre, rotate outside the corners | done | |
| Rotated objects keep a rotated bounding box and their angle; Reset Bounding Box | done | |
| Free Transform, Scale and Reflect along a rotated box | missing | |
| Free Transform widget: constrain, free distort, perspective distort; the key-held variants | done | |
| Transform tools: click sets the reference point (snaps to anchors/centres), Alt-click opens the dialog, Alt-release copies | done | |
| Transform Again, Transform Each, Scale Strokes & Effects, Scale Corners | done | |
| Reference point locator (nine squares) in Control bar and Transform panel | done | |

## Nudging, numeric entry and precision

| Behaviour | Status | Notes |
|---|---|---|
| Arrow keys nudge by Keyboard Increment (0.001–1296 pt); Shift ×10; Alt copies | done | |
| Number fields: Up/Down step, Shift ×10, Cmd/Ctrl ÷10, wheel over a focused field | done | Shift+arrow in the Stroke weight field doesn't step by 10 (#991) |
| Arithmetic in fields (`+ - * /`) and unit suffixes (`3 mm`, `1in`, `2p6`) | done | `Unit::parse`; mixed units in one expression not verified |
| Numeric fields: focus click selects the whole value; dragging selects from the pressed character | partial | #1326; select-all waits for click release; px/pt/plain/optional fields, both directions and replacement text tested; native API input, 17 px partial replacement, Undo/Redo, saved-file round trip and screenshots verified; Tab, double-click, arrows and wheel preserved; short movements still classified as clicks can select all and remain open |
| Units: pt, pc, in, mm, cm, px, ft-in, m, yd, ft; General/Stroke/Type units | done | units preference reverting to points on SVG output reported (#864) |
| Field precision: 3 decimals for steps; values kept as f64 | done | |
| Scrubby labels (beyond Illustrator) | done | #400 |
| Dialogs open with the first field focused and selected; Enter confirms | done | |

## Modifier keys and temporary tools

| Behaviour | Status | Notes |
|---|---|---|
| Space: Hand; Cmd+Space / Cmd+Alt+Space: zoom in/out; middle button pans | done | |
| Cmd/Ctrl: last-used selection tool; Cmd+Alt: Group Selection inside Direct Selection | partial | |
| Eyedropper reads visible sources while their layer/object is locked or outside isolation | done | Ordinary/Shift/Shift+Alt and gradient-stop reads preserve the source and selection; Alt reverse remains editable-only; hidden art stays excluded (`eyedropper_sources.rs`, #1251) |
| Add a visible stroke after drawing with inherited None/zero-width paint | done | Paint and Swap Fill/Stroke restore the existing 1 pt default; positive widths and painted zero widths stay as set; Undo/Redo and targeted appearance items tested (`stroke_visibility.rs`, #1253) |
| Alt-drag duplicates (objects, artboards with their art) | done | |
| Document Setup → Edit Artboards applies valid settings, closes the dialog and activates the Artboard tool | done | Pointer-click tests cover successful handoff, invalid settings, OK, Cancel and Undo in `dialogs/document_setup.rs`. |
| Escape leaves Artboard editing for Selection; during a drag it first cancels the gesture | done | Headless egui key events cover idle Escape, active-drag cancellation and a subsequent Escape in `tests_distortkeys.rs`. |
| Shift-click selects several artboards (Artboard tool, Artboards panel); they move, Alt-copy and delete together | done | `artboard.move`/`artboard.delete` `indices` (#1139) |
| Click an object with the Artboard tool to create an artboard fitted to its visible bounds | done | Topmost editable object or group; drag and modified clicks retain their meaning; one undo step (`artboard_click.rs`, #1227) |
| Shape tools: Shift constrains, Alt from centre, Space moves while dragging, arrows change sides/points/rows | done | |
| Tool double-click opens its options | done | |
| Per-platform mapping (Cmd ↔ Ctrl, Option ↔ Alt) | done | |
| Eyedropper cursor; Use Precise Cursors | partial | original pipette at the sampling tip, distinct from the precise crosshair at five DPI scales (#1269); OS cursor display and Caps Lock unverified |
| Every tool's modifiers checked against the shortcut reference | missing | this is the fidelity pass |

## Keyboard shortcuts

~110 distinct modified shortcuts on menu commands plus single-key tool shortcuts for the default toolbar
(`crates/tools/src/catalog.rs`), a shortcut editor with chords, and saved sets. Not yet diffed item by item
against Illustrator's published default shortcut list; estimated ~70% of it. Remaining: the panel shortcuts,
the type shortcuts (tracking/kerning/leading/baseline steps with Alt/Shift), and selection shortcuts in the
Layers panel.

## Panels, Control bar, Properties, context menus

| Behaviour | Status | Notes |
|---|---|---|
| ~40 panels with ≡ menus; dock, icon column, collapse, tabs | done | 51 panel modules incl. tests |
| Panels float inside the window, stack, dock back | done | #495 |
| Floating groups stack into sets; sets dock as extra dock columns (a multi-column dock); groups drop anywhere in a set or column, or onto its tabs; a column taller than the window scrolls | done | `window.panel.float {below, above}`, `window.panel.dock {column}` |
| Panels outside the app window (another monitor) | missing | single OS window |
| Edit Toolbar (•••): the All Tools drawer opens beside the toolbar by its button (Escape or a click outside closes it), dims the tools already in the toolbar; while it is open, tools drag from it onto the toolbar (between two tools, or onto one to join its group), from the toolbar into it, and within the toolbar; each toolbar keeps its edits | done | #1018; `window.editToolbar`, `toolbar.addTool`/`removeTool`/`reset`; the drawer's Reset (Illustrator has it in a drawer menu); dragging a toolbar button moves the tool it shows, not its whole group; New/Manage Toolbars missing |
| Properties panel per context (every selection type's Quick Actions) | partial | |
| Contextual task bar | done | |
| Control bar with Transform and Stroke popovers | done | |
| Pathfinder shape results keep their source names and SVG IDs | done | #1293; Minus Front keeps the back name, Minus Back the front name; Unite/Intersect/Exclude use the backmost named input; compound output carries it once; geometry and paint unchanged |
| Attributes fill-rule controls with a compound member selected | done | Edits the compound's rule; direction still edits only the member; rendered-hole and Undo/Redo regression tests (#1197) |
| Pathfinder Divide: an open cutting line splits a filled path | done | #1307; straight, crossed, bent and curved cutters, holes and frontmost appearance regression-tested; native automation: independent piece movement, one Undo/Redo, saved-document round trip and window capture; existing bounded-map fallback remains |
| Canvas context menu | done | Layers rows, panels and type have none (gap) |
| Blend panel (30.7) | missing | Blend Options dialog exists |
| SVG Interactivity, Retype, Comments, Version History panels | missing | the last two are cloud features (out of scope) |
| Workspaces (save, reset, switch) | done | |
| DXF export/import preserves intended curve closure | done | #1299; R13–2018 closed SPLINE bit set/read; an open curve ending at its start stays open; knots and control points unchanged; native CIRCLE/ARC export and CNC preset remain pending |
| macOS save sheet: choose the format from a menu | done | #866; `apps/vectorcraft/src/mac_save_format.rs`; current suffix selects the initial format, changing it updates the filename; native SVG save and cancel after changing to PDF checked |
| Preferences: every option acts | partial | ~16 stored but unused (#394) |

## Look and feel

Medium Dark theme and four brightness levels, the categorized and Advanced toolbars, 35 pt document tabs,
33 pt panel tabs, hint bar, contextual task bar and native macOS menus, restyled to values measured from
public screenshots on 2026-10-02 (~75–80% then). Missing: a System theme that follows the OS (#825), a
re-measure of the panels added since.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | Custom ruler origin (#1212): corner drag previews and sets it; double-click resets to the active artboard; X/Y fields use it and native files keep it; scores unchanged (estimated) |
| 2026-10-12 | minor | Artboard tool clicks fit the topmost editable object in one undo step (#1227); scores unchanged |
| 2026-10-12 | minor | Pathfinder single-result booleans preserve source names and SVG IDs (#1293); paint-order, grouped/compound sources, unchanged geometry/appearance and Undo/Redo regression tests; scores and estimates unchanged |
| 2026-10-12 | minor | DXF reads control-point SPLINE closure (#1299 follow-up after #1320); open curves stay open and curves retained; rational/non-rational import and export versions regression-tested; scores/hours unchanged |
| 2026-10-12 | minor | Document Setup: Edit Artboards closes the modal after applying valid settings; pointer-click regressions; scores and hours unchanged |
| 2026-10-12 | minor | Artboards: Escape exits editing; an active drag cancels first; shortcut-routing regression; scores and hours unchanged |
| 2026-10-12 | minor | Simplify previews and shows original/new point counts; refits/subdivisions select changed paths whole (#1272); auto/slider/corner-angle UI and Show Original remain partial |
| 2026-10-12 | minor | Texture-effect drag redraws optimized (#1257); idle Windows interaction still unverified; scores unchanged |
| 2026-10-12 | minor | Curvature: circular point markers, no direction handles or duplicate points; existing-path interpolation and #975 remain separate (#1236, in part); scores unchanged (estimated) |
| 2026-10-12 | minor | Direct Selection: repeating anchor Align and rounding-only Align leave Undo, Redo and saved state intact (#1180 follow-up); scores unchanged (estimated) |
| 2026-10-12 | minor | Eyedropper: original pipette at the sampling tip; precise setting and DPI checked (#1269); OS visual QA and Caps Lock unverified; scores unchanged |
| 2026-10-12 | minor | Eyedropper can read visible locked and outside-isolation sources (#1251); scores unchanged |
| 2026-10-12 | minor | Direct Selection: segment click/display and Delete preserve neighboring edges (#1278); pointer/egui/native checks, OS visual QA pending; score unchanged |
| 2026-10-12 | minor | Adding paint to an unpainted zero-width stroke restores the 1 pt default (#1253); scores unchanged |
| 2026-10-12 | minor | macOS native save sheets gain a localised Format menu (#866); filename regression tests and an actual SVG save, PDF selection and cancellation verified; scores unchanged (estimated) |
| 2026-10-12 | minor | Attributes: directly selected compound members read and change the compound fill rule; path direction stays member-local (#1197); score unchanged |
| 2026-10-12 | minor | Pathfinder Divide: open cutting lines split filled paths with one Undo (#1307); scores and hours unchanged |
| 2026-10-12 | minor | Numeric entry: click-to-focus selection waits for release so text drags keep their start (#1326, in part); keyboard focus, double-click, arrows and wheel preserved; scores and hours unchanged; short movements below the click threshold remain open |
| 2026-10-11 | minor | A marquee inside a filled shape no longer selects it: only touching its path does (#1103); score unchanged (estimated) |
| 2026-10-11 | minor | Live Corners: corners between curved sides round too, the curve trimmed to what the cut leaves (#1144); score unchanged (estimated) |
| 2026-10-11 | minor | Artboards: Shift-click selects several, which move, copy and delete together (#1139); score unchanged (estimated) |
| 2026-10-11 | minor | Edit Toolbar opens the All Tools drawer beside the toolbar instead of a centred dialog, dims the tools the toolbar holds, and edits the toolbar by drag and drop while it is open (#1018); score unchanged (estimated) |
| 2026-10-11 | minor | Direct Selection: dragging a path selected whole by an anchor moves it whole; Alt-drag copies by an anchor too, toggles during the drag and snaps onto the original (#1104); score unchanged (estimated) |
| 2026-10-10 | major | First checklist, built from the tool sources and tests against Illustrator 2026's public documentation |

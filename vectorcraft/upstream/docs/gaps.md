# Gaps

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (G7: custom ruler origin done, #1212; G1: Artboard object clicks create a fitted artboard (#1227); remaining interaction pass and estimate unchanged; Pathfinder single-result source names and SVG IDs retained, #1293; DXF control-point SPLINE closure read, #1299 follow-up; G16: Korean catalog added, 70–110 → 60–100 h; Czech render-site localization follow-up; scores and estimates unchanged; Document Setup: Edit Artboards closes the modal after applying valid settings; pointer-click regressions; scores and hours unchanged; Artboards: Escape exits editing; an active drag cancels first; shortcut-routing regression; scores and hours unchanged; G11: valid selection after Simplify and Add Anchor Points (#1272); estimates unchanged; G2: Film Grain added; Photoshop effects 29/57; 28 filters remain; G10: RPM runtime dependency metadata (#1275); estimates unchanged; G15: texture-effect distance searches optimized (#1257); idle-machine/platform drag QA still pending; estimate unchanged; G1: Curvature point display improved (#1236, in part); estimates unchanged; G4: full-name font matching (#1258); Windows font-service QA pending; estimate unchanged; G11: no-op anchor Align keeps history and saved state (#1180 follow-up); estimates unchanged; G1: dedicated Eyedropper cursor and precise-cursor checks (#1269); estimates unchanged; G1: Eyedropper reads visible locked and outside-isolation sources (#1251); estimate unchanged; G11: Direct Selection segment Delete (#1278); estimates unchanged; G1: painted strokes regain the existing default after None/zero-width inheritance (#1253); estimate unchanged; G15: large-selection edits, cached live effects and faster Pathfinder, #1173; G4: proportional vertical metrics done, #966; G11: Shape Builder merge seams on grids of lines fixed, #893; scaled text controls corrected, #1034; group font-size preservation deferred) · **Target:** Adobe Illustrator 2026 (30.x)

Every known shortfall against Illustrator, one entry each, **ranked by how much it stops a professional from
switching**. This is the work list: unless you were given a task, pick the highest-ranked gap you can make
progress on, write a plan, and update the entry (and [target-app-parity.md](target-app-parity.md)) in the same PR.

Gap ids are stable (PRs say "Part of gap 2"); the rank can change. Hours are Opus 5.5 agent wall-clock hours for
one agent, estimated unless marked; calibration in
[target-app-parity.md](target-app-parity.md#calibration). **Beta** marks the gaps that must close before the beta
stage ([ROADMAP.md](../ROADMAP.md)).

## Ranked

| Rank | Gap | Hours | Beta | Doc |
|---:|---|---:|:---:|---|
| 1 | [G1 Interaction fidelity](#g1-interaction-fidelity) | 60–90 | yes | [ui-parity.md](ui-parity.md) |
| 2 | [G9 Illustrator `.ai` and EPS files from real users](#g9-illustrator-ai-and-eps-files-from-real-users) | 10–20 | yes | [file-format-parity.md](file-format-parity.md) |
| 3 | [G10 Launch and stability on real machines](#g10-launch-and-stability-on-real-machines) | 20–35 | yes | — |
| 4 | [G8 Hardening at scale](#g8-hardening-at-scale) | 30–50 | yes (part) | — |
| 5 | [G11 Correctness bugs in core tools](#g11-correctness-bugs-in-core-tools) | 8–15 | yes | [target-app-parity.md](target-app-parity.md) |
| 6 | [G4 Advanced type](#g4-advanced-type) | 34–47 | core part | [type-parity.md](type-parity.md) |
| 7 | [G12 Preferences and panels that don't act yet](#g12-preferences-and-panels-that-dont-act-yet) | 13–20 | yes | [ui-parity.md](ui-parity.md) |
| 8 | [G2 Photoshop-style raster effects and the Effect Gallery](#g2-photoshop-style-raster-effects-and-the-effect-gallery) | 23–39 | no | [effects-parity.md](effects-parity.md) |
| 9 | [G13 Illustrator 2026 (30.x) additions](#g13-illustrator-2026-30x-additions) | 15–25 | no | [target-app-parity.md](target-app-parity.md) |
| 10 | [G14 Pen hardware beyond Windows pressure](#g14-pen-hardware-beyond-windows-pressure) | 8–16 | macOS pressure | [hardware-parity.md](hardware-parity.md) |
| 11 | [G5 Brushes, symbols and libraries](#g5-brushes-symbols-and-libraries) | 17–28 | no | [target-app-parity.md](target-app-parity.md) |
| 12 | [G3 3D and Materials](#g3-3d-and-materials) | 45–75 | no | [effects-parity.md](effects-parity.md) |
| 13 | [G7 Views and windows](#g7-views-and-windows) | 15–25 | no | [ui-parity.md](ui-parity.md) |
| 14 | [G6 Automation and scripting](#g6-automation-and-scripting) | 8–14 | no | [target-app-parity.md](target-app-parity.md) |
| 15 | [G15 Performance budgets and a GPU renderer](#g15-performance-budgets-and-a-gpu-renderer) | 15–25 | budgets only | [hardware-parity.md](hardware-parity.md) |
| 16 | [G16 Localization: the four missing key languages](#g16-localization-the-four-missing-key-languages) | 60–100 | no | [localization-parity.md](localization-parity.md) |
| 17 | [G17 Ecosystem: third-party plug-ins](#g17-ecosystem-third-party-plug-ins) | 20–45 | no | — |
| 18 | [G18 Generative AI features](#g18-generative-ai-features) | 30–60 | no | — |
| 19 | [G19 Accessibility](#g19-accessibility) | 15–25 | no | [ui-parity.md](ui-parity.md) |

**Alpha blockers:** none. Every core workflow passes the [alpha gate](roadmap.md#alpha-gate); G9 is the only
partial one, and it blocks beta, not alpha.

**To beta:** G1, G9, G10, G11, G12, the corpus and QA part of G8, the type-core part of G4, macOS pen pressure
from G14 and the budget run of G15: **~170–270 h** one agent, ~50–80 h wall clock with 4–6 agents.

## G1 Interaction fidelity

- **Done (#1227):** object clicks with the Artboard tool create a fitted artboard, preserving art and selection; handles, modified clicks and drags retain their meaning. Covered by tool-event regression tests and undo/cancellation checks.
- **Done:** Document Setup → Edit Artboards applies valid settings, closes the dialog and activates the Artboard tool. Pointer-click tests cover successful handoff, invalid settings, OK, Cancel and Undo in `dialogs/document_setup.rs`.

- **Improved:** Curvature uses circular point markers without direction handles or duplicate markers
  (#1236, in part); real egui frames cover creation, existing paths, Anchor Size and Hide Edges.
  The request to change interpolation of existing paths remains open.
- **Improved:** Eyedropper has an original pipette cursor at its sampling tip; Use Precise
  Cursors keeps the crosshair. Bitmap/DPI and preference checks cover the distinction (#1269).
  OS cursor display and Caps Lock remain unverified.
- **Done (#1251):** Eyedropper source reads are independent of locks and isolation, including Shift/Shift+Alt and gradient stops. Source and selection preservation, unchanged Alt reverse/selection restrictions, hidden art and Undo/Redo have regression coverage. This addresses the reported behavior, with no new full-parity measurement.
- **Done (#1253):** adding paint to an unpainted zero-width stroke restores the existing 1 pt default, including new-art defaults and Swap Fill/Stroke. The real Eyedropper inheritance sequence, targeted appearance items, preserved widths and Undo/Redo have regression coverage.
- **Missing:** a tool-by-tool, panel-by-panel pass over modifiers, cursors, the Properties panel per context,
  isolation mode and small behaviours, against the public tool and shortcut documentation. Itemized in
  [ui-parity.md](ui-parity.md). The Shaper's construction-mode widget and gesture refinements remain.
- **Evidence:** no side-by-side session has ever happened (the clean-room rule forbids running Illustrator, so
  the pass works from the documentation); users keep reporting small differences: #991, #975, #973, #955, #908.
- **Impact:** a power user notices this in the first minutes and stops trusting the tool.
- **Estimate:** 60–90 h; splits well by tool across agents.

## G9 Illustrator `.ai` and EPS files from real users

- **Missing:** faithful layer and group structure from the editing data in real files (#951: 109 layers instead
  of 14), guides and non-printing construction
  lines (#779), open speed on large files (#758), legacy type in `.ai` files in PostScript form (those without
  their prolog open since #1027), live effects, brushes and symbols as live objects (#637; pattern fills and global colours are read since #1025). Writing Illustrator's own editing data is out of scope
  (undocumented); Save As `.ai` stays PDF-compatible, and Illustrator 2018 opens such a file without its layers and
  groups (#868).
- **Evidence:** the issues above; 28 of the 350 issues are about `.ai`/EPS (about 22 people, the most common single
  format theme), 20 fixed, and the 8 open ones are about structure (layers, guides, speed, live objects), not missing
  art. No real-file `.ai` corpus (users' files may be used locally, never committed).
- **Impact:** blocks anyone who hands files back to Illustrator users. Opening and reworking their own archive
  works, with layer cleanup in some files. This is the main-format gap that keeps the stage at alpha.
- **Estimate:** 10–20 h with users' files to test against.

## G10 Launch and stability on real machines

- **Done:** RPM required dependencies use 64-bit library capabilities and declare glibc 2.35, libm and libgcc_s (#1275); generated metadata and isolated synthetic-provider checks cover x86_64/aarch64. Real-distribution install/start-up QA is still missing.
- **Missing:** the app not starting or staying in the background on some Windows 11 machines (#858, #964 fixed
  2026-10-10, #620, #818), slow pointer on a Linux AppImage (#575), macOS paste from other apps (#954), a
  web-free crash/hang reporter, a field record.
- **Evidence:** open issues; the never-crash lints and `guard` stop panics, not driver, windowing or packaging
  failures.
- **Impact:** an app that doesn't start on someone's machine is 0% for them.
- **Estimate:** 20–35 h, needs reporters' help on hardware we don't have.

## G8 Hardening at scale

- **Missing:** a corpus of real-world SVG/PDF/EPS files; more Affinity versions and platforms (51 pinned files
  now, including 28 original Affinity 3.2.3 feature probes; large interacting documents and complex typography
  missing); Windows, Linux and browser QA by hand; packaging polish (Homebrew cask #588, Flathub #554, update
  checks #615).
- **Impact:** unknown failures on users' files.
- **Estimate:** 30–50 h (the corpus and QA part blocks beta).

## G11 Correctness bugs in core tools

- **Done:** Divide splits filled paths along open cutting lines and curves (#1307), preserving the frontmost filled appearance, implicit filled closure, holes, unrelated objects and one Undo/Redo; the existing bounded-map fallback remains.

- **Done:** Numeric fields keep a text drag's start when the pointer gains focus and movement exceeds the click threshold (#1326, in part). A completed focus click selects the whole value; keyboard focus, double-click and numeric stepping remain regression-tested. Short movements still classified as clicks can select all; that case remains open.

- **Done:** DXF reads control-point SPLINE closure (#1299 follow-up after #1320); open curves keep their open state. Escape leaves Artboard editing for Selection; during a drag it first cancels the gesture. Headless egui key events cover idle Escape, active-drag cancellation and a subsequent Escape in `tests_distortkeys.rs`.

- **Done:** Simplify and Add Anchor Points clear stale component selections on rewritten paths; Simplify previews and reports original/new point counts (#1272).
- **Done:** no-op anchor Align preserves Undo, Redo, saved state and path geometry (#1180 follow-up),
  with the same 1e-9 pt tolerance as whole-object Align; actual alignment remains one undo step.
- **Done:** scaled text's size and horizontal-scale controls account for object/group transforms (#1034),
- **Done:** Direct Selection picks a segment independently of its anchors and Delete preserves neighboring edges (#1278);
  scaled text's size and horizontal-scale controls account for object/group transforms (#1034),
  including exact sizes across grouped labels and resetting horizontal stretch while keeping editable text.
  Single-result Pathfinder booleans keep source names and SVG IDs, including named groups/compound paths (#1293).
- **Missing:** Shape Builder misses planar regions and targets whole shapes (#937); Transform effect dialog's invisible checkboxes and missing options (#885);
  PDF text boxes moving on open (#722); SVG units reverting to points (#864); bezier drag preview freezing (#834).
- **Impact:** wrong output in everyday work.
- **Estimate:** 8–15 h.

## G4 Advanced type

- **Improved (#1258):** full font names (name ID 4) resolve the face instead of falling back, and Missing Fonts searches recognize the same aliases. Synthetic regressions cover loaded/cataloged fonts, localized names, real-family precedence and variable default faces. Windows font-service activation still needs reporter QA; estimate unchanged.
- **Missing:** hyphenation and justification options, Optical Margin Alignment (type core); CJK vertical
  composition (ruby, mojikumi sets and dialog, kinsoku settings #633, manual tate-chu-yoko), Middle Eastern
  features beyond bidi and digit types (Character Direction, kashidas, composers), variable font axes, spell check with an open dictionary,
  Touch Type, Retype, Snap to Glyph; optional preservation of font sizes during group scaling (#1034 enhancement).
  Itemized in [type-parity.md](type-parity.md).
- **Impact:** professional typography and Japanese/Chinese publishing.
- **Estimate:** 34–47 h (type core 14–19 h blocks beta).

## G12 Preferences and panels that don't act yet

- **Missing:** ~16 Preferences options stored but not acted on (#394: Highlight Alternate Glyphs, Open
  Documents As Tabs, Real-time Drawing, hyphenation language, Appearance of Black for output, devices…), Properties panel per
  context, context menus beyond the canvas, Layers panel row menus and Collect for Export, SVG Interactivity.
- **Impact:** settings that silently do nothing erode trust.
- **Estimate:** 13–20 h.

## G2 Photoshop-style raster effects and the Effect Gallery

- **Missing:** 28 of 57 filters (the remaining 14 Artistic filters and all of Sketch), the Effect Gallery dialog, Load
  Texture for Glass and Texturizer, Smart Blur's Edge Only and Overlay Edge. Measured in
  [effects-parity.md](effects-parity.md).
- **Done:** the pixel-filter pipeline and 29 filters (Artistic: Film Grain, Blur, Brush Strokes, Distort, Pixelate, Sharpen, Stylize,
  Texture, Video), Brush Strokes on 2026-10-11 and Film Grain on 2026-10-12.
- **Estimate:** 23–39 h (measured 0.7–1.2 h per filter); parallelizes well.

## G13 Illustrator 2026 (30.x) additions

- **Missing:** gradient dither and perceptual blending, gradient presets (30.0); tangent and perpendicular
  snapping and the consolidated snapping options (30.0); artboard locking, artboard colours and managing
  artboards from the canvas (30.0); the Blend panel with ease in/out and separate colour acceleration (30.7);
  TIFF in Export for Screens (30.2); the Dimension tool (28.x).
- **Evidence:** Adobe's 30.0 and 30.7 release announcements, see
  [target-app-parity.md](target-app-parity.md#how-this-was-measured-2026-10-10).
- **Estimate:** 15–25 h.

## G14 Pen hardware beyond Windows pressure

- **Missing:** pen pressure on macOS (#852) and Wayland (#491, #764) — winit 0.30 reports none there; tilt,
  bearing and barrel rotation for brushes (6D Art Pen, #372); the eraser end.
- **Estimate:** 8–16 h, partly waiting on winit 0.31; needs pen hardware to verify.

## G5 Brushes, symbols and libraries

- **Missing:** the Art brush's Overlap, auto-generated Pattern brush corners, new tile art in Pattern Brush
  Options, the Key Color eyedropper; original brush, symbol and graphic-style libraries generated in code (never
  Adobe's); dynamic symbols; Start Global Edit.
- **Estimate:** 17–28 h.

## G3 3D and Materials

- **Done:** an initial live Revolve in the `three-d` crate (#846, from the contributor who offered in #605).
- **Missing:** Extrude & Bevel, Inflate and Rotate, caps, materials and better lighting, exact visibility for
  intersecting surfaces, 3D Classic, Turntable, 3D export (OBJ, USDA, glTF).
- **Impact:** specialist, but a visible menu of stubs.
- **Estimate:** 45–75 h, the largest single gap; plan in `plan/` before coding.

## G7 Views and windows

- **Done:** custom document ruler origin (#1212): corner drag with preview, double-click reset to the active artboard, relative X/Y inputs and native save/reopen.
- **Missing:** multiple windows and Window › Arrange (Cascade, Tile, Float in Window, Consolidate All Windows),
  panels on another monitor, New View/Edit Views, global/artboard ruler mode switch and video rulers.
- **Estimate:** 15–25 h.

## G6 Automation and scripting

- **Missing:** a scripting surface over the command registry (Illustrator: JavaScript), Batch, image and graph
  variable kinds, dataset import/export. Actions, Variables (text and visibility) and full agent control over
  MCP are done.
- **Estimate:** 8–14 h.

## G15 Performance budgets and a GPU renderer

- **Improved (#1257):** Craquelure, Mosaic Tiles and Stained Glass reuse squared distances during nearest-cell searches, with the original distance ordering retained near rounding/tie boundaries. A brute-force boundary regression and full-viewport pixel comparisons agree with the old result. `texture_drag_perf` reproduces a moved CMYK rounded rectangle at 190.07% and DPI 1.5; timings from a busy development machine are supporting evidence, with idle Windows drag QA still pending.
- **Missing:** an idle-machine run of `vectorcraft-cli perf` at 50k paths (last: 2026-10-11 at 25k paths, every
  budget met, 400% zoom 12.3 ms against 16 ms, measured), a spatial index for zoomed-in frames, dirty-region
  rendering, the GPU backend spike (M5). Large-selection edits, cached live effects and Pathfinder Trim, Merge,
  Crop and Outline landed in #1173.
- **Estimate:** 15–25 h (the budget run and fixes block beta).

## G16 Localization: the four missing key languages

- **Evidence:** Czech render-site bypasses reduced in file dialogs, diagnostics, preset names and contributor tooltips; gradient grammar and printer-mark context corrected (see [localization-parity.md](localization-parity.md)). Native review and the missing-language work remain; estimate unchanged.

- **Missing:** Hindi, Arabic, Indonesian, Vietnamese catalogs; a Korean UI font for the web build; a right-to-left interface and interface
  shaping for Arabic and Devanagari; messages for Simplified Chinese and Brazilian Portuguese; native review of
  every catalog. Measured in [localization-parity.md](localization-parity.md).
- **Estimate:** 60–100 h plus human review (Korean landed 2026-10-12).

## G17 Ecosystem: third-party plug-ins

- **Missing:** Illustrator's ecosystem is a C++ plug-in SDK and a large commercial plug-in market. VectorCraft
  has sandboxed WebAssembly plug-ins for object filters and live effects ([plugins.md](plugins.md)) and MCP;
  missing are tool plug-ins, panel plug-ins, a plug-in directory and any third-party plug-ins (#710).
- **Estimate:** 20–45 h for the APIs; the ecosystem itself isn't agent work.

## G18 Generative AI features

- **Missing:** Text to Vector Graphic, Generative Recolor, Generative Shape Fill (#766), Generative Expand,
  Concept to Vector, Prompt to Edit, Rewrite Text. Adobe's run on its cloud models; ours would be a pluggable
  provider.
- **Needs:** an owner decision on models and providers.
- **Estimate:** 30–60 h after that decision.

## G19 Accessibility

- **Missing:** AccessKit is on by default, so egui widgets reach screen readers, but custom panels, the canvas and its objects aren't described and nothing has been tested with VoiceOver, Narrator or Orca; keyboard-only canvas operation; high
  contrast. Illustrator's own accessibility is limited, so this is mostly beyond parity.
- **Estimate:** 15–25 h.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | G7: custom document ruler origin (#1212) done; global/artboard mode switching and video rulers remain, estimate unchanged |
| 2026-10-12 | minor | G1: Artboard object clicks create a fitted artboard (#1227); remaining interaction pass and estimate unchanged |
| 2026-10-12 | minor | Pathfinder single-result booleans preserve source names and SVG IDs (#1293); paint-order, grouped/compound sources, unchanged geometry/appearance and Undo/Redo regression tests; scores and estimates unchanged |
| 2026-10-12 | minor | DXF reads control-point SPLINE closure (#1299 follow-up after #1320); open curves stay open and curves retained; rational/non-rational import and export versions regression-tested; scores/hours unchanged |
| 2026-10-12 | minor | G16: Korean catalog added (complete under the tests); four key languages remain, 60–100 h |
| 2026-10-11 | minor | Czech render-site follow-up: file-dialog filters, diagnostics, preset numbering, contributor tooltips/plurals, gradient grammar and contextual printer mark styles; estimates unchanged |
| 2026-10-12 | minor | Document Setup: Edit Artboards closes the modal after applying valid settings; pointer-click regressions; scores and hours unchanged |
| 2026-10-12 | minor | Artboards: Escape exits editing; an active drag cancels first; shortcut-routing regression; scores and hours unchanged |
| 2026-10-12 | minor | G11: clear stale component refs after Simplify/Add Anchor Points; preview and point counts give visible feedback (#1272); estimates unchanged |
| 2026-10-12 | minor | G2: Artistic › Film Grain added with live controls and raster exports; measured Photoshop filters 29/57, 28 remaining (23–39 h). |
| 2026-10-12 | minor | G10: RPM required dependencies and glibc floor metadata checked (#1275); real-distro install/start-up QA remains; estimates unchanged |
| 2026-10-12 | minor | G15: texture-effect distance searches optimized (#1257); idle-machine/platform drag QA still pending; estimate unchanged |
| 2026-10-12 | minor | G1: Curvature point display improved; existing-path interpolation request remains open (#1236, in part); estimates unchanged |
| 2026-10-12 | minor | G4: full-name font matching and Missing Fonts searches verified (#1258); Windows font-service QA pending; estimate unchanged |
| 2026-10-12 | minor | G11: no-op anchor Align no longer consumes Undo, clears Redo or marks saved paths modified (#1180 follow-up); estimate unchanged |
| 2026-10-12 | minor | G1: Eyedropper uses an original pipette glyph; precise setting and DPI checks verified (#1269); estimates unchanged |
| 2026-10-12 | minor | G1: Eyedropper reads visible locked and outside-isolation sources (#1251); estimate unchanged |
| 2026-10-12 | minor | G11: Direct Selection segment Delete retains neighboring geometry and appearance (#1278); estimates unchanged |
| 2026-10-12 | minor | G1: painted strokes regain the existing default after None/zero-width inheritance (#1253); estimate unchanged |
| 2026-10-12 | minor | G11: Divide uses open cutting paths without taking their paint (#1307); implicit filled closure, holes and history regression-tested; estimates unchanged |
| 2026-10-12 | minor | G11: numeric fields keep the text-drag start instead of replacing it with select-all on press (#1326, in part); keyboard and click selection regression-tested; estimates unchanged; short movements below the click threshold remain open |
| 2026-10-11 | minor | G15: large-selection edits, cached live effects and faster Pathfinder (#1173); a 25k-path perf run meets every budget; estimate unchanged |
| 2026-10-11 | minor | G4: Middle Eastern digit types done (Character panel menu › Digits); estimate unchanged |
| 2026-10-11 | minor | G4: proportional vertical metrics (#966) done; estimate unchanged |
| 2026-10-11 | minor | G11: #893 fixed (merged regions of crossing lines left seams); #937 still open, estimate unchanged |
| 2026-10-11 | minor | #1034: corrected scaled type controls; optional group font-size preservation remains separate. Estimates unchanged at this rounding. |
| 2026-10-11 | minor | G9: legacy `.ai` without the prolog opens with its layers (#1027) |
| 2026-10-11 | minor | G9: `.ai` pattern fills and global colours read from the editing data (#1025) |
| 2026-10-10 | minor | G3: initial Revolve landed (#846), 45–75 h left |
| 2026-10-10 | minor | G9 evidence counted from the tracker (28 `.ai`/EPS issues, 20 fixed); impact narrowed to handing files back |
| 2026-10-10 | minor | Alpha blockers checked against the core-workflow gate: none |
| 2026-10-10 | major | Moved from the ROADMAP's "Where we're lacking"; kept ids G1–G8, added G9–G19 (file formats, stability, bugs, preferences, 30.x additions, pen hardware, performance, localization, ecosystem, AI, accessibility) and re-ranked |

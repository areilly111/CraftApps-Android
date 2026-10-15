# Effects parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (G2: Film Grain added; Photoshop effects 29/57; 28 filters remain) · **Target:** Adobe Illustrator 2026 (30.x)

Every entry of Illustrator's Effect menu, by its public feature name, against VectorCraft's effects catalogue
(`effect_catalog()` in `crates/effects/src/lib.rs`) and the Effect menu built from it
(`crates/ui-egui/src/menus.rs`). Counts are **measured** from the catalogue; "depth" notes are estimated.
Part of [target-app-parity.md](target-app-parity.md); open work is in [gaps.md](gaps.md).

## Summary

| Group | Illustrator | VectorCraft | % | Remaining (h) |
|---|---:|---:|---:|---:|
| Illustrator effects (vector and stylize) | 50 | 44 | 88% | 6–10 |
| Photoshop effects (raster, Effect Gallery) | 57 | 29 | 51% | 23–39 |
| 3D and Materials | 4 (+3 Classic) | 1 (initial Revolve) | ~8% | 45–75 |
| Beyond Illustrator: Effect › Color Adjustments (6), effect plug-ins (WebAssembly) | — | 6 + plug-ins | — | — |

## Illustrator effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Convert to Shape | Rectangle, Rounded Rectangle, Ellipse | 3/3 | — |
| Crop Marks | Crop Marks | 1/1 | — |
| Distort & Transform | Free Distort, Pucker & Bloat, Roughen, Transform, Tweak, Twist, Zig Zag | 7/7 | Transform's Transform Objects/Patterns and Scale Strokes & Effects options, invisible Reflect checkboxes (#885) |
| Path | Offset Path, Outline Object, Outline Stroke | 2/3 | Outline Object |
| Pathfinder | Add, Intersect, Exclude, Subtract, Minus Back, Divide, Trim, Merge, Crop, Outline, Hard Mix, Soft Mix, Trap | 10/13 | Hard Mix, Soft Mix, Trap |
| Rasterize | Rasterize… (as a live effect) | 0/1 | the live effect (Object › Rasterize exists) |
| Stylize | Drop Shadow, Feather, Inner Glow, Outer Glow, Round Corners, Scribble | 6/6 | — |
| SVG Filters | Apply SVG Filter… and the filter list | 0/1 | the whole submenu (SVG filters on import round-trip already) |
| Warp | Arc, Arc Lower, Arc Upper, Arch, Bulge, Shell Lower, Shell Upper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist | 15/15 | warp point editing in envelopes |

Document Raster Effects Settings exists. When the 64 million pixel cap on an effect image lowers
its resolution, PDF export warns once per object with the requested and the lowest achieved ppi
(`crates/engine/src/tests_pdfraster.rs`).

## Photoshop effects

| Submenu | Items | Ours | Missing |
|---|---|---:|---|
| Artistic | Colored Pencil, Cutout, Dry Brush, Film Grain, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor | 1/15 (approximate Film Grain) | Colored Pencil, Cutout, Dry Brush, Fresco, Neon Glow, Paint Daubs, Palette Knife, Plastic Wrap, Poster Edges, Rough Pastels, Smudge Stick, Sponge, Underpainting, Watercolor |
| Blur | Gaussian Blur, Radial Blur, Smart Blur | 3/3 | Smart Blur's Edge Only and Overlay Edge modes |
| Brush Strokes | Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines, Spatter, Sprayed Strokes, Sumi-e | 8/8 | — |
| Distort | Diffuse Glow, Glass, Ocean Ripple | 3/3 | Glass's Load Texture |
| Pixelate | Color Halftone, Crystallize, Mezzotint, Pointillize | 4/4 | — |
| Sharpen | Unsharp Mask | 1/1 | — |
| Sketch | Bas Relief, Chalk & Charcoal, Charcoal, Chrome, Conté Crayon, Graphic Pen, Halftone Pattern, Note Paper, Photocopy, Plaster, Reticulation, Stamp, Torn Edges, Water Paper | 0/14 | all |
| Stylize | Glowing Edges | 1/1 | — |
| Texture | Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass, Texturizer | 6/6 | Texturizer's Load Texture |
| Video | De-Interlace, NTSC Colors | 2/2 | — |
| Effect Gallery | the combined browser and stack dialog | 0/1 | all |

The pixel-filter pipeline (`vectorcraft_effects::pixel`) is in place: each new filter is one function, a
catalogue row, a menu entry and translations. Measured throughput: 0.7–1.2 agent hours per filter
(see [calibration](target-app-parity.md#calibration)), so the 28 missing filters are ~19–34 h and the Effect
Gallery dialog another 4–6 h. These parallelize well across agents.

Film Grain (`artistic.filmGrain`, Effect › Artistic) adds stable monochrome noise to shadows and
midtones, with Grain, Highlight Area and Intensity sliders, live preview and appearance editing.
Its smooth grain field follows translations, preserves transparency and stays in document units;
zoomed-out rendering uses bounded approximate footprint filtering. Changing the bounds shifts
the pattern. Command exports bake it to an image in SVG and PDF, while the native document keeps the effect editable. Behaviour is an original visual
approximation based on [Illustrator's public effects reference](https://helpx.adobe.com/illustrator/using/summary-effects.html)
and the [public Artistic filters guide](https://helpx.adobe.com/photoshop-elements/using/artistic-filters.html),
not a pixel-identical proprietary implementation. Intensity is interpreted as highlight strength and
quieter highlight grain. Grain strength stays even across shadows and midtones and rolls off above
mid-grey to a quarter-strength floor at full Intensity before highlight headroom; cream retains
soft grain, black takes light speckles and white stays clean. The tonal curve and endpoint behaviour
are interpretations, not verified proprietary behaviour. Saturated colour channels may clip. The parameter
ranges and defaults are chosen for VectorCraft, not asserted as measured proprietary defaults.
Near white, highlight headroom can limit grain equally at every Intensity; highlight lift still
depends on Intensity when Highlight Area is nonzero.
The two rotated, variance-normalised noise layers can leave a faint diagonal weave at high
magnification. Removing that remaining direction would require a more expensive field; the
current implementation keeps bounded per-pixel work. Film Grain is still slower than Texture
Grain (Regular), particularly when its wider footprint needs four samples.
Calibration on uniform grey at Grain 20, using 32×32 fine samples per coarse pixel, measured
grain standard deviation within about 15% of the dense area average at pixel widths of 2, 4
and 8 document units (ratios 1.13, 0.89, 0.85). This is evidence for the bounded approximation,
not an accuracy guarantee for arbitrary artwork. Tests: `pixel::tests::film_grain_*`,
`tests_rasterfilters`, `tests_adjust::film_grain_dialog_*` and `tests_labels::effect_menu_lists_film_grain_*`.

## 3D and Materials

| Item | Ours |
|---|---|
| Revolve | initial: live, editable profile, rotation and light gizmos, flat shading, SVG/PDF output, Expand Appearance ([revolve.md](revolve.md), #846) |
| Extrude & Bevel, Inflate, Rotate | none (menu stubs) |
| Materials panel, lighting, ray-traced render | none |
| 3D (Classic): Extrude & Bevel, Revolve, Rotate | none |
| Turntable (30.0 beta) | none |
| Export 3D objects (OBJ, USDA, glTF) | none |

The `three-d` crate (depends only on `geom`) builds, rotates, projects, shades and depth-sorts the surfaces;
the effects crate turns them into art. What's left (Extrude & Bevel, Inflate, Rotate, caps, materials, ray-traced
rendering, Turntable, 3D export) is 45–75 h, still the largest single gap.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | G2: Artistic › Film Grain added with live controls and raster exports; measured Photoshop filters 29/57, 28 remaining (23–39 h). |
| 2026-10-11 | minor | PDF cap warnings identify original objects after appearance splitting and report the lowest achieved ppi; cap/rendering and scores unchanged |
| 2026-10-11 | minor | Zig Zag's dialog: Size and Ridges per segment sliders, Points as Smooth / Corner radio buttons (#1133) |
| 2026-10-11 | minor | Brush Strokes landed (8 filters): 28/57 Photoshop effects |
| 2026-10-10 | minor | Initial Revolve landed (#846) |
| 2026-10-10 | major | First checklist, counted from the effects catalogue (20/57 Photoshop effects, 44/50 Illustrator effects) |

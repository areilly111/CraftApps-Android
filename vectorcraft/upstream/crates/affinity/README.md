# vectorcraft-affinity

An independent, bounded Rust reader for native Affinity documents: `.af` from Affinity 3 and
`.afdesign`, `.afphoto` and `.afpub` from Affinity 1 and 2 (container versions 8 to 12). It reads
the archive, the tagged object stream in `doc.dat` and the part of the document model listed below,
and hands the engine a document in pixels with every transform composed. It does not write
Affinity files.

## What is imported

| Affinity | VectorCraft |
|---|---|
| Pages, Publisher spreads (one or two pages) | artboards, spreads side by side |
| Artboards (legacy `ABEn` or Affinity 3 `phrp`/`aprp`), with name, paint and any transform | artboards plus a clip group with the actual vector outline, background and stroke |
| Layers (`Scop`), groups (`Grup`), pass-through or isolated | layers and sublayers, groups |
| Curves (`PCrv`): cubic subpaths, closed flags, live corners (`CnrD`) | paths (multi-subpath curves fill even-odd) |
| Rectangles with corner radii (relative or absolute), ellipses, polygons (smooth too), stars, square stars, pies, triangles, trapezoids | paths |
| Compound shapes (`Comp`, add and subtract) | one even-odd path |
| A shape or curve with children clips them | clip group whose clipping path keeps its fill (behind) and stroke (over) |
| Vector masks (`AdCh` curves and shapes) | clip group |
| Pixel masks (`MRst`, attached or as a layer in a group) | opacity mask |
| Solid fills: RGBA, HSLA, CMYK, grey, Lab (D50) | fill appearance (alpha as fill opacity) |
| Linear, elliptical and radial gradients with Affinity's midpoint bias | gradients (the bias as the midpoint where the blend is half way) |
| Strokes: weight, scale with object, dash pattern and phase, alignment, miter limit, behind the fill | stroke appearance |
| Fill layers (`FRst`) | filled rectangles |
| Artistic and frame text: Unicode scalar-indexed runs, stored fallback font, family/weight/style, boolean OpenType features, all caps, size, tracking, fixed leading, colour, paragraph alignment, first baseline | point type and area type |
| Placed images (`ImgN`): the original JPEG/PNG embedded in the file | embedded images |
| Pixel layers (`Rstr`): RGBA 8/16-bit and CMYK 8-bit tiles, cropped to their content; source-backed RGBA8 JPEG/PNG tiles with matching dimensions and zero origin | embedded PNG images |
| Embedded documents and symbols (`EmbN`) | the picture Affinity cached of them |
| Opacity, visibility, lock, names, the blend modes VectorCraft has | the same |

Everything else is reported in the import warnings, one line per kind with a count, never
dropped silently: layer effects, adjustment layers and live filters, brush and pressure strokes,
transparency gradients, fill opacity, bitmap fills, master pages, conical gradients (as radial),
special shapes (cloud, heart, cog, callouts, arrows…, imported as their bounding ellipse), corner
types other than round, stars with rounded points, several fills or strokes on one object (the
active one is used), outlined or scaled text, text fields such as page numbers, frames with
columns or a curved outline, paragraph indents/spacing, mixed paragraph alignments, unsupported font selectors, grey/Lab/32-bit pixels, CMYK pixels (converted to RGB without the
document's profile) and Affinity-only blend modes (Add, Linear Light… as Normal).

When the native document can't be read at all, the engine opens the embedded PNG preview instead,
with a warning that names the reason; that preview can't be placed, used as a template or mined
for swatches and styles. Save never writes back over the Affinity file.

## Provenance and clean-room

Affinity has no published file-format specification, so this reader is built only from a public
description of the format and from public files, never from Affinity itself. This is how the sibling app PhotoCraft already treats formats that
are only partly documented:
its PSD reader fills the gaps in Adobe's specification from the MIT-licensed psd-tools and
ag-psd, and its camera raw decoder recovered Nikon's compression tables by black-box analysis
of CC0 sample files.

* **No Affinity software.** Affinity was never downloaded, installed, run, scripted, screenshotted or
  disassembled for this work, and nothing from an Affinity installation (program code, resources,
  presets, fonts, colour profiles) was read. No file was made with Affinity for it: every sample
  is a document its author published.
* **A public, permissively licensed description.** The container and object-stream layout was
  learned from [VMDevCpp/afread](https://github.com/VMDevCpp/afread) (MIT, at
  `04b672334a43e3e37ded6b5ffc57af231d589774`, written for container versions 7–11) and re-described
  in our own words before this Rust code was written; no code was translated. No GPL/AGPL code (such
  as Inkscape's Affinity extension) was read.
* **Every structure checked against public files.** afread doesn't say how its author learned the
  format, so nothing here rests on it alone: each structure the reader relies on is confirmed by
  the public documents. Every archive entry carries a CRC-32 that must match, and every field in
  the object stream names its own type, so a misread layout fails loudly instead of producing
  plausible wrong data.
* **Meaning fitted to Affinity's own pictures.** What shapes, paints, text and pixel data mean was
  worked out by comparing our render with the thumbnail every Affinity document embeds (Affinity's
  render of itself), on 176 public documents saved by Affinity 1.x, 2.x and 3.0/3.1 on Windows,
  macOS and iPad. The collection has since grown to 189 distinct documents, and the reader opens
  all of them. Most are published under CC0, MIT, BSD, Apache-2.0 or CC BY (-SA); the others
  (no licence stated, or GPL or non-commercial terms) were only opened locally to compare
  pictures, never committed or redistributed. The corpus test keeps the comparison for the pinned
  files, which are CC0, MIT or Apache-2.0 only.
* **Nothing of Affinity's is in this repository**: no program code, assets or documents. The test
  files are fetched at pinned commits and sha256-verified, never committed.
* **Read-only.** Nothing is written in Affinity's format; export waits until someone can check
  written files in Affinity itself.

The reader also has an independent copy in PhotoCraft's `photocraft-affinity`.

## Validation

* Unit tests on synthetic containers built by the `synth` feature (stored, zlib and zstd entries,
  checksums, budgets, cycles, hostile streams, every truncation) and property tests of random
  mutations.
* `cargo xtask corpus --affinity` fetches 51 public CC0/MIT/Apache-2.0 documents at pinned
  commits, SHA-256 verified against `xtask/affinity-corpus.sha256`, with upstream license notices.
  Of these, 33 are current `.af`: four CC0 vector illustrations and 28 original Affinity 3.2.3
  feature probes plus one separately identified derived regression input from
  [SethRobinson/Patchy](https://github.com/SethRobinson/Patchy). Only document data and provenance
  notices were inspected, never that project's importer or authoring scripts.
* `tests/real_files.rs` parses every file and freezes the modeled features of all 33 current
  `.af` files. Native raster tests compare decoded pixels with independent embedded images;
  text tests assert Unicode run boundaries, fallback fonts and feature retention.
* `engine/tests/affinity_corpus.rs` requires editable native import, exact `.vectorcraft`
  save/reload, and SVG/PDF/PSD export of every board in all 33 current files. It checks SVG/PDF
  dimensions and reimport, and independently decodes PSD merged pixels using the published
  specification. These are VectorCraft consistency checks, not independent Affinity PSD oracles.
  Saved-thumbnail comparisons cover the existing 22 documents plus nine new representative
  fixtures, checking every board; unsupported effects and text layout are not claimed visually
  faithful. The Affinity corpus workflow requires the files, so missing fixtures cannot silently
  pass CI. Run locally with `AFFINITY_CORPUS_REQUIRED=1` for the same strict gate.
* Synthetic engine regressions cover nested, rotated and curved artboards, negative origins,
  multiple spreads and resolution conversion, alongside text recovery and native save/export.
* `engine/tests/import_fuzz.rs` mutates a synthetic native document's stream and archive and checks
  that whatever opens also renders and exports; `fuzz/` has `cargo-fuzz` targets for the whole
  reader (`container`) and the object stream (`stream`):

  ```sh
  cd crates/affinity
  cargo +nightly fuzz run stream -- -max_total_time=600
  ```

The source audit, licensing decisions and remaining complex-document/font cases are recorded in
[affinity-validation.md](../../docs/affinity-validation.md). Reopening VectorCraft's exports in
Affinity remains an independent validation step that hasn't been done.

What has not been verified: files from Affinity builds or platforms outside the public set, files
written by other applications, rotated or skewed images against Affinity's render, mask polarity
against a render with and without the mask, and anything listed as a warning above. The `synth`
builders write only what this reader accepts; Affinity has never opened their output, so they are
test fixtures, not an exporter.

## Safety limits

Every size is checked before it is allocated: 256 MiB per archive entry and 1 GiB per import by
default (`Limits`), a 64 MiB zstd window, 4096 saved revisions, array lengths no longer than the
bytes left, 16 777 216 decoded values per document stream (fields and array elements together, counted before
anything is allocated: a value takes about 40 bytes in memory however few it took in the file; the largest of
those 189 public documents uses 3.3 million), 384 levels of object nesting, 128 levels of layers, 500 000 layers, four million curve
nodes per curve; compound outlines also cap recursion at 128 levels, operand visits at 100 000
and cumulative geometry work at four million, rejecting cycles and partial results on limits.
Pie sweeps normalize finite angles in constant time. Pixel layers are limited to 64 megapixels
(cropped to their content first); gradients to 1024 stops. Text collection is bounded to a
million characters per node before allocation, with a cumulative 16 MiB budget for owned font
names, features and gradient stops. Exhaustion warns and uses default attributes for remaining
text, preserving its characters.
Every archive entry's CRC-32 and size must match. Malformed input returns an `Error`; the
crate has no panics outside tests.

## Privacy

Affinity documents can hold the folder they were saved in, user names, original image paths and
XMP metadata. This reader never imports those fields.

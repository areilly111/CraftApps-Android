# Gaps: where LightCraft falls short of Lightroom

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (created; absorbs the checklist's Top gaps and the ROADMAP's honest assessment, re-ranked after a full re-measure) · **Target:** Adobe Lightroom Classic 15.5 / Lightroom 9.6

Every known shortfall, one entry each, ranked by user impact (how many Lightroom users hit it, and
how hard it blocks them), then by effort. **Agents pick work from the top** (the first entry nobody is
working on). When you close a gap, delete or shrink its entry, update its rows in
[parity-checklist.md](parity-checklist.md), and the numbers in [target-app-parity.md](target-app-parity.md)
and [ROADMAP.md](../ROADMAP.md) if they move.

Hours are Opus 5.5 agent wall-clock hours (see [target-app-parity.md](target-app-parity.md#how-hours-are-calibrated)).
`B` marks a gap that blocks **beta**.

## Ranked

| # | Gap | Evidence | User impact | Hours | Doc |
|---:|---|---|---|---:|---|
| 1 B | **Camera colour is estimated, not measured** (LR-PROF-CAMERACOLOR). Raws start from a per-file fit to the camera JPEG, 4 bundled profiles or 52 spectral matrices; others use a neutral matrix | ΔE vs camera JPEG 2.9–10 after fitting (raw-parity.md); 0 chart-calibrated models vs 1,500 Adobe Standard profiles | every raw shooter sees colour that is close but not Lightroom's; some get neutral-matrix colour | 50–90 | [raw-parity](raw-parity.md#colour) |
| 2 B | **CR3 beyond three verified bodies** (LR-IMP-FORMATS): CRX variants other than those verified on M50 / R100 / R8 fall back to the preview; CR3 sRAW / mRAW and HEIF CR3 too | measured: Canon is the largest bucket (33% of photographers) and its share whose camera fully works is the lowest of the big four, 72%; 17 of 22 popular Canon bodies write CR3, 2 verified (raw-parity.md) | +6.5 points of photographers whose camera works, the biggest single raw gain | 25–45 | [raw-parity](raw-parity.md#remaining-work-ranked-by-photographers-unlocked-per-hour) |
| 3 B | **Render fidelity unmeasured** (LR-BEHAV-RENDER-FIDELITY): tone, highlights, texture / clarity / dehaze, NR and sharpening are tuned by eye | no side-by-side suite; a retune needs a new process version (only V1 exists) | edits imported from Lightroom look different; the look "isn't Lightroom's" | 30–60 | [raw-parity](raw-parity.md), [process-versions](process-versions.md) |
| 4 B | **Original-pixel detail at 100%** (LR-VIEW-ZOOM, P0): the whole-image preview is capped (2,560 px default); compare, overlays and soft proofing lack sharp zoom windows (#323) | checklist row 🟡 | checking focus, the first thing culling needs, isn't reliable on large files | 8–15 | [ui-parity](ui-parity.md) |
| 5 B | **AI masks: Subject, Sky, People, Background, Landscape** are heuristics; Object / Describe run SAM 3 but its download has no mirror configured | `crates/segment`; ai-masks.md; MASK rows 67% | masking is how most Lightroom users edit today | 40–70 + model decision | [target-app-parity](target-app-parity.md#ai) |
| 6 B | **Compressed ORF** (every Olympus / OM body since ~2008) | measured: 66 of 78 Olympus / OM models preview-only; `orf.rs` refuses; OM bucket 3% of photographers, 3% of them fully work | +2.6 points; best gain per hour of any raw gap (~0.2) | 8–15 | [raw-parity](raw-parity.md) |
| 7 | **Nikon High Efficiency NEF** (HE / HE★ on Z 8, Z 9, Z 6III, Z f, Z50II, Z5II when set to HE) | named and preview only (#193); needs a pure-Rust JPEG XS decoder | +1.1 points (only shots taken in HE; lossless works on the same bodies); low gain per hour (~0.04), so not a beta blocker | 20–40 | [raw-parity](raw-parity.md) |
| 8 B | **No lens profile database** (LR-EDIT-OPTICS-PROFILE): only DNG opcodes, RW2 in-file distortion and Sony ILCE-7RM4A distortion | Adobe ships 3,635 lens profiles; we never use them | distortion / vignetting correction is a default step for many | 40–80 + calibration shots | [raw-parity](raw-parity.md#lens-profiles) |
| 9 B | **Tests don't run in CI on macOS, Windows or Linux x64**: only FreeBSD runs `cargo test`; the others build packages | `.github/workflows/` (freebsd, packaging-lint, release, windows-arm64) | regressions reach users on the main platforms; `cargo xtask ci` runs only locally | 6–12 | [target-app-parity](target-app-parity.md#by-dimension) |
| 10 B | **Per-model verification thin, plus cheap per-body decode fixes**: 234 of 1,446 Adobe models verified (16%), ~40% of photographers on a verified body; Nikon 5 bodies verified, Canon 11. Cheap fixes: Z 6II packed 14-bit NEF (#769, PR #671), Sony A100 / F828 regression (#702), DSLR-A200…A390 / A850 12-bit ARW (#535) | measured: [raw/coverage.tsv](raw/coverage.tsv) | per-body bugs (levels, crops, WB) surface only from users | 25–45 | [raw-parity](raw-parity.md#by-maker-measured) |
| 11 | **Enhance: Raw Details, Super Resolution; AI denoise without shippable weights or X-Trans** | ENH rows 0%; denoise Bayer only, RawNIND weights GPL and opt-in | Fujifilm users get no AI denoise; nobody gets it out of the box | 30–60 + model decision | [raw-parity](raw-parity.md#demosaic-levels-crops-and-lens-metadata) |
| 12 | **Content-aware / generative remove, detect distractions, reflections removal** | REM rows 75%; patch synthesis missing | cleanup work goes back to Photoshop | 25–45 (+ generative model) | [target-app-parity](target-app-parity.md#ai) |
| 13 | **Lightroom catalog migration is one-way and approximate**: unsupported Adobe profiles / AI settings archived, history kept as data | lightroom-catalog-import.md | switching users lose some looks | 8–12 | [file-format-parity](file-format-parity.md) |
| 14 | **HDR display** (EDR / Windows HDR); HLG; HDR settings from `crs` XMP | HDR rows 70%; LR-VIEW-HDR-DISPLAY ⬜ | HDR editing works blind on HDR monitors | 12–25 | [hardware-parity](hardware-parity.md) |
| 15 | **Print module**: custom packages, printer colour management (contact-sheet PDF only) | LRC-PRINT rows | Classic users who print | 25–40 | [target-app-parity](target-app-parity.md#features) |
| 16 | **Map view, Book, Slideshow module, publish services** | LRC-MAP / BOOK / SS rows ⬜ | Classic users; publish services matter for Flickr / SmugMug workflows | 60–100 | [target-app-parity](target-app-parity.md#features) |
| 17 | **Tethered capture** | ⬜ | studio photographers | 25–45 + cameras | [hardware-parity](hardware-parity.md) |
| 18 | **Video**: playback, trim, global edits, export | VID rows 0% | users who mix stills and clips | 25–45 | [target-app-parity](target-app-parity.md#features) |
| 19 | **Plug-in ecosystem**: no Lightroom SDK equivalent (export / publish plug-ins, metadata plug-ins) | ⬜ | pro workflows built on plug-ins (LR/Enfuse, Jeffrey's exporters, Negative Lab Pro) | 40–80 + owner decision | [target-app-parity](target-app-parity.md#by-dimension) |
| 20 | **Long-tail raw formats**: compressed SRW, ARQ pixel shift, IIQ, Hasselblad compression, X3F, CRW, MRW, ERF, KDC, DCR, MEF, GPR, old FinePix | raw-parity.md | small user share each | 45–85 | [raw-parity](raw-parity.md) |
| 21 | **Localization**: 9 of 15 shipped languages partial (74–99% of catalog messages), lower-layer errors English; Hindi, Arabic, Indonesian, Korean, Vietnamese absent; no RTL or complex shaping | measured from catalogs | non-English users see mixed English | 55–90 (shipped to full); 145–255 (five new) | [localization-parity](localization-parity.md) |
| 22 | **Pen pressure**, OS monitor profiles, follow-window colour | hardware-parity.md | brush users with tablets; multi-monitor setups | 11–22 | [hardware-parity](hardware-parity.md) |
| 23 | **Accessibility** audit with screen readers; canvas tools keyboard-reachable | LR-BEHAV-ACCESS 🟡 | blind and low-vision users | 10–20 | [ui-parity](ui-parity.md) |
| 24 | **Brush live preview and masking feel** (#517, #518); Feather / Edge mask refinement (15.5) | issues | masking feels slower than Lightroom | 8–12 | [ui-parity](ui-parity.md) |
| 25 | **File writes**: JXL encode, PSD export, lossy DNG, Render to DNG; AVIF import | file-format-parity.md | interchange edge cases | 25–45 | [file-format-parity](file-format-parity.md) |
| 26 | **Performance at scale**: 85k-photo library opens in 1.7–4.7 s; 100k grid test pending; colour NR at half resolution | ROADMAP log, gpu-pipeline.md | large-catalog users | 15–30 | [target-app-parity](target-app-parity.md#by-dimension) |
| 27 | **Windows / Linux runtime coverage**: Windows installer UI unverified on Windows (#79); GPU path proven on Apple only | ROADMAP, issues | non-Mac users find bugs first | 15–30 | [hardware-parity](hardware-parity.md) |
| 28 | **Small Classic shortcuts and keys**: rating `[` `]`, flag cycling, filter-bar keys, F1 help, panel keys (Tab, F5–F8) | [ui-parity](ui-parity.md#shortcuts) | Classic muscle memory | 4–8 | [ui-parity](ui-parity.md) |

Beta-blocking gaps (B) total **~230–430 h**; with the stability and platform work they need around them,
**~340–600 h** to beta (incl. 40–80 h for the ~92 open core-path user issues) (see [target-app-parity.md](target-app-parity.md#stage)).

## Gaps by kind

- **Feature:** 5, 8, 11, 12, 13, 15, 16, 18, 19
- **UI / UX:** 4, 23, 24, 28
- **File format:** 2, 6, 7, 10, 13, 20, 25 (raw ones in [raw-parity.md](raw-parity.md))
- **Hardware:** 14, 17, 22, 27
- **Localization:** 21
- **Quality / fidelity:** 1, 3
- **Stability and platforms:** 9, 26, 27

## Closed recently (kept for calibration)

- 2026-10-10: Nikon lossy-after-split NEF (#633), ORF 12.8 bpp (#651), HDR gain map / PQ AVIF / float
  TIFF export (#646), activity stack (#345).
- 2026-10-09: HEIC in release builds (#609), Samsung uncompressed SRW (#459), contact-sheet PDF (#563),
  Ukrainian UI (#564), Sony ILCE-7CR profile.
- 2026-10-08: CR3 CRX lossless and C-RAW (#279), Fujifilm compressed RAF (#282), AI RAW denoise (#384),
  Lightroom catalog import (#218), German, Spanish, Russian UI.
- 2026-10-07: RW2 every format (#215), faces recognition in pure Rust (#174).
- 2026-10-05: compressed NEF (#86). The older history is in [ROADMAP.md](../ROADMAP.md#progress-log).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | minor | RAW gaps re-measured model by model (raw-parity.md): CR3, compressed ORF and verification entries carry measured shares; HE NEF no longer a beta blocker (+1.1 points for 20–40 h); verification entry absorbs the cheap per-body fixes (#769, #702, A200 series) |
| 2026-10-10 | minor | Beta total raised to 340–600 h to include the ~92 open core-path user issues |
| 2026-10-10 | major | Created from the checklist's Top gaps (whose items 1–2 had been duplicated by a merge) and ROADMAP's Where we stand; re-ranked; added evidence, impact, hours and beta flags; added platform-CI, lens-database, HE NEF and localization gaps |

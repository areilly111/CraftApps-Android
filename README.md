# CraftApps for Android

Unofficial Android ports of the Craft suite by the ArtCraft team
([storytold](https://github.com/storytold)), rebuilt for tablets with
full S Pen support. Each app is a separate APK sharing one codebase of
Android plumbing: a file-based SAF picker bridge, a shared pen runtime,
a GPU cascade, and shared suite storage under `/sdcard/CraftApps/`.

## Apps

| App | Port version | What it is |
|---|---|---|
| **PhotoCraft** | 0.4.6 | Raster image editor (Photoshop-style) |
| **VectorCraft** | 0.2.2 | Vector graphics editor |
| **LightCraft** | 0.9.5 | Photo library & lighting editor |
| **DesignCraft** | 0.9.3 | Layout / design tool |
| **PdfCraft** | 0.2.3 | PDF viewer & editor |
| **FilmCraft** | 0.1.2 | Video editor |
| **EffectCraft** | 0.1.1 | Effects / compositing |
| **SoundCraft** | 0.1.1 | Audio workstation (DAW) |
| **CadCraft** | 0.1.0 | CAD and drafting tool (AutoCAD-style) |

## Versioning

Two version lines, kept separate:

- **`v<x>`** — the official app version from the upstream source
  (the ArtCraft team's own releases).
- **`P<x>`** — this Android port's version. Suite releases are
  tagged `P<version>` (current: **P0.1.0**); each app also carries
  its own port version (above), which is what its APK reports.

APKs are attached to the [latest release](../../releases/latest).
Install over the top to update — package names and signing keys are
stable across builds.

## Android features

- **S Pen support** — pressure, eraser end, palm rejection, and the
  barrel button, *including while hovering* (the button press wakes
  the app via a pen input pump, so remapped actions fire immediately).
  Buttons and gestures are remappable in Stylus Settings.
- **File open/save/folders** through the Android system picker
  (Storage Access Framework) via a file-based bridge — the same
  method in every app. Opened files are copied to an in-app inbox;
  saves write where you picked.
- **Hardware acceleration** — Vulkan → GLES → CPU cascade.
- **Immersive fullscreen** — the system dock/bars stay out of the way.
- **Shared storage** — `/sdcard/CraftApps/Media/` for work,
  `/sdcard/CraftApps/Settings/<App>/` for settings. Apps ask for
  All-files access on first run; libraries and folders are used as
  real filesystem paths.

## Known issues (October 2026 builds)

- **DesignCraft**: saving can crash the app — under investigation.
  Open files first, save early.
- **EffectCraft**: cold start shows a black screen for a few seconds
  before the UI appears; it works normally after that.
- **SoundCraft**: microphone recording goes through AAudio; report
  back if your device delivers no audio.

## Building

Builds run on a machine with the Rust toolchain, Android SDK + NDK 27
and JDK 17:

```
./scripts/build-apk.sh <app>   # photocraft, vectorcraft, lightcraft,
                               # designcraft, pdfcraft, filmcraft,
                               # effectcraft, soundcraft, cadcraft
```

The script vendors and patches `android-activity` (stylus support),
builds the app's Rust crate for `aarch64-linux-android`, page-aligns
the native libraries, and signs with the suite keystore (not in this
repo — see `AGENTS.md` / `PATCHES.md` for the port's patch notes).

## Credits & licence

Unofficial Android port — based on PhotoCraft, VectorCraft,
LightCraft, DesignCraft, PdfCraft, FilmCraft, EffectCraft,
SoundCraft and CadCraft by the ArtCraft team. Upstream sources live under each
app's `upstream/` directory with their own licences
(Apache-2.0 / MIT). The Android port code is original work.

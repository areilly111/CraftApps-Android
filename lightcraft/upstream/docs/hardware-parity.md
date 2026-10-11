# Hardware parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (created from the code, gpu-pipeline.md and the parity checklist) · **Target:** Adobe Lightroom Classic 15.5 / Lightroom 9.6

What Lightroom does with the machine's hardware, per platform, and what LightCraft does. Sources:
[gpu-pipeline.md](gpu-pipeline.md), [display-profiles.md](display-profiles.md), [denoise.md](denoise.md),
[ai-masks.md](ai-masks.md), `crates/gpu`, `crates/engine/src/devices.rs`, `crates/ui-egui/src/panels/second.rs`.

| Feature | Target | LightCraft: macOS | Windows | Linux / BSD | Web | Status |
|---|---|---|---|---|---|---|
| GPU for display and editing | Metal / DX12, "Use GPU for display / image processing / export" | wgpu Metal: geometry, resize, blur, WB, NR, guided filter, dehaze, masks, finish; CPU fallback per render | DX12 only (Vulkan opt-in, #136) | Vulkan | CPU only (no WebGPU yet) | 🟡 defringe, spot removal, heuristic masks, long brushes and high-bit exports stay on CPU |
| GPU export | ✅ | ✅ (6000 × 4000 in ~0.3 s vs ~1.2 s CPU) | ✅ | ✅ | — | ✅ (black-export fix on an Intel iGPU, #78) |
| AI on GPU / neural engine | Metal / DirectML / Core ML, model per feature | AI denoise on GPU or CPU (setting); SAM 3 on Metal via candle; faces on CPU | CPU / GPU denoise | CPU / GPU denoise | — | 🟡 no Core ML / Neural Engine / DirectML |
| HDR (EDR) display | ✅ (macOS EDR, Windows HDR) | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ (LR-VIEW-HDR-DISPLAY) |
| Monitor profile | automatic per display, follows the window | 🟡 picked by hand | 🟡 | 🟡 | — | 🟡 OS profile not read; no follow across monitors |
| Second window / second monitor | ✅ (Classic) | ✅ (⌘F11) | ✅ | ✅ | — | ✅ (Classic secondary-window keys missing) |
| Pen pressure / tilt (brush masks) | ✅ pressure for size / flow | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| Trackpad gestures | pinch zoom, pan | ✅ pinch, two-finger pan | 🟡 (winit) | 🟡 | 🟡 | ✅ on macOS |
| Card readers / cameras as import sources | ✅ (devices panel, eject after import) | ✅ mounted volumes with DCIM | ✅ | ✅ | — | 🟡 no eject-after-import check, no MTP / PTP phones |
| Tethered capture | ✅ Classic (Canon, Nikon, Sony, Leica) | ⬜ | ⬜ | ⬜ | — | ⬜ |
| Printers | ✅ Print module, printer colour management | ⬜ (PDF contact sheets only) | ⬜ | ⬜ | — | ⬜ |
| Control surfaces (MIDI / Loupedeck via plug-ins) | via SDK plug-ins | control channel and MCP can drive every command | same | same | — | 🟡 different mechanism; no MIDI mapping |
| Multi-core CPU | ✅ | ✅ worker pools, parallel JPEG decode, parallel import copy | ✅ | ✅ | single thread | ✅ |
| Memory pressure | managed | ✅ memory budget, `crates/sysmem` returns allocator pages on macOS | ✅ budget | ✅ budget | — | ✅ |

## Score

**~40% ready** (estimated): the GPU editing path is real and fast (measured: 24 MP exposure drag
3.9 ms on GPU vs 22–42 ms CPU; NR drag 18 ms vs 301–471 ms), but HDR display, pen pressure,
tethering, printing, automatic monitor profiles and neural-engine inference are missing. GPU
correctness is proven on Apple GPUs; other vendors rely on user reports.

## Remaining (Opus 5.5 agent hours)

| Work | Hours | Needs |
|---|---:|---|
| HDR display (EDR on macOS, Windows HDR, Linux where possible) | 10–20 | an HDR display to verify (human) |
| OS monitor profile, follow window across displays | 5–10 | multi-monitor setup |
| Pen pressure for brush masks (winit tablet events or a pure-Rust tablet layer) | 6–12 | a tablet |
| Remaining stages on GPU (defringe, spot removal, masks) | 10–20 | |
| Tethered capture (PTP over USB in pure Rust, Canon / Nikon / Sony first) | 25–45 | cameras (human) |
| Printing with colour management | 15–25 | printers (human) |
| Neural-engine / DirectML inference for AI features | 10–20 | depends on the AI model decision |
| **Total** | **80–150 h** (the core set for beta, HDR display + monitor profile + pen pressure: 20–40 h) | |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | Created |

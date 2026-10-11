//! GPU backend cascade for Android: Vulkan → OpenGL ES → CPU canvas.
//!
//! Written for the `CraftApps-Android` shared crate: no app identity is hardcoded. Per-app
//! values arrive via [`GpuAppInfo`]; the canvas attach step goes through [`GpuCanvasHost`],
//! which each port implements for its app type (a few lines of glue).
//!
//! Vulkan stays the default (the tablet runs great on it). When no Vulkan adapter exists
//! (older/low-end devices) the instance falls back to OpenGL ES; when neither exists the
//! window still comes up on a software adapter and the canvas composites on the CPU — the
//! same CPU-canvas path the web build uses (`<APP>_CPU_CANVAS=1`, e.g.
//! `PHOTOCRAFT_CPU_CANVAS=1`).
//!
//! Feature unification (verified 2026-10-10 against the eframe 0.36.2 / egui-wgpu 0.36.0 /
//! wgpu 30.0.1 sources): eframe's `wgpu` feature → `egui-wgpu/default` → `wgpu/default`,
//! and `wgpu/default` enables the `vulkan` AND `gles` backend features — so both backends
//! are compiled into the Android binary. egui-wgpu's default `WgpuSetupCreateNew` requests
//! `PRIMARY | GL`; this module makes the backends explicit (`VULKAN | GL`) and orders the
//! adapter pick Vulkan-first instead of relying on wgpu's enumeration order.
//!
//! The shared Preferences › Performance › GPU backend selector (`performance.gpuBackend`:
//! auto/vulkan/dx12/metal/gl/cpu, "applies at next launch") is honoured: it is read here,
//! before the wgpu instance exists, from the same `preferences.json` the services persist.
//! `vulkan` restricts the instance to Vulkan, `gl` to GLES, everything else (including
//! `auto`) gets the full cascade. `cpu` (or RenderingMode::Cpu) keeps the window on wgpu
//! but leaves the CPU canvas in place.

//! (cont.) The cascade itself is wired by the GPU-backend track, not the storage
//! track; the items below are intentionally unused until then.

use std::path::Path;
use std::sync::Arc;

/// Per-app identity for the GPU cascade. Each port fills this once.
pub struct GpuAppInfo {
    /// Display name, for logs (e.g. `"PhotoCraft"`).
    pub app_name: &'static str,
    /// Env var forcing the CPU canvas (e.g. `"PHOTOCRAFT_CPU_CANVAS"`).
    pub cpu_canvas_env: &'static str,
}

/// Per-app hooks for the canvas attach step. Each port implements this for its app type;
/// the cascade logic stays shared.
pub trait GpuCanvasHost {
    /// Record the chosen adapter (e.g. into the app's GPU info panel).
    fn record_adapter(&mut self, info: &eframe::wgpu::AdapterInfo);
    /// Note that the canvas fell back to the CPU (software adapter).
    fn record_software_fallback(&mut self, note: &str);
    /// Hand the wgpu render state to the app for GPU canvas compositing.
    fn set_gpu_canvas(&mut self, rs: eframe::egui_wgpu::RenderState);
    /// Watch the device for errors instead of letting them panic (the window still draws
    /// with wgpu even when the canvas stays on the CPU).
    fn watch_device(&mut self, device: &eframe::wgpu::Device);
}

/// Startup GPU plan, derived from the persisted prefs before eframe exists.
pub struct GpuPlan {
    /// Backends the wgpu instance may enumerate.
    pub backends: eframe::wgpu::Backends,
    /// Whether the app's canvas may use the GPU compositor. The eframe window always
    /// needs a wgpu adapter; this only gates [`attach_canvas`]. Mirrors the desktop's
    /// `main.rs` canvas decision.
    pub allow_gpu_canvas: bool,
}

/// Build the plan from the persisted `preferences.json`.
///
/// Best effort: a missing or corrupt file means Auto/Auto — the default cascade
/// (Vulkan → GLES → CPU canvas). Only the two performance prefs are read; the rest of
/// the file is ignored.
pub fn plan(prefs_path: &Path, info: &GpuAppInfo) -> GpuPlan {
    let (gpu_backend, rendering_mode, use_gpu) = read_perf_prefs(prefs_path);
    let backends = match gpu_backend.as_str() {
        // Explicit user override: restrict the instance to that backend.
        "vulkan" => eframe::wgpu::Backends::VULKAN,
        "gl" => eframe::wgpu::Backends::GL,
        // dx12/metal don't exist on Android; cpu still needs a wgpu window. Auto and
        // everything else: the full cascade.
        _ => eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL,
    };
    // Mirrors `Performance::effective_rendering_mode`: an explicit rendering mode wins,
    // otherwise the legacy flags resolve.
    let effective_cpu = rendering_mode.as_deref() == Some("cpu")
        || (rendering_mode.as_deref() != Some("gpu") && (!use_gpu || gpu_backend == "cpu"));
    let allow_gpu_canvas = !effective_cpu && std::env::var_os(info.cpu_canvas_env).is_none();
    log::info!(
        "{} GPU plan: backends={:?} gpu_canvas={}",
        info.app_name,
        backends,
        allow_gpu_canvas
    );
    GpuPlan { backends, allow_gpu_canvas }
}

/// Read `performance.gpuBackend`, `performance.renderingMode` and `performance.useGpu`
/// from the prefs JSON. Defaults match a fresh install (auto/auto/true).
fn read_perf_prefs(prefs_path: &Path) -> (String, Option<String>, bool) {
    let (mut backend, mut mode, mut use_gpu) = ("auto".to_string(), None, true);
    let Ok(text) = std::fs::read_to_string(prefs_path) else {
        return (backend, mode, use_gpu);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (backend, mode, use_gpu);
    };
    if let Some(perf) = v.get("performance") {
        if let Some(b) = perf.get("gpuBackend").and_then(|b| b.as_str()) {
            backend = b.to_string();
        }
        mode = perf.get("renderingMode").and_then(|m| m.as_str()).map(str::to_string);
        if let Some(u) = perf.get("useGpu").and_then(|u| u.as_bool()) {
            use_gpu = u;
        }
    }
    (backend, mode, use_gpu)
}

/// Install the cascade on the eframe options: explicit instance backends plus an adapter
/// selector that orders Vulkan → GLES → any non-CPU adapter → anything.
///
/// The trailing "anything" is the SwiftShader/software fallback: the window still comes
/// up, and [`attach_canvas`] then keeps the CPU canvas instead of handing a software
/// adapter to the GPU compositor.
pub fn configure(options: &mut eframe::NativeOptions, plan: &GpuPlan) {
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = plan.backends;
        create.native_adapter_selector = Some(Arc::new(
            |adapters: &[eframe::wgpu::Adapter], surface: Option<&eframe::wgpu::Surface<'_>>| {
                let supports = |a: &&eframe::wgpu::Adapter| surface.is_none_or(|s| a.is_surface_supported(s));
                let is_hardware = |a: &&eframe::wgpu::Adapter| {
                    a.get_info().device_type != eframe::wgpu::DeviceType::Cpu
                };
                let by_backend = |b| adapters.iter().filter(supports).find(|a| a.get_info().backend == b && is_hardware(a));
                let pick = by_backend(eframe::wgpu::Backend::Vulkan)
                    .or_else(|| by_backend(eframe::wgpu::Backend::Gl))
                    .or_else(|| adapters.iter().filter(supports).find(is_hardware))
                    .or_else(|| adapters.iter().find(supports));
                match pick {
                    Some(a) => Ok(a.clone()),
                    None => Err("no graphics adapter found".to_string()),
                }
            },
        ));
    }
}

/// Attach the wgpu render state to the app for GPU canvas compositing, or keep the CPU
/// canvas. Mirrors the desktop `main.rs` decision: a software adapter (SwiftShader, or a
/// `DeviceType::Cpu` device) never drives the GPU compositor — the window draws with it,
/// the canvas composites on the CPU.
pub fn attach_canvas(
    app: &mut impl GpuCanvasHost,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    plan: &GpuPlan,
) {
    let Some(rs) = render_state else { return };
    let info = rs.adapter.get_info();
    app.record_adapter(&info);
    let software_window = info.device_type == eframe::wgpu::DeviceType::Cpu;
    if plan.allow_gpu_canvas && !software_window {
        app.set_gpu_canvas(rs);
    } else {
        if software_window {
            app.record_software_fallback("No hardware graphics adapter; the canvas composites on the CPU.");
        }
        app.watch_device(&rs.device);
    }
}

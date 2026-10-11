// Advanced 3D on the GPU after the rasteriser (`advanced3d.wgsl`): the steps of
// `effectcraft_render::three_d::adv::render_prepared` on GPU-resident buffers, operation for
// operation:
//
//   resolve   box-filter the 2×2 supersampled colour and camera depth of one motion-blur
//             sub-sample and add it to the accumulation (weight 1/n; depth = nearest)
//   radius    each pixel's depth-of-field blur radius (`Dof::coc` · scale / 2, ≤ 48) and the
//             range over the frame (atomics on the float bits; radii are ≥ 0)
//   boost     Highlight Gain / Threshold / Saturation (`bokeh::boost_highlights`), blended in
//             where the pixel is blurred
//   prefix    per-row prefix sums (`bokeh::row_prefix`)
//   gather    the iris-shaped gather at the two blur levels around each pixel's radius
//             (`bokeh::span_gather` on `bokeh::kernel` spans) blended (`progressive_blur`)
//   finish    linear → sRGB (`adv::encode`) and premultiplied "over" the canvas (`draw_run`)
//   wire      Wireframe-quality outlines: set the listed pixels to opaque white
//
// Depth "nothing drawn" is BIG on the GPU (∞ on the CPU).

struct Post {
    u0: vec4<u32>,
    u1: vec4<u32>,
    f0: vec4<f32>,
    f1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> P: Post;
@group(0) @binding(1) var color_in: texture_2d<f32>;
// Camera depth as f32 bits (an `R32Uint` target: integer targets render on every backend,
// `R32Float` doesn't on GLES without EXT_color_buffer_float).
@group(0) @binding(2) var z_in: texture_2d<u32>;
@group(0) @binding(3) var<storage, read_write> acc: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> depth: array<f32>;
@group(0) @binding(5) var<storage, read_write> src: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read> table: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read_write> radius: array<f32>;
@group(0) @binding(8) var<storage, read_write> range: array<atomic<u32>>;
@group(0) @binding(9) var<storage, read_write> prefix: array<vec4<f32>>;
@group(0) @binding(10) var out: texture_storage_2d<rgba32float, write>;

const BIG: f32 = 3.0e38;
const MAX_RADIUS: f32 = 48.0;
const NEAR: f32 = 1.0;

// P.u0 = (width, height, ssaa, first), P.f0.x = weight
@compute @workgroup_size(16, 16)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    let h = P.u0.y;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let k = P.u0.z;
    var a = vec4<f32>(0.0);
    var dz = 0.0;
    var dn = 0.0;
    for (var j = 0u; j < k; j = j + 1u) {
        for (var i = 0u; i < k; i = i + 1u) {
            let p = vec2<i32>(i32(id.x * k + i), i32(id.y * k + j));
            a = a + textureLoad(color_in, p, 0);
            let z = bitcast<f32>(textureLoad(z_in, p, 0).x);
            if (z >= 0.0) {
                dz = dz + z;
                dn = dn + 1.0;
            }
        }
    }
    let n = f32(k * k);
    let res = a / n;
    var d = BIG;
    if (dn > 0.0) {
        d = dz / dn;
    }
    let ix = id.y * w + id.x;
    let wt = P.f0.x;
    if (P.u0.w == 1u) {
        acc[ix] = res * wt;
        depth[ix] = d;
    } else {
        acc[ix] = acc[ix] + res * wt;
        depth[ix] = min(depth[ix], d);
    }
}

// P.u0 = (width, height), P.f0 = (focus, aperture, blur level, scale)
@compute @workgroup_size(16, 16)
fn radius_of(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    if (id.x >= w || id.y >= P.u0.y) {
        return;
    }
    let ix = id.y * w + id.x;
    var z = depth[ix];
    if (z >= BIG * 0.5) {
        z = 1.0e9;
    }
    var coc = 0.0;
    if (z > NEAR && P.f0.x > 0.0) {
        coc = P.f0.y * abs(z - P.f0.x) / z * P.f0.z;
    }
    let r = min(coc * P.f0.w * 0.5, MAX_RADIUS);
    radius[ix] = r;
    atomicMin(&range[0], bitcast<u32>(r));
    atomicMax(&range[1], bitcast<u32>(r));
}

fn luminance(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

// P.u0 = (width, height), P.f0 = (gain, threshold, saturation)
@compute @workgroup_size(16, 16)
fn boost(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    if (id.x >= w || id.y >= P.u0.y) {
        return;
    }
    let ix = id.y * w + id.x;
    let p = acc[ix];
    var b = p;
    let gain = P.f0.x;
    if (gain > 0.0 && p.w > 0.0) {
        let a = p.w;
        let c = p.xyz / a;
        let l = luminance(c);
        let thr = clamp(P.f0.y, 0.0, 1.0);
        if (!(l < thr || (thr >= 1.0 && l < 1.0))) {
            var over = 1.0;
            if (thr < 1.0) {
                over = clamp((l - thr) / (1.0 - thr), 0.0, 1.0);
            }
            let k = 1.0 + gain * 4.0 * max(over, 0.25);
            let sat = 1.0 + P.f0.z;
            let s = max(vec3<f32>(l) + (c - vec3<f32>(l)) * sat, vec3<f32>(0.0)) * k;
            b = vec4<f32>(s * a, a);
        }
        let kk = clamp(radius[ix] - 0.5, 0.0, 1.0);
        b = p + (b - p) * kk;
    }
    src[ix] = b;
}

// One invocation per row. P.u0 = (width, height)
@compute @workgroup_size(64)
fn prefix_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    let y = id.x;
    if (y >= P.u0.y) {
        return;
    }
    let base = y * (w + 1u);
    var a = vec4<f32>(0.0);
    prefix[base] = a;
    for (var x = 0u; x < w; x = x + 1u) {
        a = a + src[y * w + x];
        prefix[base + x + 1u] = a;
    }
}

fn prefix_at(y: u32, t: f32) -> vec4<f32> {
    let n = P.u0.x;
    let base = y * (n + 1u);
    if (t <= 0.0) {
        return vec4<f32>(0.0);
    }
    if (t >= f32(n)) {
        return prefix[base + n];
    }
    let i = u32(floor(t));
    let f = t - f32(i);
    let a = prefix[base + i];
    let b = prefix[base + i + 1u];
    return a + (b - a) * f;
}

// Level `j` of the table: (first span, span count, 1 / norm, identity).
fn level(j: u32, x: u32, y: u32) -> vec4<f32> {
    let w = P.u0.x;
    let h = i32(P.u0.y);
    let l = table[j];
    if (l.w > 0.5) {
        return src[y * w + x];
    }
    let first = u32(l.x);
    let count = u32(l.y);
    var a = vec4<f32>(0.0);
    for (var s = first; s < first + count; s = s + 1u) {
        let sp = table[s];
        let sy = i32(y) + i32(sp.x);
        if (sy < 0 || sy >= h) {
            continue;
        }
        let hi = prefix_at(u32(sy), f32(x) + sp.z + 0.5);
        let lo = prefix_at(u32(sy), f32(x) + sp.y + 0.5);
        a = a + (hi - lo) * sp.w;
    }
    return a * l.z;
}

// P.u0 = (width, height, levels), P.f0 = (lo, hi)
@compute @workgroup_size(16, 16)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    if (id.x >= w || id.y >= P.u0.y) {
        return;
    }
    let ix = id.y * w + id.x;
    let n = P.u0.z;
    if (n <= 1u) {
        acc[ix] = level(0u, id.x, id.y);
        return;
    }
    let lo = P.f0.x;
    let hi = P.f0.y;
    let r = radius[ix];
    let f = clamp((r - lo) / (hi - lo) * f32(n - 1u), 0.0, f32(n - 1u));
    let k = min(u32(floor(f)), n - 2u);
    let t = f - f32(k);
    let a = level(k, id.x, id.y);
    let b = level(k + 1u, id.x, id.y);
    acc[ix] = a + (b - a) * t;
}

fn linear_to_srgb(c: f32) -> f32 {
    if (c <= 0.0031308) {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

// P.u0 = (width, height, encode, over the canvas in color_in)
@compute @workgroup_size(16, 16)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    if (id.x >= w || id.y >= P.u0.y) {
        return;
    }
    var p = acc[id.y * w + id.x];
    if (P.u0.z == 1u && p.w > 1e-6) {
        let a = p.w;
        p = vec4<f32>(linear_to_srgb(p.x / a) * a, linear_to_srgb(p.y / a) * a, linear_to_srgb(p.z / a) * a, a);
    }
    let q = vec2<i32>(i32(id.x), i32(id.y));
    if (P.u0.w == 1u) {
        let d = textureLoad(color_in, q, 0);
        let k = 1.0 - p.w;
        p = vec4<f32>(p.x + d.x * k, p.y + d.y * k, p.z + d.z * k, p.w + d.w * k);
    }
    textureStore(out, q, p);
}

// One invocation per pixel listed in `table` (x, y). P.u0.x = count
@compute @workgroup_size(64)
fn wire(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.u0.x) {
        return;
    }
    let p = table[id.x];
    textureStore(out, vec2<i32>(i32(p.x), i32(p.y)), vec4<f32>(1.0));
}

// A layer rendered on its own (color_in, finished) hidden where the run's main scene is nearer
// (`adv::draw_run`): depth = its depth, src = the main scene's colour, radius = the main
// scene's depth. Pixels it didn't draw (BIG) are never hidden, as ∞ compares on the CPU.
// P.u0 = (width, height)
@compute @workgroup_size(16, 16)
fn occlude(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = P.u0.x;
    if (id.x >= w || id.y >= P.u0.y) {
        return;
    }
    let i = id.y * w + id.x;
    let q = vec2<i32>(i32(id.x), i32(id.y));
    var p = textureLoad(color_in, q, 0);
    let z = depth[i];
    let mz = radius[i];
    if (z < BIG * 0.5 && mz < z - 0.01 * max(abs(z), 1.0)) {
        p *= 1.0 - clamp(src[i].w, 0.0, 1.0);
    }
    textureStore(out, q, p);
}

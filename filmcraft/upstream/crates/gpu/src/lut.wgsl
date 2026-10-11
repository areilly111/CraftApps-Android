// 3D LUT application with tetrahedral interpolation (same math as filmcraft_color::Lut3d::apply).
// Pixels are straight (unpremultiplied) display-encoded RGBA; alpha passes through.

struct Params {
    size: u32,
    count: u32,
    _pad0: u32,
    _pad1: u32,
    dmin: vec4<f32>,
    dmax: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> lut: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> dst: array<vec4<f32>>;

fn at(r: u32, g: u32, b: u32) -> vec3<f32> {
    let n = params.size;
    return lut[r + g * n + b * n * n].xyz;
}

fn tetra(c: vec3<f32>) -> vec3<f32> {
    let n = params.size;
    let m = f32(n - 1u);
    let t = clamp((c - params.dmin.xyz) / (params.dmax.xyz - params.dmin.xyz), vec3(0.0), vec3(1.0)) * m;
    let i = min(vec3<u32>(t), vec3(n - 2u));
    let f = t - vec3<f32>(i);
    let fr = f.x;
    let fg = f.y;
    let fb = f.z;
    let c000 = at(i.x, i.y, i.z);
    let c111 = at(i.x + 1u, i.y + 1u, i.z + 1u);
    var ca: vec3<f32>;
    var cb: vec3<f32>;
    var w: vec4<f32>;
    if (fr > fg) {
        if (fg > fb) {
            ca = at(i.x + 1u, i.y, i.z); cb = at(i.x + 1u, i.y + 1u, i.z); w = vec4(1.0 - fr, fr - fg, fg - fb, fb);
        } else if (fr > fb) {
            ca = at(i.x + 1u, i.y, i.z); cb = at(i.x + 1u, i.y, i.z + 1u); w = vec4(1.0 - fr, fr - fb, fb - fg, fg);
        } else {
            ca = at(i.x, i.y, i.z + 1u); cb = at(i.x + 1u, i.y, i.z + 1u); w = vec4(1.0 - fb, fb - fr, fr - fg, fg);
        }
    } else if (fb > fg) {
        ca = at(i.x, i.y, i.z + 1u); cb = at(i.x, i.y + 1u, i.z + 1u); w = vec4(1.0 - fb, fb - fg, fg - fr, fr);
    } else if (fb > fr) {
        ca = at(i.x, i.y + 1u, i.z); cb = at(i.x, i.y + 1u, i.z + 1u); w = vec4(1.0 - fg, fg - fb, fb - fr, fr);
    } else {
        ca = at(i.x, i.y + 1u, i.z); cb = at(i.x + 1u, i.y + 1u, i.z); w = vec4(1.0 - fg, fg - fr, fr - fb, fb);
    }
    return w.x * c000 + w.y * ca + w.z * cb + w.w * c111;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x + id.y * 65535u * 64u;
    if (k >= params.count) {
        return;
    }
    let p = src[k];
    dst[k] = vec4(tetra(p.xyz), p.w);
}

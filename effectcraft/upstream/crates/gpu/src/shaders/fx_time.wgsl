// GPU effects, time: see src/fx_time.rs. The frames come from the effect host (fetched and
// placed on one grid on the CPU, as Echo's); these kernels combine them as the CPU effects do.

fn ftm_inside(p: vec2<i32>) -> bool {
    let d = out_dims();
    return p.x < d.x && p.y < d.y;
}

// util::unpremul: (straight colour, alpha).
fn ftm_unpremul(p: vec4<f32>) -> vec4<f32> {
    if (p.w > 1e-6) {
        return vec4<f32>(p.xyz / p.w, p.w);
    }
    return vec4<f32>(0.0, 0.0, 0.0, max(p.w, 0.0));
}

// Weighted sum, one frame per pass (time_fx::average): src + aux × f[0].x.
@compute @workgroup_size(16, 16)
fn ftm_acc(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (!ftm_inside(p)) {
        return;
    }
    textureStore(out, p, textureLoad(src, p, 0) + textureLoad(aux, p, 0) * P.f[0].x);
}

// Cross-fade (raster::flow::mix): src + (aux − src) × f[0].x.
@compute @workgroup_size(16, 16)
fn ftm_mix(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (!ftm_inside(p)) {
        return;
    }
    let a = textureLoad(src, p, 0);
    textureStore(out, p, a + (textureLoad(aux, p, 0) - a) * P.f[0].x);
}

// Timewarp's Matte Layer with Pixel Motion (time_fx::layered): src = foreground, aux =
// background; u[0].x = Show (1 foreground, 2 background, 3 the foreground's alpha as a matte,
// else foreground over background).
@compute @workgroup_size(16, 16)
fn ftm_layer(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (!ftm_inside(p)) {
        return;
    }
    let f = textureLoad(src, p, 0);
    let b = textureLoad(aux, p, 0);
    var o = f + b * (1.0 - f.w);
    switch (P.u[0].x) {
        case 1u: {
            o = f;
        }
        case 2u: {
            o = b;
        }
        case 3u: {
            o = vec4<f32>(f.w, f.w, f.w, 1.0);
        }
        default: {}
    }
    textureStore(out, p, o);
}

// ---------------------------------------------------------------- Time Difference (time_fx::time_difference)
// src = the layer now, aux = the target. u[0] = (absolute difference, alpha channel);
// f[0].x = contrast gain.

@compute @workgroup_size(16, 16)
fn ftm_difference(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (!ftm_inside(p)) {
        return;
    }
    let cu = ftm_unpremul(textureLoad(src, p, 0));
    let tu = ftm_unpremul(textureLoad(aux, p, 0));
    let c = cu.xyz;
    let d = tu.xyz;
    let ca = cu.w;
    let da = tu.w;
    let gain = P.f[0].x;
    let diff = c - d;
    var rgb = clamp(0.5 + diff * 0.5 * gain, vec3<f32>(0.0), vec3<f32>(1.0));
    if (P.u[0].x != 0u) {
        rgb = clamp(abs(diff) * gain, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let mx = max(max(rgb.x, rgb.y), rgb.z);
    let light = (mx + min(min(rgb.x, rgb.y), rgb.z)) * 0.5;
    var a: f32;
    switch P.u[0].y {
        case 1u: {
            a = da;
        }
        case 2u: {
            a = (ca + da) * 0.5;
        }
        case 3u: {
            a = max(ca, da);
        }
        case 4u: {
            a = 1.0;
        }
        case 5u: {
            a = light;
        }
        case 6u: {
            a = mx;
        }
        case 7u: {
            a = abs(ca - da) * gain;
        }
        case 8u: {
            a = clamp(abs(ca - da) * gain, 0.0, 1.0);
            rgb = vec3<f32>(1.0);
        }
        default: {
            a = ca;
        }
    }
    a = clamp(a, 0.0, 1.0);
    textureStore(out, p, vec4<f32>(rgb * a, a));
}

// ---------------------------------------------------------------- Time Displacement (time_fx::time_displacement)
// One pass per sampled time: pixels whose frame index (data, u32 per pixel, row length u[0].y)
// is u[0].x take frame aux at (x + dx, y + dy) (u[0].zw, i32), the others keep src.

@compute @workgroup_size(16, 16)
fn ftm_displace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (!ftm_inside(p)) {
        return;
    }
    let k = bitcast<u32>(data[u32(p.y) * P.u[0].y + u32(p.x)]);
    if (k == P.u[0].x) {
        textureStore(out, p, tex_get(aux, p.x + bitcast<i32>(P.u[0].z), p.y + bitcast<i32>(P.u[0].w)));
    } else {
        textureStore(out, p, textureLoad(src, p, 0));
    }
}

// ---------------------------------------------------------------- Timewarp Pixel Motion (time_fx::timewarp_interpolate)
// src = frame a, aux = frame b (both source-cropped); data = the motion vectors (x, y pairs).
// u[0] = (columns, rows, extreme filtering, build from one image); f[0] = (vector spacing,
// fraction w, error threshold).

// raster::flow::Flow::at.
fn ftm_flow_at(x: f32, y: f32) -> vec2<f32> {
    let cols = P.u[0].x;
    let rows = P.u[0].y;
    if (cols == 0u || rows == 0u) {
        return vec2<f32>(0.0);
    }
    let b = P.f[0].x;
    let fx = clamp(x / b - 0.5, 0.0, f32(cols - 1u));
    let fy = clamp(y / b - 0.5, 0.0, f32(rows - 1u));
    let x0 = u32(floor(fx));
    let y0 = u32(floor(fy));
    let x1 = min(x0 + 1u, cols - 1u);
    let y1 = min(y0 + 1u, rows - 1u);
    let tx = fx - f32(x0);
    let ty = fy - f32(y0);
    let g00 = vec2<f32>(data[(y0 * cols + x0) * 2u], data[(y0 * cols + x0) * 2u + 1u]);
    let g10 = vec2<f32>(data[(y0 * cols + x1) * 2u], data[(y0 * cols + x1) * 2u + 1u]);
    let g01 = vec2<f32>(data[(y1 * cols + x0) * 2u], data[(y1 * cols + x0) * 2u + 1u]);
    let g11 = vec2<f32>(data[(y1 * cols + x1) * 2u], data[(y1 * cols + x1) * 2u + 1u]);
    let top = g00 + (g10 - g00) * tx;
    let bot = g01 + (g11 - g01) * tx;
    return top + (bot - top) * ty;
}

fn ftm_sample(t: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    if (P.u[0].z != 0u) {
        return sample_bicubic(t, x, y);
    }
    return sample_bilinear_clamped(t, x, y);
}

@compute @workgroup_size(16, 16)
fn ftm_motion(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (!ftm_inside(p)) {
        return;
    }
    let cx = f32(p.x) + 0.5;
    let cy = f32(p.y) + 0.5;
    let f = ftm_flow_at(cx, cy);
    let w = P.f[0].y;
    var pa = ftm_sample(src, cx - w * f.x, cy - w * f.y);
    var pb = ftm_sample(aux, cx + (1.0 - w) * f.x, cy + (1.0 - w) * f.y);
    if (P.u[0].w != 0u) {
        textureStore(out, p, select(pb, pa, w < 0.5));
        return;
    }
    let d = abs(pa - pb);
    let err = max(max(d.x, d.y), max(d.z, d.w));
    let thr = P.f[0].z;
    if (thr > 0.0 && err > thr && thr < 1.0) {
        pa = textureLoad(src, p, 0);
        pb = textureLoad(aux, p, 0);
    }
    textureStore(out, p, pa + (pb - pa) * w);
}

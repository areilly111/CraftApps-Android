// ---------------------------------------------------------------- particle render passes (fx_particles.rs)
//
// fxp_raster composites tiled items (the layout of fxm_raster: data = tile offsets, item
// indices, then the records, u[0].w floats each, the last four the item's integer pixel bounds)
// over a base image (aux), so large plans draw in several passes. Every shading step mirrors
// effects::sim / sim2 / sim3.

// One texel of a Layer Map frame in the atlas (src): transparent outside the frame, as
// Image::sample_bilinear reads outside an image. Record fields 14..17 = atlas x, y, frame w, h.
fn fxp_atlas_get(at: u32, x: i32, y: i32) -> vec4<f32> {
    if (x < 0 || y < 0 || x >= i32(data[at + 16u]) || y >= i32(data[at + 17u])) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, vec2<i32>(x + i32(data[at + 14u]), y + i32(data[at + 15u])), 0);
}

// Image::sample_bilinear of a Layer Map frame.
fn fxp_atlas_bilinear(at: u32, x: f32, y: f32) -> vec4<f32> {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let a = fxp_atlas_get(at, xi, yi);
    let b = fxp_atlas_get(at, xi + 1, yi);
    let c = fxp_atlas_get(at, xi, yi + 1);
    let d = fxp_atlas_get(at, xi + 1, yi + 1);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    return top + (bot - top) * ty;
}

// sim3::blit_layer at a pixel centre. Record: kind, x, y, sin, cos (rotation), scale x, scale y,
// buffer scale, alpha, frame layer size w, h, frame buffer scale, offset x, y, atlas x, y,
// frame pixels w, h.
fn fxp_blit(at: u32, x: f32, y: f32) -> FxmShade {
    let bs = data[at + 7u];
    let dx = (x - data[at + 1u]) / bs;
    let dy = (y - data[at + 2u]) / bs;
    let sa = data[at + 3u];
    let ca = data[at + 4u];
    let sw = data[at + 9u];
    let sh = data[at + 10u];
    let u = (dx * ca + dy * sa) / data[at + 5u] + sw * 0.5;
    let v = (-dx * sa + dy * ca) / data[at + 6u] + sh * 0.5;
    if (u < 0.0 || v < 0.0 || u >= sw || v >= sh) {
        return fxm_none();
    }
    let fs = data[at + 11u];
    let s = fxp_atlas_bilinear(at, u * fs + data[at + 12u], v * fs + data[at + 13u]);
    if (s.w <= 0.0) {
        return fxm_none();
    }
    return FxmShade(true, s * data[at + 8u]);
}

// sim2::foam_disc's coverage and disc coordinates (u, v clamped to −1..1) at a pixel centre for
// the disc (x, y, r) in record fields 1..3: (u, v, coverage); coverage ≤ 0 = outside.
fn fxp_disc(at: u32, x: f32, y: f32) -> vec3<f32> {
    let r = data[at + 3u];
    let u = (x - data[at + 1u]) / r;
    let v = (y - data[at + 2u]) / r;
    let d = sqrt(u * u + v * v);
    let cov = clamp((1.0 - d) * r + 0.5, 0.0, 1.0);
    return vec3<f32>(clamp(u, -1.0, 1.0), clamp(v, -1.0, 1.0), cov);
}

// Foam's User Defined texture filling a bubble (src = the texture layer's buffer). Record:
// kind, x, y, r, sin, cos (rotation), fade, texture layer size w, h, buffer scale, offset x, y.
fn fxp_foam_tex(at: u32, x: f32, y: f32) -> FxmShade {
    let q = fxp_disc(at, x, y);
    if (q.z <= 0.0) {
        return fxm_none();
    }
    let sr = data[at + 4u];
    let cr = data[at + 5u];
    let tu = q.x * cr + q.y * sr;
    let tv = -q.x * sr + q.y * cr;
    let tx = (tu * 0.5 + 0.5) * data[at + 7u];
    let ty = (tv * 0.5 + 0.5) * data[at + 8u];
    let ts = data[at + 9u];
    let px = sample_bilinear(src, tx * ts + data[at + 10u], ty * ts + data[at + 11u]);
    return FxmShade(true, px * data[at + 6u] * q.z);
}

// Foam's Environment Map reflection on a bubble (src = the map fitted to the buffer). Record:
// kind, x, y, r, convergence, strength, buffer w, h.
fn fxp_foam_env(at: u32, x: f32, y: f32) -> FxmShade {
    let q = fxp_disc(at, x, y);
    if (q.z <= 0.0) {
        return fxm_none();
    }
    let bx = data[at + 1u];
    let by = data[at + 2u];
    let conv = data[at + 4u];
    var ex = bx;
    var ey = by;
    if (conv < 1.0) {
        ex = bx * conv + (q.x * 0.5 + 0.5) * data[at + 6u] * (1.0 - conv);
        ey = by * conv + (q.y * 0.5 + 0.5) * data[at + 7u] * (1.0 - conv);
    }
    let e = sample_bilinear(src, ex, ey);
    let rim = min(q.x * q.x + q.y * q.y, 1.0);
    let k = data[at + 5u] * (0.3 + 0.7 * rim);
    return FxmShade(true, vec4<f32>(e.xyz * k, 0.0) * q.z);
}

// Items over the base (aux). u[0] = (kind: 0 sprites, 1 Layer Map frames, 2 Foam texture discs,
// 3 Foam reflection discs, 4 pieces without a back texture; mode: 0 over, 1 add, 2 Foam's disc
// compositing; items offset; item stride); u[1].x = tiles across.
@compute @workgroup_size(16, 16)
fn fxp_raster(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = out_dims();
    let p = vec2<i32>(gid.xy);
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    let tile = u32(p.y / 16) * P.u[1].x + u32(p.x / 16);
    let start = u32(data[tile]);
    let end = u32(data[tile + 1u]);
    let stride = P.u[0].w;
    let x = f32(p.x) + 0.5;
    let y = f32(p.y) + 0.5;
    var d = textureLoad(aux, p, 0);
    for (var k = start; k < end; k++) {
        let at = P.u[0].z + u32(data[k]) * stride;
        let bb = at + stride - 4u;
        if (f32(p.x) < data[bb] || f32(p.y) < data[bb + 1u] || f32(p.x) > data[bb + 2u] || f32(p.y) > data[bb + 3u]) {
            continue;
        }
        var s: FxmShade;
        switch (P.u[0].x) {
            case 0u: {
                s = fxm_sprite(at, x, y);
            }
            case 1u: {
                s = fxp_blit(at, x, y);
            }
            case 2u: {
                s = fxp_foam_tex(at, x, y);
            }
            case 3u: {
                s = fxp_foam_env(at, x, y);
            }
            default: {
                s = fxm_piece(at, x, y);
            }
        }
        if (!s.ok) {
            continue;
        }
        let c = s.c;
        switch (P.u[0].y) {
            case 1u: {
                d = vec4<f32>(d.xyz + c.xyz, min(d.w + c.w, 1.0));
            }
            case 2u: {
                let kk = 1.0 - c.w;
                d = vec4<f32>(c.xyz + d.xyz * kk, min(c.w + d.w * kk, 1.0));
            }
            default: {
                d = c + d * (1.0 - c.w);
            }
        }
    }
    textureStore(out, p, d);
}

// Foam's Draft + Flow Map view: the flow map (aux) at half strength under the bubbles (src).
@compute @workgroup_size(16, 16)
fn fxp_under(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = out_dims();
    let p = vec2<i32>(gid.xy);
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    let o = textureLoad(src, p, 0);
    let f = textureLoad(aux, p, 0);
    textureStore(out, p, o + f * 0.5 * (1.0 - o.w));
}

// ---------------------------------------------------------------- Curl Noise (noise2::curl_noise)
//
// u[0] = (seed, flow steps, wrap edges); f[0] = (offset x, offset y, scale, complexity);
// f[1] = (cos rotation, sin rotation, evolution · 4, step length).

// The fBm potential at each pixel (in .x).
@compute @workgroup_size(16, 16)
fn fxp_curl_pot(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = out_dims();
    let p = vec2<i32>(gid.xy);
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    let dx = f32(p.x) + 0.5 - P.f[0].x;
    let dy = f32(p.y) + 0.5 - P.f[0].y;
    let cr = P.f[1].x;
    let sr = P.f[1].y;
    let u = (dx * cr + dy * sr) / P.f[0].z;
    let v = (-dx * sr + dy * cr) / P.f[0].z;
    textureStore(out, p, vec4<f32>(fxs_fbm(u, v, P.f[1].z, P.u[0].x, P.f[0].w), 0.0, 0.0, 0.0));
}

// The curl of the potential (src): (∂ψ/∂y, −∂ψ/∂x) · scale, edges clamped.
@compute @workgroup_size(16, 16)
fn fxp_curl_vel(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = out_dims();
    let p = vec2<i32>(gid.xy);
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    let gx = (tex_get_clamped(src, p.x + 1, p.y).x - tex_get_clamped(src, p.x - 1, p.y).x) * 0.5;
    let gy = (tex_get_clamped(src, p.x, p.y + 1).x - tex_get_clamped(src, p.x, p.y - 1).x) * 0.5;
    textureStore(out, p, vec4<f32>(gy * P.f[0].z, -gx * P.f[0].z, 0.0, 0.0));
}

// The Flow Field view of the velocities (src).
@compute @workgroup_size(16, 16)
fn fxp_curl_view(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = out_dims();
    let p = vec2<i32>(gid.xy);
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    let v = textureLoad(src, p, 0);
    textureStore(out, p, vec4<f32>(clamp(0.5 + v.x * 0.5, 0.0, 1.0), clamp(0.5 + v.y * 0.5, 0.0, 1.0), 0.5, 1.0));
}

// The layer (src) advected backwards along the velocities (aux).
@compute @workgroup_size(16, 16)
fn fxp_curl_flow(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = out_dims();
    let p = vec2<i32>(gid.xy);
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    var sx = f32(p.x) + 0.5;
    var sy = f32(p.y) + 0.5;
    let dt = P.f[1].w;
    for (var i = 0u; i < P.u[0].y; i++) {
        let vel = sample_bilinear_clamped(aux, sx, sy);
        sx -= vel.x * dt;
        sy -= vel.y * dt;
    }
    if (P.u[0].z != 0u) {
        let w = f32(dims.x);
        let h = f32(dims.y);
        sx = sx - w * floor(sx / w);
        sy = sy - h * floor(sy / h);
    }
    textureStore(out, p, sample_bilinear_clamped(src, sx, sy));
}

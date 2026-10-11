// GPU effects, Numbers and Timecode: see src/fx_text.rs. The glyph coverage is rasterised on
// the CPU (aux: fill in x, stroke ring in y, opacity applied); this kernel composites it as
// textfx::draw_plan does: the layer or transparency, Timecode's box, then fill and stroke.
// u[0] = (display options, keep the layer, box); f[0] = fill colour; f[1] = stroke colour;
// f[2] = box (x0, x1, y0, y1); f[3] = (box colour, box opacity).

fn ftx_over(dst: vec4<f32>, s: vec4<f32>) -> vec4<f32> {
    return s + dst * (1.0 - s.w);
}

// textfx::tint.
fn ftx_tint(c: vec4<f32>, a0: f32) -> vec4<f32> {
    let a = clamp(a0 * c.w, 0.0, 1.0);
    return vec4<f32>(c.xyz * a, a);
}

@compute @workgroup_size(16, 16)
fn ftx_text(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    let dims = out_dims();
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    var o = vec4<f32>(0.0);
    if (P.u[0].y != 0u) {
        o = textureLoad(src, p, 0);
    }
    if (P.u[0].z != 0u) {
        let r = P.f[2];
        let fy = f32(p.y) + 0.5;
        let cy = clamp(fy - r.z + 0.5, 0.0, 1.0) * clamp(r.w - fy + 0.5, 0.0, 1.0);
        if (cy > 0.0) {
            let fx = f32(p.x) + 0.5;
            let cx = clamp(fx - r.x + 0.5, 0.0, 1.0) * clamp(r.y - fx + 0.5, 0.0, 1.0);
            let k = cx * cy * P.f[3].w;
            if (k > 0.0) {
                o = ftx_over(o, vec4<f32>(P.f[3].xyz * k, k));
            }
        }
    }
    let cov = textureLoad(aux, p, 0);
    let fill = ftx_tint(P.f[0], cov.x);
    let stroke = ftx_tint(P.f[1], cov.y);
    switch P.u[0].x {
        case 1u: {
            o = ftx_over(o, stroke);
        }
        case 2u: {
            o = ftx_over(ftx_over(o, stroke), fill);
        }
        case 3u: {
            o = ftx_over(ftx_over(o, fill), stroke);
        }
        default: {
            o = ftx_over(o, fill);
        }
    }
    textureStore(out, p, o);
}

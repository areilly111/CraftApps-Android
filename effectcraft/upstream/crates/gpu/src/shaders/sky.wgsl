// Environment Light Background layers (`three_d::compose::draw_sky`): the layer's
// equirectangular image looked up by each pixel's view ray through the camera, composited over
// the canvas. src = the canvas, aux = the layer's buffer. f[0] = (forward, zoom); f[1] =
// (right, orthographic); f[2] = (down, environment rotation); f[3] = (x offset, y offset,
// 1 / scale, opacity) with the offsets ROI / scale − view size / 2; f[4] = (image width, height).

@compute @workgroup_size(16, 16)
fn adv_sky(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    let dims = out_dims();
    if (p.x >= dims.x || p.y >= dims.y) {
        return;
    }
    let fwd = P.f[0].xyz;
    var dir = fwd;
    if (P.f[1].w == 0.0) {
        let cx = (f32(p.x) + 0.5) * P.f[3].z + P.f[3].x;
        let cy = (f32(p.y) + 0.5) * P.f[3].z + P.f[3].y;
        dir = normalize(fwd * P.f[0].w + P.f[1].xyz * cx + P.f[2].xyz * cy);
    }
    // shade::equirect_uv.
    let pi = 3.141592653589793;
    var u = 0.5 + (atan2(dir.x, dir.z) + P.f[2].w) / (2.0 * pi);
    let v = acos_p(clamp(-dir.y, -1.0, 1.0)) / pi;
    u = u - floor(u);
    let iw = P.f[4].x;
    let ih = P.f[4].y;
    let s = sample_bilinear(aux, clamp(u * iw, 0.5, iw - 0.5), clamp(v * ih, 0.5, ih - 0.5));
    let op = P.f[3].w;
    let d = textureLoad(src, p, 0);
    textureStore(out, p, s * op + d * (1.0 - s.w * op));
}

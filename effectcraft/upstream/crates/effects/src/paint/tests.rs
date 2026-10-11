use super::*;
use crate::{EffectEnv, flatten_params, instantiate};

fn fx_with(strokes: &[StrokeSpec], tweak: impl Fn(&mut PropGroup)) -> PropGroup {
    let mut next = 1000;
    let mut ids = Ids(&mut next);
    let mut g = instantiate(crate::find(ID).unwrap(), &mut ids, "Paint", [64.0, 64.0]);
    for (i, s) in strokes.iter().enumerate() {
        let name = format!("{} {}", s.kind.label(), i + 1);
        g.children.push(stroke_group(&mut ids, &name, s).into());
    }
    tweak(&mut g);
    g
}

fn run(g: &PropGroup, img: Image, time: f64) -> Buf {
    let params = flatten_params(g, &mut |p| p.value.clone());
    let ctx = EffectCtx { params: &params, time, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: EffectEnv::default() };
    crate::apply(crate::find(ID).unwrap(), &ctx, Buf { img, offset: [0.0; 2], scale: 1.0 })
}

fn brush(points: &[[f64; 2]]) -> StrokeSpec {
    StrokeSpec { points: points.to_vec(), size_pressure: false, ..Default::default() }
}

fn stroke_of(spec: &StrokeSpec) -> Stroke {
    let g = fx_with(std::slice::from_ref(spec), |_| {});
    let params = flatten_params(&g, &mut |p| p.value.clone());
    parse(&params).remove(0)
}

#[test]
fn hard_dab_known_pixels() {
    let s = stroke_of(&StrokeSpec { diameter: 10.0, ..brush(&[[20.5, 20.5]]) });
    let d = dabs(&s, 1.0, [0.0; 2]);
    assert_eq!(d.len(), 1);
    let c = coverage(&s, &d, 64, 64);
    assert_eq!(c.at(20, 20), 1.0);
    assert_eq!(c.at(24, 20), 1.0);
    assert!((c.at(25, 20) - 0.5).abs() < 1e-6, "{}", c.at(25, 20));
    assert_eq!(c.at(26, 20), 0.0);
    assert_eq!(c.at(20, 26), 0.0);
    assert_eq!(c.at(16, 20), 1.0);
}

#[test]
fn soft_dab_falls_off() {
    let s = stroke_of(&StrokeSpec { diameter: 20.0, hardness: 0.0, ..brush(&[[20.5, 20.5]]) });
    let c = coverage(&s, &dabs(&s, 1.0, [0.0; 2]), 64, 64);
    assert_eq!(c.at(20, 20), 1.0);
    // Half the radius out: smoothstep(0.5) = 0.5.
    assert!((c.at(25, 20) - 0.5).abs() < 1e-6, "{}", c.at(25, 20));
    assert!(c.at(28, 20) < 0.2 && c.at(28, 20) > 0.0);
    assert_eq!(c.at(31, 20), 0.0);
}

#[test]
fn roundness_and_angle_shape_the_tip() {
    // 20 px wide, 5 px tall ellipse; rotated 90° it is 5 wide, 20 tall.
    let flat = stroke_of(&StrokeSpec { diameter: 20.0, roundness: 25.0, ..brush(&[[30.5, 30.5]]) });
    let c = coverage(&flat, &dabs(&flat, 1.0, [0.0; 2]), 64, 64);
    assert_eq!(c.at(38, 30), 1.0);
    assert_eq!(c.at(30, 38), 0.0);
    let tall = stroke_of(&StrokeSpec { diameter: 20.0, roundness: 25.0, angle: 90.0, ..brush(&[[30.5, 30.5]]) });
    let c = coverage(&tall, &dabs(&tall, 1.0, [0.0; 2]), 64, 64);
    assert_eq!(c.at(30, 38), 1.0);
    assert_eq!(c.at(38, 30), 0.0);
}

#[test]
fn spacing_sets_dab_count() {
    let line = [[10.0, 10.0], [60.0, 10.0]];
    let s = stroke_of(&StrokeSpec { diameter: 10.0, spacing: 100.0, ..brush(&line) });
    let d = dabs(&s, 1.0, [0.0; 2]);
    assert_eq!(d.len(), 6);
    assert_eq!(d[1].x, 20.0);
    let s = stroke_of(&StrokeSpec { diameter: 10.0, spacing: 25.0, ..brush(&line) });
    assert_eq!(dabs(&s, 1.0, [0.0; 2]).len(), 21);
    // Half resolution: same dabs at half the pixel positions and radius.
    let h = dabs(&s, 0.5, [0.0; 2]);
    assert_eq!(h.len(), 21);
    assert_eq!(h[4].x, 10.0);
    assert_eq!(h[4].r, 2.5);
}

#[test]
fn pressure_scales_the_tip() {
    let s = stroke_of(&StrokeSpec {
        diameter: 20.0,
        size_pressure: true,
        min_size: 0.0,
        pressure: vec![0.0, 1.0],
        spacing: 100.0,
        ..brush(&[[0.0, 0.0], [40.0, 0.0]])
    });
    let d = dabs(&s, 1.0, [0.0; 2]);
    assert_eq!(d[0].r, 0.0);
    assert_eq!(d.last().unwrap().r, 10.0);
    assert!((d[1].r - 5.0).abs() < 1e-9);
}

#[test]
fn write_on_end_reveals_progressively() {
    let line = [[10.5, 30.5], [50.5, 30.5]];
    let src = Image::new(64, 64);
    let half = fx_with(&[brush(&line)], |g| {
        let st = g.groups().next().unwrap().uid;
        let o = g.find_group_mut(st).unwrap().sub_mut("stroke_options").unwrap();
        o.get_mut("end").unwrap().value = Value::Scalar(50.0);
    });
    let out = run(&half, src.clone(), 0.0);
    assert_eq!(out.img.get(20, 30)[3], 1.0);
    assert_eq!(out.img.get(30, 30)[3], 1.0);
    assert_eq!(out.img.get(40, 30)[3], 0.0);
    let zero = fx_with(&[brush(&line)], |g| {
        let st = g.groups().next().unwrap().uid;
        g.find_group_mut(st).unwrap().sub_mut("stroke_options").unwrap().get_mut("end").unwrap().value = Value::Scalar(0.0);
    });
    let out = run(&zero, src.clone(), 0.0);
    assert!(out.img.data.iter().all(|p| p[3] == 0.0));
    let full = run(&fx_with(&[brush(&line)], |_| {}), src, 0.0);
    assert_eq!(full.img.get(45, 30)[3], 1.0);
}

#[test]
fn brush_paints_colour_and_respects_span() {
    let mut s = brush(&[[20.5, 20.5]]);
    s.color = [1.0, 0.0, 0.0, 1.0];
    s.in_time = 1.0;
    s.out_time = 2.0;
    let g = fx_with(&[s], |_| {});
    let src = Image::filled(64, 64, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(run(&g, src.clone(), 0.5).img.get(20, 20), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(run(&g, src.clone(), 1.0).img.get(20, 20), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(run(&g, src.clone(), 2.0).img.get(20, 20), [0.0, 0.0, 1.0, 1.0]);
    let (vis, other) = cache_key(&g, 1.5);
    assert_eq!(vis, vec![true]);
    assert!(!other);
    assert_eq!(cache_key(&g, 0.0).0, vec![false]);
}

#[test]
fn opacity_and_paint_on_transparent() {
    let mut s = brush(&[[20.5, 20.5]]);
    s.opacity = 50.0;
    s.color = [1.0, 1.0, 1.0, 1.0];
    let g = fx_with(&[s], |g| g.get_mut("on_transparent").unwrap().value = Value::Bool(true));
    let out = run(&g, Image::filled(64, 64, [0.0, 0.0, 1.0, 1.0]), 0.0);
    assert_eq!(out.img.get(20, 20), [0.5, 0.5, 0.5, 0.5]);
    assert_eq!(out.img.get(50, 50), [0.0; 4]);
}

#[test]
fn channels_rgb_and_alpha() {
    let mut rgb = brush(&[[20.5, 20.5]]);
    rgb.channels = 1;
    rgb.color = [1.0, 0.0, 0.0, 1.0];
    let half = Image::filled(64, 64, [0.0, 0.0, 0.5, 0.5]);
    let out = run(&fx_with(&[rgb], |_| {}), half.clone(), 0.0);
    assert_eq!(out.img.get(20, 20), [0.5, 0.0, 0.0, 0.5]);
    let mut alpha = brush(&[[20.5, 20.5]]);
    alpha.channels = 2;
    alpha.color = [0.0, 0.0, 0.0, 1.0];
    let out = run(&fx_with(&[alpha], |_| {}), half, 0.0);
    assert_eq!(out.img.get(20, 20)[3], 0.0);
    assert_eq!(out.img.get(40, 40), [0.0, 0.0, 0.5, 0.5]);
}

fn eraser(points: &[[f64; 2]], mode: u32) -> StrokeSpec {
    StrokeSpec { kind: StrokeKind::Eraser, erase_mode: mode, diameter: 6.0, ..brush(points) }
}

#[test]
fn eraser_modes() {
    let red = Image::filled(64, 64, [1.0, 0.0, 0.0, 1.0]);
    let mut green = brush(&[[20.5, 20.5]]);
    green.diameter = 20.0;
    green.color = [0.0, 1.0, 0.0, 1.0];
    let mut blue = brush(&[[24.5, 20.5]]);
    blue.diameter = 20.0;
    blue.color = [0.0, 0.0, 1.0, 1.0];
    let at = [24.5, 20.5];
    // Layer Source & Paint: a hole down to transparency.
    let out = run(&fx_with(&[green.clone(), blue.clone(), eraser(&[at], 0)], |_| {}), red.clone(), 0.0);
    assert_eq!(out.img.get(24, 20), [0.0; 4]);
    assert_eq!(out.img.get(30, 20), [0.0, 0.0, 1.0, 1.0]);
    // Paint Only: all paint goes, the source shows.
    let out = run(&fx_with(&[green.clone(), blue.clone(), eraser(&[at], 1)], |_| {}), red.clone(), 0.0);
    assert_eq!(out.img.get(24, 20), [1.0, 0.0, 0.0, 1.0]);
    // Last Stroke Only: only the blue stroke goes, green shows through.
    let out = run(&fx_with(&[green, blue, eraser(&[at], 2)], |_| {}), red.clone(), 0.0);
    assert_eq!(out.img.get(24, 20), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(out.img.get(32, 20), [0.0, 0.0, 1.0, 1.0]);
    // Paint Only never touches the source.
    let out = run(&fx_with(&[eraser(&[at], 1)], |_| {}), red, 0.0);
    assert_eq!(out.img.get(24, 20), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn clone_copies_offset_source_pixels() {
    let mut src = Image::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            src.set(x, y, [x as f32 / 64.0, y as f32 / 64.0, ((x * 7 + y * 3) % 5) as f32 / 5.0, 1.0]);
        }
    }
    let s = StrokeSpec { kind: StrokeKind::Clone, diameter: 12.0, clone_position: [10.5, 12.5], ..brush(&[[40.5, 30.5], [48.5, 30.5]]) };
    let out = run(&fx_with(&[s], |_| {}), src.clone(), 0.0);
    for (x, y) in [(40, 30), (42, 31), (46, 29), (48, 30)] {
        assert_eq!(out.img.get(x, y), src.get(x - 30, y - 18), "({x},{y})");
    }
    // Untouched elsewhere.
    assert_eq!(out.img.get(5, 5), src.get(5, 5));
}

#[test]
fn stroke_transform_moves_the_stroke() {
    let g = fx_with(&[brush(&[[20.5, 20.5]])], |g| {
        let st = g.groups().next().unwrap().uid;
        let t = g.find_group_mut(st).unwrap().sub_mut("transform").unwrap();
        t.get_mut("position").unwrap().value = Value::Vec2([40.5, 20.5]);
    });
    let out = run(&g, Image::new(64, 64), 0.0);
    assert_eq!(out.img.get(20, 20)[3], 0.0);
    assert_eq!(out.img.get(40, 20)[3], 1.0);
}

#[test]
fn render_is_deterministic() {
    let mut a = brush(&[[5.0, 5.0], [30.0, 50.0], [60.0, 10.0]]);
    a.hardness = 30.0;
    a.flow = 40.0;
    a.diameter = 14.0;
    let g = fx_with(&[a, eraser(&[[30.0, 30.0], [40.0, 30.0]], 0)], |_| {});
    let src = Image::filled(64, 64, [0.2, 0.3, 0.4, 1.0]);
    let x = run(&g, src.clone(), 0.0);
    let y = run(&g, src, 0.0);
    assert_eq!(x.img, y.img);
}

#[test]
fn stroke_groups_round_trip_serde() {
    let g = fx_with(&[brush(&[[1.0, 2.0], [3.0, 4.0]]), eraser(&[[1.0, 1.0]], 2)], |_| {});
    let j = serde_json::to_string(&g).unwrap();
    let back: PropGroup = serde_json::from_str(&j).unwrap();
    assert_eq!(back, g);
    assert_eq!(strokes(&back).count(), 2);
}

//! `vectorcraft-cli perf`: the performance budgets (plan §4) measured on a synthetic document.
//!
//! Each row is the median of several runs. Timings are wall-clock: on a busy machine (load
//! average above the core count) they are noise, and the report says so.

use std::time::Instant;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, hit};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_render::{RenderOptions, Renderer};

/// Deterministic xorshift in 0..1.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 1_000_000) as f64 / 1_000_000.0
    }
}

/// `n` mixed paths (ellipses with strokes, translucent rectangles, stroked stars) on a 1600×1200 page.
pub fn synthetic(n: usize) -> Result<Document, String> {
    let mut d = Document::new(1600.0, 1200.0);
    let l = d.layers.first().map(|l| l.id);
    let mut r = Rng(42);
    for i in 0..n {
        let (x, y, s) = (r.next() * 1600.0, r.next() * 1200.0, 3.0 + r.next() * 22.0);
        let c = Color::rgb(r.next() as f32, r.next() as f32, r.next() as f32);
        let id = d.alloc_id();
        let mut node = match i % 3 {
            0 => Node::path(
                id,
                shapes::ellipse(Rect::from_center_size(Point::new(x, y), (2.0 * s, 2.0 * s))),
                Appearance::basic(Paint::solid(c), Paint::solid(Color::BLACK), 0.5),
            ),
            1 => Node::path(id, shapes::rectangle(Rect::new(x, y, x + 2.0 * s, y + s)), Appearance::basic(Paint::solid(c), Paint::None, 0.0)),
            _ => Node::path(id, shapes::star(Point::new(x, y), s, s / 2.0, 5, 0.0), Appearance::basic(Paint::None, Paint::solid(c), 2.0)),
        };
        if i % 3 == 1 {
            node.opacity = 0.8;
        }
        d.insert(l, usize::MAX, node).map_err(|e| format!("building the synthetic document: {e}"))?;
    }
    Ok(d)
}

fn median_ms(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut v: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// 1-minute load average (macOS / Linux), if available.
fn load_average() -> Option<f64> {
    let out = std::process::Command::new("sysctl").args(["-n", "vm.loadavg"]).output().ok().filter(|o| o.status.success());
    let text = match out {
        Some(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        None => std::fs::read_to_string("/proc/loadavg").ok()?,
    };
    text.split_whitespace().map(|t| t.trim_matches(|c| c == '{' || c == '}')).find_map(|t| t.parse().ok())
}

/// Milliseconds per pointer move of a Direct Selection drag of a handle, then of a segment, of a
/// 5,000-anchor wave of smooth anchors added on top of the synthetic `n`-path document.
fn edit_drags(n: usize) -> Result<(f64, f64), String> {
    use vectorcraft_engine::tools::{Mods, PointerEvent, PointerKind};
    use vectorcraft_geom::{Anchor, PathData, SubPath, Vec2};
    let mut doc = synthetic(n)?;
    let l = doc.layers.first().map(|l| l.id);
    let id = doc.alloc_id();
    // Anchors 2 pt apart, up and down by 20 pt, their handles 0.6 pt long.
    let anchor = |i: usize| {
        let p = Point::new(i as f64 * 2.0, if i.is_multiple_of(2) { 500.0 } else { 520.0 });
        Anchor::smooth(p, p + Vec2::new(0.6, 0.0))
    };
    let path = PathData::single(SubPath::new((0..5000).map(anchor).collect(), false));
    doc.insert(l, usize::MAX, Node::path(id, path, Appearance::default_art())).map_err(|e| e.to_string())?;
    let mut s = vectorcraft_engine::Session::new();
    s.add_document(doc, None);
    let view = vectorcraft_engine::ViewInfo { zoom: 8.0, ..Default::default() };
    let err = |e: vectorcraft_engine::EngineError| e.to_string();
    let mut drag = |down: Point| -> Result<f64, String> {
        let ev = |kind, y: f64| PointerEvent { kind, pos: down + Vec2::new(0.0, y), mods: Mods::default(), pressure: 1.0 };
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 400]]})).map_err(err)?;
        s.select_tool("directSelection", view).map_err(err)?;
        let undo = s.doc().map_err(err)?.history.undo.len();
        s.pointer(&ev(PointerKind::Down, 0.0), view).map_err(err)?;
        let mut y = 0.0;
        let ms = median_ms(9, || {
            y += 3.0;
            // A failed move shows as the missing undo step below.
            let _ = s.pointer(&ev(PointerKind::Drag, y), view);
        });
        s.pointer(&ev(PointerKind::Up, y), view).map_err(err)?;
        if s.doc().map_err(err)?.history.undo.len() != undo + 1 {
            return Err(format!("the drag from {down:?} made no undo step"));
        }
        Ok(ms)
    };
    // Anchor 400 is at (800, 500): its outgoing handle's end, then its segment to anchor 401.
    let handle = drag(Point::new(800.6, 500.0))?;
    let segment = drag(Point::new(801.0, 510.0))?;
    Ok((handle, segment))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mut n = 50_000usize;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--paths" => n = it.next().and_then(|v| v.parse().ok()).ok_or("--paths needs a number")?,
            other => return Err(format!("unknown perf option `{other}`")),
        }
    }
    let cores = std::thread::available_parallelism().map_or(1, |c| c.get());
    let load = load_average();
    outln!("VectorCraft performance budgets — {n} paths, {cores} cores, load average {}", load.map_or("?".into(), |l| format!("{l:.1}")));
    let noisy = load.is_some_and(|l| l > cores as f64 * 0.75);
    if noisy {
        outln!("WARNING: the machine is busy; wall-clock timings below are not trustworthy.");
    }
    let t = Instant::now();
    let doc = synthetic(n)?;
    outln!("  (built in {:.0} ms)", t.elapsed().as_secs_f64() * 1000.0);

    let mut rows: Vec<(&str, f64, f64)> = vec![];
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let fit = Affine::scale(1.6);
    r.render(&doc, 2880, 1800, fit, &opts);
    // The canvas never waits for these: while panning/zooming it shows the last frame reprojected
    // and re-renders in the background, so the budget is for that refresh to land quickly.
    rows.push(("render: fit page, 2880×1800", median_ms(5, || drop(r.render(&doc, 2880, 1800, fit, &opts))), 100.0));
    let mut dx = 0.0;
    rows.push((
        "render: background refresh after a pan",
        median_ms(9, || {
            dx += 7.0;
            drop(r.render(&doc, 2880, 1800, Affine::translate((dx, 0.0)) * fit, &opts));
        }),
        100.0,
    ));
    let zoom = Affine::scale(6.4) * Affine::translate((-600.0, -400.0));
    r.render(&doc, 2880, 1800, zoom, &opts);
    // Zoomed in, culling leaves few objects: this one should fit a frame.
    rows.push(("render: 400% zoom, 2880×1800", median_ms(5, || drop(r.render(&doc, 2880, 1800, zoom, &opts))), 16.0));

    let mut rng = Rng(7);
    let points: Vec<Point> = (0..200).map(|_| Point::new(rng.next() * 1600.0, rng.next() * 1200.0)).collect();
    let hits = median_ms(3, || {
        for p in &points {
            drop(hit::hit_test(&doc, *p, hit::HitOptions::default()));
        }
    }) / points.len() as f64;
    rows.push(("hit test (per click)", hits, 2.0));

    let mut bytes = vec![];
    rows.push(("save .vectorcraft", median_ms(3, || bytes = vectorcraft_format::save_file(&doc)), 300.0));
    let mut failed = None;
    let open = median_ms(3, || {
        if let Err(e) = vectorcraft_format::load(&bytes) {
            failed = Some(format!("open .vectorcraft: {e}"));
        }
    });
    rows.push(("open .vectorcraft", open, 300.0));
    let svg_opts = vectorcraft_svg::ExportOptions::default();
    rows.push(("export SVG", median_ms(3, || drop(vectorcraft_svg::export(&doc, &svg_opts))), 500.0));

    // Pathfinder Unite on 1,000 overlapping paths.
    let unite = median_ms(3, || {
        let mut s = vectorcraft_engine::Session::new();
        let r = synthetic(1000).and_then(|d| {
            s.add_document(d, None);
            s.execute("select.all", &json!({})).and_then(|_| s.execute("object.pathfinder.unite", &json!({}))).map_err(|e| e.to_string())
        });
        if let Err(e) = r {
            failed = Some(format!("Pathfinder Unite: {e}"));
        }
    });
    rows.push(("Pathfinder Unite, 1,000 paths", unite, 150.0));

    // Path editing: one pointer move of a Direct Selection drag (the tool event and the preview it
    // applies), on a 5,000-anchor path among the synthetic paths.
    let (handle, segment) = edit_drags(n).unwrap_or_else(|e| {
        failed = Some(e);
        (0.0, 0.0)
    });
    rows.push(("handle drag, 5,000-anchor path (per move)", handle, 8.0));
    rows.push(("segment drag, 5,000-anchor path (per move)", segment, 8.0));
    if let Some(e) = failed {
        return Err(e);
    }

    let mut over = 0;
    outln!("  {:<40} {:>10} {:>10}", "", "measured", "budget");
    for (name, ms, budget) in &rows {
        let ok = ms <= budget;
        over += usize::from(!ok);
        outln!("  {name:<40} {ms:>8.2} ms {budget:>7.0} ms  {}", if ok { "ok" } else { "OVER" });
    }
    match (over, noisy) {
        (0, _) => Ok(()),
        (_, true) => {
            outln!("{over} over budget, but the machine is busy: re-run when idle.");
            Ok(())
        }
        _ => Err(format!("{over} budget(s) exceeded")),
    }
}

//! The CLI opens every readable format (PDF, .ai, images) through the engine's `document.open`.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft-cli");

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("vectorcraft-cli-fileio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().to_string()
}

fn ok(args: &[&str]) -> String {
    let out = Command::new(BIN).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// `run` a new `width`×`height` document with a rectangle and export it to `out`.
fn make(out: &str, width: u32, height: u32) {
    ok(&[
        "run",
        "--cmd",
        "file.new",
        "--params",
        &format!(r#"{{"width":{width},"height":{height},"artboards":2}}"#),
        "--cmd",
        "shape.rectangle",
        "--params",
        r#"{"x":1,"y":1,"width":2,"height":1}"#,
        "--export",
        out,
    ]);
}

fn info(file: &str) -> Value {
    serde_json::from_str(&ok(&["info", file])).unwrap()
}

#[test]
fn convert_pdf_to_svg_and_info_on_ai() {
    let pdf = tmp("in.pdf");
    make(&pdf, 120, 80);
    let svg = tmp("out.svg");
    ok(&["convert", &pdf, &svg]);
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));

    let ai = tmp("in.ai");
    std::fs::copy(&pdf, &ai).unwrap();
    let v = info(&ai);
    assert_eq!(v["artboards"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(v["artboards"][0]["rect"][2], 120.0);
    // `run --in` takes it too.
    ok(&["run", "--in", &ai, "--cmd", "document.inspect"]);
    // A PDF keeps every artboard unless --artboard / --range names some.
    let all = tmp("all.pdf");
    ok(&["convert", &ai, &all]);
    assert_eq!(info(&all)["artboards"].as_array().unwrap().len(), 2);
    let second = tmp("second.pdf");
    ok(&["convert", &ai, &second, "--range", "2"]);
    assert_eq!(info(&second)["artboards"].as_array().unwrap().len(), 1);
}

#[test]
fn images_open_at_their_pixel_size() {
    let webp = tmp("tiny.webp");
    make(&webp, 3, 2);
    let v = info(&webp);
    assert_eq!(v["artboards"][0]["rect"], serde_json::json!([0.0, 0.0, 3.0, 2.0]), "{v}");
    assert_eq!(v["kinds"]["Image"], 1, "{v}");
    let png = tmp("tiny.png");
    ok(&["convert", &webp, &png, "--scale", "2"]);
    assert_eq!(&std::fs::read(&png).unwrap()[16..24], [0, 0, 0, 6, 0, 0, 0, 4], "IHDR 6×4");
}

#[test]
fn help_lists_readable_formats() {
    let out = ok(&["--help"]);
    assert!(out.contains("Readable formats: .vectorcraft") && out.contains(".ait") && out.contains(".webp"), "{out}");
}

/// `info` reports the import warnings: an EPS read only up to an error names it.
#[test]
fn info_reports_why_an_eps_read_partly() {
    let eps = tmp("partial.eps");
    std::fs::write(&eps, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 10 10 rectfill frobnicate\n%%EOF\n").unwrap();
    let v = info(&eps);
    let w = v["warnings"][0].as_str().unwrap_or_default();
    assert!(w.contains("`frobnicate`"), "{v}");
    assert_eq!(v["kinds"]["Path"], 1, "{v}");
}

//! Minimal plug-ins in WebAssembly text (WAT): the skeleton `docs/plugins.md` describes, which the
//! tests turn into modules (with the `wat` crate) to exercise the host.

/// Where [`returning`] keeps its output in memory (the manifest sits at 16).
const OUTPUT_AT: usize = 0x1_0000;

/// `s` as the body of a WAT string literal (every byte that isn't a letter, digit or space
/// escaped).
pub fn string(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b' ' { char::from(b).to_string() } else { format!("\\{b:02x}") }).collect()
}

/// A plug-in module: `manifest` (JSON), a `vc_run` whose body is `run` (parameters `$in $il $pp
/// $pl`, result i64), extra module fields `extra` and `pages` of initial memory. `vc_alloc` grows
/// memory by whole pages, so its blocks never overlap data segments.
pub fn module(manifest: &str, run: &str, extra: &str, pages: u32) -> String {
    format!(
        r#"(module
  {extra}
  (memory (export "memory") {pages})
  (data (i32.const 16) "{data}")
  (func (export "vc_abi_version") (result i32) (i32.const 1))
  (func (export "vc_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "vc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1)) (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "vc_run") (param $in i32) (param $il i32) (param $pp i32) (param $pl i32) (result i64)
    {run}))"#,
        data = string(manifest),
        len = manifest.len(),
    )
}

/// A `vc_run` body returning the input unchanged.
pub const ECHO: &str = "(i64.or (i64.shl (i64.extend_i32_u (local.get $il)) (i64.const 32)) (i64.extend_i32_u (local.get $in)))";

/// A plug-in that always returns `output` (whatever it is given).
pub fn returning(manifest: &str, output: &str) -> String {
    let run = format!("(i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const {OUTPUT_AT}))", output.len());
    let extra = format!(r#"(data (i32.const {OUTPUT_AT}) "{}")"#, string(output));
    module(manifest, &run, &extra, (OUTPUT_AT + output.len()).div_ceil(0x1_0000) as u32 + 1)
}

/// The manifest of a test plug-in `id` of `kind` with `params` (a JSON object of specs).
pub fn manifest(id: &str, kind: &str, params: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"{kind}","params":{params}}}"#)
}

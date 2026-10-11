//! A minimal, valid ONNX model made in memory: for tests, and later for the install self-check. It is not a
//! working network (it has no layers), only a graph with the right inputs, outputs and a weight blob.

pub(crate) fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

pub fn len_field(n: u64, body: &[u8], out: &mut Vec<u8>) {
    varint(n << 3 | 2, out);
    varint(body.len() as u64, out);
    out.extend_from_slice(body);
}

pub(crate) fn varint_field(n: u64, v: u64, out: &mut Vec<u8>) {
    varint(n << 3, out);
    varint(v, out);
}

/// A graph input or output: `Ok(n)` is a fixed dimension, `Err(name)` a named (dynamic) one.
pub fn value_info_bytes(name: &str, elem: u64, dims: &[std::result::Result<u64, &str>]) -> Vec<u8> {
    let mut shape = Vec::new();
    for d in dims {
        let mut dim = Vec::new();
        match d {
            Ok(v) => varint_field(1, *v, &mut dim),
            Err(p) => len_field(2, p.as_bytes(), &mut dim),
        }
        len_field(1, &dim, &mut shape);
    }
    let mut tensor = Vec::new();
    varint_field(1, elem, &mut tensor);
    len_field(2, &shape, &mut tensor);
    let mut ty = Vec::new();
    len_field(1, &tensor, &mut ty);
    let mut vi = Vec::new();
    len_field(1, name.as_bytes(), &mut vi);
    len_field(2, &ty, &mut vi);
    vi
}

/// A small valid model shaped like a face embedder: float input `data` `[N, 3, 112, 112]`, an old-style
/// weight also listed as an input, a weight with bulk data, float output `emb` `[N, output_dim]`, opset 17.
pub fn embedder_model(output_dim: u64) -> Vec<u8> {
    let mut graph = Vec::new();
    len_field(11, &value_info_bytes("data", 1, &[Err("N"), Ok(3), Ok(112), Ok(112)]), &mut graph);
    len_field(11, &value_info_bytes("w", 1, &[Ok(4)]), &mut graph);
    let mut tensor = Vec::new();
    varint_field(1, 4, &mut tensor);
    len_field(8, b"w", &mut tensor);
    len_field(9, &[7u8; 5000], &mut tensor);
    len_field(5, &tensor, &mut graph);
    len_field(12, &value_info_bytes("emb", 1, &[Err("N"), Ok(output_dim)]), &mut graph);
    let mut ops = Vec::new();
    len_field(1, b"", &mut ops);
    varint_field(2, 17, &mut ops);
    let mut model = Vec::new();
    varint_field(1, 8, &mut model);
    len_field(2, b"pytorch", &mut model);
    len_field(7, &graph, &mut model);
    len_field(8, &ops, &mut model);
    model
}

/// A node attribute for [`node_bytes`].
enum Attribute<'a> {
    Int(i64),
    Ints(&'a [i64]),
    Float(f32),
}

fn node_bytes(op: &str, inputs: &[&str], output: &str, attrs: &[(&str, Attribute)]) -> Vec<u8> {
    let mut n = Vec::new();
    for i in inputs {
        len_field(1, i.as_bytes(), &mut n);
    }
    len_field(2, output.as_bytes(), &mut n);
    len_field(4, op.as_bytes(), &mut n);
    for (name, value) in attrs {
        let mut a = Vec::new();
        len_field(1, name.as_bytes(), &mut a);
        match value {
            Attribute::Float(v) => {
                varint(2 << 3 | 5, &mut a);
                a.extend_from_slice(&v.to_le_bytes());
                varint_field(20, 1, &mut a);
            }
            Attribute::Int(v) => {
                varint_field(3, *v as u64, &mut a);
                varint_field(20, 2, &mut a); // INT
            }
            Attribute::Ints(values) => {
                for v in *values {
                    varint_field(8, *v as u64, &mut a);
                }
                varint_field(20, 7, &mut a); // INTS
            }
        }
        len_field(5, &a, &mut n);
    }
    n
}

fn float_tensor_bytes(name: &str, dims: &[u64], values: &[f32]) -> Vec<u8> {
    let mut t = Vec::new();
    for d in dims {
        varint_field(1, *d, &mut t);
    }
    varint_field(2, 1, &mut t); // FLOAT
    len_field(8, name.as_bytes(), &mut t);
    let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    len_field(9, &raw, &mut t);
    t
}

/// A tiny working network shaped like a face embedder, for tests of the runtime: input `data`
/// `[1, 3, 112, 112]`, a 1 × 1 convolution to `dim` channels, global average pooling and flattening to
/// `[1, dim]`. Its output depends on the picture's per-channel brightness, which is all the tests need.
pub fn tiny_embedder_model(dim: u64) -> Vec<u8> {
    let mut graph = Vec::new();
    len_field(1, &node_bytes("Conv", &["data", "w", "b"], "c", &[("kernel_shape", Attribute::Ints(&[1, 1]))]), &mut graph);
    len_field(1, &node_bytes("GlobalAveragePool", &["c"], "p", &[]), &mut graph);
    len_field(1, &node_bytes("Flatten", &["p"], "emb", &[("axis", Attribute::Int(1))]), &mut graph);
    let w: Vec<f32> = (0..dim * 3).map(|i| ((i * 37 % 19) as f32 - 9.0) / 20.0).collect();
    let b: Vec<f32> = (0..dim).map(|i| (i % 5) as f32 / 10.0 - 0.2).collect();
    len_field(5, &float_tensor_bytes("w", &[dim, 3, 1, 1], &w), &mut graph);
    len_field(5, &float_tensor_bytes("b", &[dim], &b), &mut graph);
    len_field(11, &value_info_bytes("data", 1, &[Ok(1), Ok(3), Ok(112), Ok(112)]), &mut graph);
    len_field(12, &value_info_bytes("emb", 1, &[Ok(1), Ok(dim)]), &mut graph);
    let mut ops = Vec::new();
    len_field(1, b"", &mut ops);
    varint_field(2, 13, &mut ops);
    let mut model = Vec::new();
    varint_field(1, 8, &mut model);
    len_field(7, &graph, &mut model);
    len_field(8, &ops, &mut model);
    model
}

/// A deterministic small recognition graph using all SFace/AuraFace operators (no downloaded weights).
/// Intended for runtime regression and independent reference comparisons.
pub fn recognition_model(mut seed: u32) -> Vec<u8> {
    let mut random = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((seed >> 16) as f32 / 65535.0 - 0.5) * 0.2
    };
    let mut g = Vec::new();
    for (op, inputs, out, attrs) in [
        ("Sub", vec!["data", "center"], "centered", vec![]),
        ("Mul", vec!["centered", "scale"], "scaled", vec![]),
        ("Conv", vec!["scaled", "conv.w", "conv.b"], "conv", vec![("pads", Attribute::Ints(&[1, 1, 1, 1])), ("strides", Attribute::Ints(&[2, 2]))]),
        ("BatchNormalization", vec!["conv", "bn.s", "bn.b", "bn.m", "bn.v"], "bn", vec![("epsilon", Attribute::Float(1e-5))]),
        ("PRelu", vec!["bn", "slope"], "act", vec![]),
        ("Add", vec!["act", "bn"], "skip", vec![]),
        ("Dropout", vec!["skip"], "drop", vec![("ratio", Attribute::Float(0.4))]),
        ("GlobalAveragePool", vec!["drop"], "pool", vec![]),
        ("Flatten", vec!["pool"], "flat", vec![]),
        ("Gemm", vec!["flat", "fc.w", "fc.b"], "emb", vec![("transB", Attribute::Int(1))]),
    ] {
        len_field(1, &node_bytes(op, &inputs, out, &attrs), &mut g);
    }
    for (name, dims, data) in [
        ("center", vec![1], vec![127.5]),
        ("scale", vec![1], vec![1.0 / 127.5]),
        ("conv.w", vec![5, 3, 3, 3], (0..135).map(|_| random()).collect()),
        ("conv.b", vec![5], (0..5).map(|_| random()).collect()),
        ("bn.s", vec![5], (0..5).map(|_| 1.0 + random()).collect()),
        ("bn.b", vec![5], (0..5).map(|_| random()).collect()),
        ("bn.m", vec![5], (0..5).map(|_| random()).collect()),
        ("bn.v", vec![5], (0..5).map(|_| 1.0 + random()).collect()),
        ("slope", vec![5, 1, 1], (0..5).map(|_| 0.2 + random()).collect()),
        ("fc.w", vec![8, 5], (0..40).map(|_| random()).collect()),
        ("fc.b", vec![8], (0..8).map(|_| random()).collect()),
    ] {
        len_field(5, &float_tensor_bytes(name, &dims, &data), &mut g);
    }
    len_field(11, &value_info_bytes("data", 1, &[Ok(1), Ok(3), Ok(112), Ok(112)]), &mut g);
    len_field(12, &value_info_bytes("emb", 1, &[Ok(1), Ok(8)]), &mut g);
    let mut ops = Vec::new();
    varint_field(2, 11, &mut ops);
    let mut model = Vec::new();
    varint_field(1, 8, &mut model);
    len_field(7, &g, &mut model);
    len_field(8, &ops, &mut model);
    model
}

#[cfg(test)]
pub(crate) fn norm_overflow_model() -> Vec<u8> {
    let mut graph = Vec::new();
    len_field(1, &node_bytes("Flatten", &["data"], "flat", &[]), &mut graph);
    len_field(1, &node_bytes("Gemm", &["flat", "w"], "emb", &[("transB", Attribute::Int(1))]), &mut graph);
    let plane = 112 * 112;
    let mut weights = vec![0.0; 3 * 3 * plane];
    for c in 0..3 {
        weights[c * 3 * plane + c * plane] = 5e37;
    }
    len_field(5, &float_tensor_bytes("w", &[3, (3 * plane) as u64], &weights), &mut graph);
    len_field(11, &value_info_bytes("data", 1, &[Ok(1), Ok(3), Ok(112), Ok(112)]), &mut graph);
    len_field(12, &value_info_bytes("emb", 1, &[Ok(1), Ok(3)]), &mut graph);
    let mut ops = Vec::new();
    varint_field(2, 11, &mut ops);
    let mut model = Vec::new();
    len_field(7, &graph, &mut model);
    len_field(8, &ops, &mut model);
    model
}

/// A small YuNet-shaped stand-in that reports no faces, independent of the real downloaded model.
pub fn tiny_detector_model() -> Vec<u8> {
    let mut graph = Vec::new();
    len_field(11, &value_info_bytes("input", 1, &[Ok(1), Ok(3), Ok(64), Ok(64)]), &mut graph);
    for stride in [8u64, 16, 32] {
        let cells = (64 / stride) * (64 / stride);
        for (kind, components) in [("cls", 1u64), ("obj", 1), ("bbox", 4), ("kps", 10)] {
            let name = format!("{kind}_{stride}");
            let weight = format!("weight_{name}");
            let values = vec![0.0; (cells * components) as usize];
            len_field(5, &float_tensor_bytes(&weight, &[1, cells, components], &values), &mut graph);
            len_field(1, &node_bytes("Identity", &[&weight], &name, &[]), &mut graph);
            len_field(12, &value_info_bytes(&name, 1, &[Ok(1), Ok(cells), Ok(components)]), &mut graph);
        }
    }
    let mut model = Vec::new();
    varint_field(1, 8, &mut model);
    len_field(7, &graph, &mut model);
    let mut ops = Vec::new();
    varint_field(2, 13, &mut ops);
    len_field(8, &ops, &mut model);
    model
}

/// Manifest for the stand-in, for model-install and background-scan tests only.
pub fn tiny_detector_manifest() -> crate::manifest::ModelManifest {
    let bytes = tiny_detector_model();
    let mut m = crate::known::yunet();
    m.name = "Synthetic detector (tests only)".into();
    m.input.width = 64;
    m.input.height = 64;
    m.sha256 = Some(crate::hash::sha256_hex(&bytes));
    m.size_bytes = Some(bytes.len() as u64);
    m
}

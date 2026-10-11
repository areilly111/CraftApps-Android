//! Running TensorFlow Lite models without TensorFlow: a reader for the `.tflite` FlatBuffer
//! (fields as numbered in TensorFlow Lite's published `schema.fbs`, Apache-2.0) and an
//! interpreter for the float operators small vision models use: `CONV_2D`, `DEPTHWISE_CONV_2D`,
//! `MAX_POOL_2D`, `ADD`, `PAD`, `CONCATENATION`, `RESHAPE`, `RELU`, `RELU6`, `PRELU`, `LOGISTIC`
//! and `DEQUANTIZE` of float16 weights (folded at load). Weights are re-laid out once at load for
//! the [`nn`](crate::nn) kernels. Anything else (other operators, quantised tensors, dilation) is a
//! load error, never a panic: models are pinned by SHA-256, so what loads once always loads.
//!
//! A model is also refused at load when running it would take more than a fixed budget: memory
//! (each tensor, all tensors alive at once, one operator's working space, the weights), work
//! (multiply-adds, values written) or kernel size. Every one is worked out from the static
//! shapes with checked arithmetic, so a small hostile file can neither exhaust memory nor keep
//! [`Model::run`] busy for minutes. The budgets leave wide headroom over the official
//! MediaPipe face models.

use crate::nn::{Conv, Depthwise, Linear};
use crate::pt::half_to_f32;

type Result<T> = std::result::Result<T, String>;

// ---------------------------------------------------------------- FlatBuffers

/// A FlatBuffer: bounds-checked little-endian reads.
#[derive(Clone, Copy)]
struct Fb<'a>(&'a [u8]);

/// A table: its position in the buffer.
#[derive(Clone, Copy)]
struct Table(usize);

impl<'a> Fb<'a> {
    fn bytes<const N: usize>(&self, at: usize) -> Result<[u8; N]> {
        self.0.get(at..at.checked_add(N).ok_or("bad offset")?).and_then(|s| s.try_into().ok()).ok_or_else(|| "truncated model".to_string())
    }
    fn u8(&self, at: usize) -> Result<u8> {
        Ok(self.bytes::<1>(at)?[0])
    }
    fn u16(&self, at: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(at)?))
    }
    fn u32(&self, at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(at)?))
    }
    fn i32(&self, at: usize) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(at)?))
    }
    fn u64(&self, at: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(at)?))
    }
    /// Follow the offset stored at `at`.
    fn deref(&self, at: usize) -> Result<usize> {
        at.checked_add(self.u32(at)? as usize).ok_or_else(|| "bad offset".to_string())
    }
    fn root(&self) -> Result<Table> {
        Ok(Table(self.deref(0)?))
    }
    /// Where field `id` of `t` is stored (absent fields have their default).
    fn field(&self, t: Table, id: usize) -> Result<Option<usize>> {
        let vt = (t.0 as i64 - self.i32(t.0)? as i64).try_into().map_err(|_| "bad vtable")?;
        let vt_len = self.u16(vt)? as usize;
        let slot = 4 + 2 * id;
        if slot + 2 > vt_len {
            return Ok(None);
        }
        let off = self.u16(vt + slot)? as usize;
        Ok((off != 0).then_some(t.0 + off))
    }
    fn int(&self, t: Table, id: usize, default: i32) -> Result<i32> {
        self.field(t, id)?.map_or(Ok(default), |p| self.i32(p))
    }
    fn byte(&self, t: Table, id: usize, default: u8) -> Result<u8> {
        self.field(t, id)?.map_or(Ok(default), |p| self.u8(p))
    }
    fn table(&self, t: Table, id: usize) -> Result<Option<Table>> {
        self.field(t, id)?.map(|p| self.deref(p).map(Table)).transpose()
    }
    /// A vector field: (start of its elements, length).
    fn vector(&self, t: Table, id: usize) -> Result<(usize, usize)> {
        let Some(p) = self.field(t, id)? else { return Ok((0, 0)) };
        let v = self.deref(p)?;
        let n = self.u32(v)? as usize;
        if v.saturating_add(4).saturating_add(n) > self.0.len() {
            return Err("truncated model".into());
        }
        Ok((v + 4, n))
    }
    fn tables(&self, t: Table, id: usize) -> Result<Vec<Table>> {
        let (at, n) = self.vector(t, id)?;
        (0..n).map(|i| self.deref(at + 4 * i).map(Table)).collect()
    }
    fn ints(&self, t: Table, id: usize) -> Result<Vec<i32>> {
        let (at, n) = self.vector(t, id)?;
        (0..n).map(|i| self.i32(at + 4 * i)).collect()
    }
    fn string(&self, t: Table, id: usize) -> Result<String> {
        let (at, n) = self.vector(t, id)?;
        Ok(String::from_utf8_lossy(self.0.get(at..at + n).ok_or("truncated model")?).into_owned())
    }
}

// ---------------------------------------------------------------- the model

/// Operator codes (`BuiltinOperator`).
mod op {
    pub const ADD: i32 = 0;
    pub const CONCATENATION: i32 = 2;
    pub const CONV_2D: i32 = 3;
    pub const DEPTHWISE_CONV_2D: i32 = 4;
    pub const DEQUANTIZE: i32 = 6;
    pub const LOGISTIC: i32 = 14;
    pub const MAX_POOL_2D: i32 = 17;
    pub const RELU: i32 = 19;
    pub const RELU6: i32 = 21;
    pub const RESHAPE: i32 = 22;
    pub const PAD: i32 = 34;
    pub const PRELU: i32 = 54;
}

/// The most values a tensor may hold.
const MAX_TENSOR: usize = 1 << 28;
/// The widest convolution or pooling window side.
const MAX_KERNEL: usize = 32;
/// The most floats one operator may use as working space (a convolution's patch matrix for one
/// output row: `ow·kh·kw·cin`).
const MAX_SCRATCH: u64 = 1 << 22;
/// The most multiply-adds (convolutions) and window comparisons (pooling) in one run.
const MAX_MACS: u64 = 1 << 33;
/// The most values all operators together may write in one run.
const MAX_VALUES: u64 = 1 << 30;
/// The most floats alive at once while the model runs.
const MAX_LIVE: u64 = 1 << 29;
/// The most constant values decoded and laid out at load (a weight shared by several tensors or
/// operators counts once per use).
const MAX_CONSTS: usize = 1 << 27;

/// Tensor types (`TensorType`).
const FLOAT32: u8 = 0;
const FLOAT16: u8 = 1;
const INT32: u8 = 2;

/// A fused activation (`ActivationFunctionType`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    None,
    Relu,
    ReluN1To1,
    Relu6,
    Tanh,
}

impl Act {
    fn from(code: u8) -> Result<Act> {
        Ok(match code {
            0 => Act::None,
            1 => Act::Relu,
            2 => Act::ReluN1To1,
            3 => Act::Relu6,
            4 => Act::Tanh,
            c => return Err(format!("unsupported fused activation {c}")),
        })
    }
    fn apply(self, x: &mut [f32]) {
        let f: fn(f32) -> f32 = match self {
            Act::None => return,
            Act::Relu => |v| v.max(0.0),
            Act::ReluN1To1 => |v| v.clamp(-1.0, 1.0),
            Act::Relu6 => |v| v.clamp(0.0, 6.0),
            Act::Tanh => f32::tanh,
        };
        for v in x {
            *v = f(*v);
        }
    }
}

/// One operator, its weights laid out for the kernels.
#[derive(Clone, Debug)]
enum Op {
    /// A 1×1, stride-1 convolution: a dense layer over the pixels.
    Pointwise(Linear, Act),
    Conv(Conv, [usize; 2], Act),
    Depthwise(Depthwise, [usize; 2], Act),
    /// Kernel, stride, padding before (top, left).
    MaxPool {
        k: [usize; 2],
        stride: [usize; 2],
        pad: [usize; 2],
        act: Act,
    },
    Add(Act),
    /// Constant padding (zeros), per dimension (before, after).
    Pad(Vec<[usize; 2]>),
    Concat {
        axis: usize,
        act: Act,
    },
    Reshape,
    Act(Act),
    Prelu(Vec<f32>),
    Logistic,
}

#[derive(Clone, Debug)]
struct Step {
    op: Op,
    /// Runtime inputs (tensor ids).
    inputs: Vec<usize>,
    output: usize,
}

/// A loaded model, ready to run (batch of one, static shapes).
#[derive(Clone, Debug)]
pub struct Model {
    shapes: Vec<Vec<usize>>,
    names: Vec<String>,
    /// Constant float tensors (weights folded into the steps are not kept).
    consts: Vec<Option<Vec<f32>>>,
    steps: Vec<Step>,
    inputs: Vec<usize>,
    outputs: Vec<usize>,
    /// After which step each tensor is last needed (to free memory as the model runs).
    last_use: Vec<usize>,
}

fn index(i: i32, n: usize) -> Result<usize> {
    usize::try_from(i).ok().filter(|&i| i < n).ok_or_else(|| format!("bad tensor index {i}"))
}

/// "Same" padding: output size and padding before.
fn same(input: usize, k: usize, stride: usize) -> (usize, usize) {
    let out = input.div_ceil(stride);
    let total = ((out.saturating_sub(1)) * stride + k).saturating_sub(input);
    (out, total / 2)
}

/// Output size and padding before, for `padding` (0 same, 1 valid).
fn window(padding: u8, input: usize, k: usize, stride: usize) -> Result<(usize, usize)> {
    match padding {
        0 => Ok(same(input, k, stride)),
        1 if input >= k => Ok(((input - k) / stride + 1, 0)),
        _ => Err("bad window".into()),
    }
}

impl Model {
    /// Read a `.tflite` file.
    pub fn read(bytes: &[u8]) -> Result<Model> {
        let fb = Fb(bytes);
        let root = fb.root()?;
        let codes: Vec<i32> = fb.tables(root, 1)?.into_iter().map(|c| Ok(fb.int(c, 3, 0)?.max(fb.byte(c, 0, 0)? as i8 as i32))).collect::<Result<_>>()?;
        let buffers = fb.tables(root, 4)?;
        let graphs = fb.tables(root, 2)?;
        let g = *graphs.first().ok_or("the model has no graph")?;
        let tensors = fb.tables(g, 0)?;
        let n = tensors.len();
        let mut shapes = Vec::with_capacity(n);
        let mut names = Vec::with_capacity(n);
        // Constant data: float tensors as f32, int tensors (pads, shapes) as i32, decoded once
        // their total is known to fit the budget.
        let mut floats: Vec<Option<Vec<f32>>> = vec![None; n];
        let mut ints: Vec<Option<Vec<i32>>> = vec![None; n];
        let mut data: Vec<(usize, u8, usize, &[u8])> = vec![];
        let mut consts = 0usize;
        for (i, t) in tensors.iter().enumerate() {
            let shape: Vec<usize> = fb
                .ints(*t, 0)?
                .into_iter()
                .map(|d| usize::try_from(d).map_err(|_| "dynamic shapes are not supported"))
                .collect::<std::result::Result<_, _>>()?;
            let ty = fb.byte(*t, 1, FLOAT32)?;
            let buf = fb.int(*t, 2, 0)? as u32 as usize;
            names.push(fb.string(*t, 3)?);
            // Every size computed from shapes later stays in range, even a product of some of the
            // dimensions (a zero dimension does not let the others grow without bound).
            let count = shape.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).ok_or("a tensor is too large")?;
            if shape.iter().try_fold(1usize, |a, &d| a.checked_mul(d.max(1))).is_none_or(|c| c > MAX_TENSOR) {
                return Err("a tensor is too large".into());
            }
            if fb.field(*t, 4)?.is_some() && fb.table(*t, 4)?.is_some_and(|q| fb.vector(q, 2).is_ok_and(|v| v.1 > 0)) {
                return Err(format!("{}: quantised tensors are not supported", names[i]));
            }
            if buf > 0 {
                let b = *buffers.get(buf).ok_or("bad buffer index")?;
                let (mut at, mut len) = fb.vector(b, 0)?;
                if len == 0 {
                    // Large models keep data after the FlatBuffer: offset and size from its start.
                    let (o, s) = (fb.field(b, 1)?.map(|p| fb.u64(p)).transpose()?.unwrap_or(0), fb.field(b, 2)?.map(|p| fb.u64(p)).transpose()?.unwrap_or(0));
                    (at, len) = (o as usize, s as usize);
                }
                if len > 0 {
                    data.push((i, ty, count, bytes.get(at..at.checked_add(len).ok_or("bad buffer")?).ok_or("truncated model")?));
                    // Several tensors may share one buffer: each is decoded on its own.
                    consts = consts.checked_add(count).filter(|&c| c <= MAX_CONSTS).ok_or("the model's weights are too large")?;
                }
            }
            shapes.push(shape);
        }
        for (i, ty, count, data) in data {
            match ty {
                FLOAT32 if data.len() == 4 * count => {
                    floats[i] = Some(data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
                }
                FLOAT16 if data.len() == 2 * count => {
                    floats[i] = Some(data.as_chunks::<2>().0.iter().map(|c| half_to_f32(u16::from_le_bytes([c[0], c[1]]))).collect())
                }
                INT32 if data.len() == 4 * count => {
                    ints[i] = Some(data.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
                }
                _ => return Err(format!("{}: unsupported tensor data", names[i])),
            }
        }
        // Copies of constants made below (folded DEQUANTIZEs, weights laid out for the kernels).
        let consts = std::cell::Cell::new(consts);
        let spend = |values: usize| -> Result<()> {
            consts.set(consts.get().checked_add(values).filter(|&c| c <= MAX_CONSTS).ok_or("the model's weights are too large")?);
            Ok(())
        };
        let inputs = fb.ints(g, 1)?.into_iter().map(|i| index(i, n)).collect::<Result<Vec<_>>>()?;
        let outputs = fb.ints(g, 2)?.into_iter().map(|i| index(i, n)).collect::<Result<Vec<_>>>()?;
        let mut steps = vec![];
        for o in fb.tables(g, 3)? {
            let code = *codes.get(fb.int(o, 0, 0)? as u32 as usize).ok_or("bad operator code")?;
            // Optional inputs are -1.
            let ins: Vec<Option<usize>> = fb.ints(o, 1)?.into_iter().map(|i| if i < 0 { Ok(None) } else { index(i, n).map(Some) }).collect::<Result<_>>()?;
            let outs = fb.ints(o, 2)?;
            let output = index(*outs.first().ok_or("an operator has no output")?, n)?;
            // Float16 weights: fold their DEQUANTIZE.
            if code == op::DEQUANTIZE
                && let Some(i) = ins.first().copied().flatten()
                && let Some(len) = floats[i].as_ref().map(Vec::len)
            {
                spend(len)?;
                floats[output] = floats[i].clone();
                continue;
            }
            let opts = fb.table(o, 4)?;
            let opt_int = |id: usize, default: i32| opts.map_or(Ok(default), |t| fb.int(t, id, default));
            let opt_byte = |id: usize, default: u8| opts.map_or(Ok(default), |t| fb.byte(t, id, default));
            let input = |k: usize| ins.get(k).copied().flatten().ok_or_else(|| format!("{}: missing input {k}", names[output]));
            let weights = |k: usize| -> Result<Vec<f32>> {
                let id = input(k)?;
                let w = floats[id].as_ref().ok_or_else(|| format!("{}: input {k} is not constant", names[output]))?;
                spend(w.len())?;
                Ok(w.clone())
            };
            let bias = |count: usize| -> Result<Vec<f32>> {
                spend(count)?;
                match ins.get(2).copied().flatten() {
                    Some(id) => floats[id].clone().filter(|b| b.len() == count).ok_or_else(|| format!("{}: bad bias", names[output])),
                    None => Ok(vec![0.0; count]),
                }
            };
            let in_shape = |k: usize| -> Result<Vec<usize>> { Ok(shapes[input(k)?].clone()) };
            let out_shape = shapes[output].clone();
            let stride = |w: usize, h: usize| -> Result<[usize; 2]> {
                let s = [opt_int(h, 1)?, opt_int(w, 1)?];
                if s.iter().any(|v| *v < 1) {
                    return Err("bad stride".into());
                }
                Ok(s.map(|v| v as usize))
            };
            let mut runtime = vec![input(0)?];
            let op = match code {
                op::DEQUANTIZE => Op::Reshape,
                op::CONV_2D | op::DEPTHWISE_CONV_2D => {
                    let depthwise = code == op::DEPTHWISE_CONV_2D;
                    let (padding, s) = (opt_byte(0, 0)?, stride(1, 2)?);
                    let act_id = if depthwise { 4 } else { 3 };
                    let dil = if depthwise { [opt_int(6, 1)?, opt_int(5, 1)?] } else { [opt_int(5, 1)?, opt_int(4, 1)?] };
                    if dil != [1, 1] {
                        return Err(format!("{}: dilated convolutions are not supported", names[output]));
                    }
                    let act = Act::from(opt_byte(act_id, 0)?)?;
                    let (x, f) = (in_shape(0)?, shapes[input(1)?].clone());
                    let (&[1, h, w, cin], &[fo, kh, kw, fi], &[1, oh, ow, cout]) = (&x[..], &f[..], &out_shape[..]) else {
                        return Err(format!("{}: unexpected convolution shapes", names[output]));
                    };
                    if kh != kw || s[0] != s[1] {
                        return Err(format!("{}: only square kernels and strides are supported", names[output]));
                    }
                    if kh > MAX_KERNEL {
                        return Err(format!("{}: the convolution kernel is too large", names[output]));
                    }
                    let (ey, py) = window(padding, h, kh, s[0])?;
                    let (ex, px) = window(padding, w, kw, s[1])?;
                    if (ey, ex) != (oh, ow) {
                        return Err(format!("{}: output size disagrees with the padding", names[output]));
                    }
                    let wt = weights(1)?;
                    let b = bias(cout)?;
                    if depthwise {
                        if fo != 1 || fi != cout || cin != cout {
                            return Err(format!("{}: depth multipliers are not supported", names[output]));
                        }
                        Op::Depthwise(Depthwise { w: wt, b, c: cout, ks: kh, stride: s[0] }, [py, px], act)
                    } else {
                        if fo != cout || fi != cin {
                            return Err(format!("{}: unexpected filter shape", names[output]));
                        }
                        // OHWI → [(ky·ks + kx)·inp + ci] × out.
                        let kk = kh * kw * cin;
                        spend(kk * cout)?;
                        let mut t = vec![0.0f32; kk * cout];
                        for o in 0..cout {
                            for p in 0..kk {
                                t[p * cout + o] = *wt.get(o * kk + p).ok_or("bad filter")?;
                            }
                        }
                        if kh == 1 && s[0] == 1 {
                            Op::Pointwise(Linear { wt: t, b, inp: cin, out: cout }, act)
                        } else {
                            Op::Conv(Conv { wt: t, b, inp: cin, out: cout, ks: kh, stride: s[0], pad: 0 }, [py, px], act)
                        }
                    }
                }
                op::MAX_POOL_2D => {
                    let (padding, s) = (opt_byte(0, 0)?, stride(1, 2)?);
                    let k = [opt_int(4, 1)?, opt_int(3, 1)?];
                    if k.iter().any(|v| *v < 1) {
                        return Err("bad pool size".into());
                    }
                    let k = k.map(|v| v as usize);
                    if k.iter().any(|&v| v > MAX_KERNEL) {
                        return Err(format!("{}: the pooling window is too large", names[output]));
                    }
                    let x = in_shape(0)?;
                    let (&[1, h, w, c], &[1, oh, ow, oc]) = (&x[..], &out_shape[..]) else { return Err("unexpected pool shapes".into()) };
                    if c != oc {
                        return Err(format!("{}: pooling changes the channels", names[output]));
                    }
                    let (ey, py) = window(padding, h, k[0], s[0])?;
                    let (ex, px) = window(padding, w, k[1], s[1])?;
                    if (ey, ex) != (oh, ow) {
                        return Err(format!("{}: output size disagrees with the padding", names[output]));
                    }
                    Op::MaxPool { k, stride: s, pad: [py, px], act: Act::from(opt_byte(5, 0)?)? }
                }
                op::ADD => {
                    runtime.push(input(1)?);
                    Op::Add(Act::from(opt_byte(0, 0)?)?)
                }
                op::PAD => {
                    let p = ints[input(1)?].clone().ok_or("PAD: paddings must be constant")?;
                    let pads = p
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| Ok([usize::try_from(c[0]).map_err(|_| "bad pad")?, usize::try_from(c[1]).map_err(|_| "bad pad")?]))
                        .collect::<Result<Vec<_>>>()?;
                    // The padded shape is the declared one (so it is as bounded as any tensor).
                    let x = in_shape(0)?;
                    let padded = x.iter().zip(&pads).map(|(d, p)| d.checked_add(p[0])?.checked_add(p[1])).collect::<Option<Vec<_>>>();
                    if pads.len() != x.len() || padded.as_ref() != Some(&out_shape) {
                        return Err(format!("{}: the padding disagrees with the output shape", names[output]));
                    }
                    Op::Pad(pads)
                }
                op::CONCATENATION => {
                    runtime = (0..ins.len()).map(input).collect::<Result<_>>()?;
                    let rank = out_shape.len() as i32;
                    let a = opt_int(0, 0)?;
                    let axis = index(if a < 0 { a.saturating_add(rank) } else { a }, out_shape.len())?;
                    // The parts fit the output: same rank, same sizes but along `axis`, where they add up.
                    let mut along = 0usize;
                    for &id in &runtime {
                        let p = &shapes[id];
                        let fits = p.len() == out_shape.len() && p.iter().zip(&out_shape).enumerate().all(|(d, (a, b))| d == axis || a == b);
                        along = p
                            .get(axis)
                            .filter(|_| fits)
                            .and_then(|&d| along.checked_add(d))
                            .ok_or_else(|| format!("{}: the parts disagree with the output shape", names[output]))?;
                    }
                    if out_shape.get(axis) != Some(&along) {
                        return Err(format!("{}: the parts disagree with the output shape", names[output]));
                    }
                    Op::Concat { axis, act: Act::from(opt_byte(1, 0)?)? }
                }
                op::RESHAPE => Op::Reshape,
                op::RELU => Op::Act(Act::Relu),
                op::RELU6 => Op::Act(Act::Relu6),
                op::LOGISTIC => Op::Logistic,
                op::PRELU => {
                    let alpha = weights(1)?;
                    let c = *out_shape.last().ok_or("PRELU: bad shape")?;
                    if alpha.len() != c {
                        return Err(format!("{}: only per-channel PReLU is supported", names[output]));
                    }
                    Op::Prelu(alpha)
                }
                c => return Err(format!("operator {c} is not supported")),
            };
            steps.push(Step { op, inputs: runtime, output });
        }
        let mut last_use = vec![usize::MAX; n];
        for (k, s) in steps.iter().enumerate() {
            for &i in &s.inputs {
                last_use[i] = k;
            }
        }
        for &o in &outputs {
            last_use[o] = usize::MAX;
        }
        let model = Model { shapes, names, consts: floats, steps, inputs, outputs, last_use };
        let c = model.cost().ok_or("the model is too expensive to run")?;
        let over = [
            (c.kernel > MAX_KERNEL as u64, "a kernel is too large"),
            (c.scratch > MAX_SCRATCH, "an operator's working space is too large"),
            (c.macs > MAX_MACS, "the model needs too many multiply-adds"),
            (c.values > MAX_VALUES, "the model does too much work"),
            (c.live > MAX_LIVE, "the model needs too much memory"),
        ];
        if let Some((_, why)) = over.iter().find(|o| o.0) {
            return Err((*why).to_string());
        }
        Ok(model)
    }

    /// What one run takes, from the static shapes (`None` if a count overflows).
    fn cost(&self) -> Option<Cost> {
        let size = |i: usize| -> Option<u64> { self.shapes.get(i)?.iter().try_fold(1u64, |a, &d| a.checked_mul(d as u64)) };
        let mut c = Cost::default();
        // Floats alive per tensor, as `run` keeps them: inputs are copied in, and every step's
        // output stays until its last use (for good when nothing uses it).
        let mut alive = vec![0u64; self.shapes.len()];
        let mut total = 0u64;
        for &i in &self.inputs {
            total = total.checked_sub(*alive.get(i)?)?.checked_add(size(i)?)?;
            *alive.get_mut(i)? = size(i)?;
        }
        c.live = total;
        for (k, s) in self.steps.iter().enumerate() {
            let &x = s.inputs.first()?;
            let (xs, ys) = (self.shapes.get(x)?, self.shapes.get(s.output)?);
            let dim = |sh: &[usize], d: usize| sh.get(d).copied().unwrap_or(1) as u64;
            let (h, w, oh, ow) = (dim(xs, 1), dim(xs, 2), dim(ys, 1), dim(ys, 2));
            let out = size(s.output)?;
            let mut scratch = 0u64;
            let macs = match &s.op {
                Op::Pointwise(l, _) => h.checked_mul(w)?.checked_mul(l.inp as u64)?.checked_mul(l.out as u64)?,
                Op::Conv(cv, ..) => {
                    let ks = cv.ks as u64;
                    c.kernel = c.kernel.max(ks);
                    let kk = ks.checked_mul(ks)?.checked_mul(cv.inp as u64)?;
                    scratch = ow.checked_mul(kk)?;
                    oh.checked_mul(ow)?.checked_mul(cv.out as u64)?.checked_mul(kk)?
                }
                Op::Depthwise(d, ..) => {
                    let ks = d.ks as u64;
                    c.kernel = c.kernel.max(ks);
                    oh.checked_mul(ow)?.checked_mul(d.c as u64)?.checked_mul(ks)?.checked_mul(ks)?
                }
                Op::MaxPool { k, .. } => {
                    c.kernel = c.kernel.max(k[0].max(k[1]) as u64);
                    out.checked_mul(k[0] as u64)?.checked_mul(k[1] as u64)?
                }
                _ => 0,
            };
            c.macs = c.macs.checked_add(macs)?;
            c.values = c.values.checked_add(out)?;
            c.scratch = c.scratch.max(scratch);
            // The output and the scratch are allocated while the inputs (and any earlier value of
            // the output) are still alive.
            c.live = c.live.max(total.checked_add(out)?.checked_add(scratch)?);
            total = total.checked_sub(*alive.get(s.output)?)?.checked_add(out)?;
            *alive.get_mut(s.output)? = out;
            for &i in &s.inputs {
                if self.last_use.get(i) == Some(&k) {
                    total = total.checked_sub(*alive.get(i)?)?;
                    *alive.get_mut(i)? = 0;
                }
            }
        }
        // Outputs that are constants are copied out.
        for &o in &self.outputs {
            if *alive.get(o)? == 0 {
                total = total.checked_add(size(o)?)?;
            }
        }
        c.live = c.live.max(total);
        Some(c)
    }

    /// The shape of input `k`.
    pub fn input_shape(&self, k: usize) -> Option<&[usize]> {
        self.inputs.get(k).and_then(|&i| self.shapes.get(i)).map(Vec::as_slice)
    }

    /// The shape and name of output `k`.
    pub fn output(&self, k: usize) -> Option<(&[usize], &str)> {
        let &i = self.outputs.get(k)?;
        Some((self.shapes.get(i)?.as_slice(), self.names.get(i)?.as_str()))
    }

    /// Run the model on its inputs (row-major, NHWC); returns its outputs.
    pub fn run(&self, inputs: &[&[f32]]) -> Result<Vec<Vec<f32>>> {
        let mut vals: Vec<Option<Vec<f32>>> = vec![None; self.shapes.len()];
        for (k, &id) in self.inputs.iter().enumerate() {
            let x = inputs.get(k).ok_or("missing model input")?;
            if x.len() != self.shapes[id].iter().product::<usize>() {
                return Err(format!("input {k}: expected {:?}", self.shapes[id]));
            }
            vals[id] = Some(x.to_vec());
        }
        for (k, s) in self.steps.iter().enumerate() {
            let y = {
                let get = |i: usize| -> Result<&[f32]> {
                    vals.get(i)
                        .and_then(Option::as_deref)
                        .or_else(|| self.consts.get(i).and_then(Option::as_deref))
                        .ok_or_else(|| format!("{}: input not computed", self.names[s.output]))
                };
                let x = get(s.inputs[0])?;
                let xs = &self.shapes[s.inputs[0]];
                let ys = &self.shapes[s.output];
                let (h, w) = (xs.get(1).copied().unwrap_or(1), xs.get(2).copied().unwrap_or(1));
                let (oh, ow) = (ys.get(1).copied().unwrap_or(1), ys.get(2).copied().unwrap_or(1));
                let y = match &s.op {
                    Op::Pointwise(l, a) => {
                        let mut y = l.forward(x, h * w);
                        a.apply(&mut y);
                        y
                    }
                    Op::Conv(c, pad, a) => {
                        let mut y = c.forward_sized(x, h, w, oh, ow, *pad);
                        a.apply(&mut y);
                        y
                    }
                    Op::Depthwise(d, pad, a) => {
                        let mut y = d.forward_sized(x, h, w, oh, ow, *pad);
                        a.apply(&mut y);
                        y
                    }
                    Op::MaxPool { k, stride, pad, act } => {
                        let mut y = max_pool(x, [h, w], *xs.last().unwrap_or(&1), [oh, ow], *k, *stride, *pad);
                        act.apply(&mut y);
                        y
                    }
                    Op::Add(a) => {
                        let b = get(s.inputs[1])?;
                        if b.len() != x.len() {
                            return Err(format!("{}: broadcasting ADD is not supported", self.names[s.output]));
                        }
                        let mut y: Vec<f32> = x.iter().zip(b).map(|(p, q)| p + q).collect();
                        a.apply(&mut y);
                        y
                    }
                    Op::Pad(p) => pad(x, xs, p)?,
                    Op::Concat { axis, act } => {
                        let parts = s.inputs.iter().map(|&i| Ok((get(i)?, &self.shapes[i]))).collect::<Result<Vec<_>>>()?;
                        let mut y = concat(&parts, *axis)?;
                        act.apply(&mut y);
                        y
                    }
                    Op::Reshape => x.to_vec(),
                    Op::Act(a) => {
                        let mut y = x.to_vec();
                        a.apply(&mut y);
                        y
                    }
                    Op::Prelu(alpha) => {
                        let c = alpha.len().max(1);
                        x.iter().enumerate().map(|(i, &v)| if v >= 0.0 { v } else { v * alpha[i % c] }).collect()
                    }
                    Op::Logistic => x.iter().map(|v| 1.0 / (1.0 + (-v).exp())).collect(),
                };
                if y.len() != ys.iter().product::<usize>() {
                    return Err(format!("{}: computed {} values for shape {ys:?}", self.names[s.output], y.len()));
                }
                y
            };
            vals[s.output] = Some(y);
            for &i in &s.inputs {
                if self.last_use[i] == k {
                    vals[i] = None;
                }
            }
        }
        self.outputs.iter().map(|&o| vals[o].take().or_else(|| self.consts[o].clone()).ok_or_else(|| "an output was not computed".to_string())).collect()
    }
}

/// What running a model takes, worked out at load from its static shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cost {
    /// Multiply-adds (convolutions) and window comparisons (pooling).
    macs: u64,
    /// Values all operators write.
    values: u64,
    /// The most floats alive at once.
    live: u64,
    /// The largest working space one operator allocates, in floats.
    scratch: u64,
    /// The widest window side.
    kernel: u64,
}

/// Max pooling over `k` windows; window cells outside the input are ignored.
fn max_pool(x: &[f32], [h, w]: [usize; 2], c: usize, [oh, ow]: [usize; 2], k: [usize; 2], s: [usize; 2], pad: [usize; 2]) -> Vec<f32> {
    let mut y = vec![f32::NEG_INFINITY; oh * ow * c];
    for oy in 0..oh {
        for ox in 0..ow {
            let out = &mut y[(oy * ow + ox) * c..][..c];
            for ky in 0..k[0] {
                let Some(iy) = (oy * s[0] + ky).checked_sub(pad[0]).filter(|&v| v < h) else { continue };
                for kx in 0..k[1] {
                    let Some(ix) = (ox * s[1] + kx).checked_sub(pad[1]).filter(|&v| v < w) else { continue };
                    let Some(src) = x.get((iy * w + ix) * c..(iy * w + ix + 1) * c) else { continue };
                    for (o, v) in out.iter_mut().zip(src) {
                        *o = o.max(*v);
                    }
                }
            }
        }
    }
    y
}

/// Zero padding of a row-major tensor of shape `shape`.
fn pad(x: &[f32], shape: &[usize], pads: &[[usize; 2]]) -> Result<Vec<f32>> {
    if pads.len() != shape.len() {
        return Err("PAD: rank mismatch".into());
    }
    let out: Vec<usize> = shape.iter().zip(pads).map(|(d, p)| d + p[0] + p[1]).collect();
    let mut y = vec![0.0f32; out.iter().product()];
    let rank = shape.len();
    let inner = *shape.last().unwrap_or(&1);
    let rows = x.len() / inner.max(1);
    // Copy each innermost row to its padded place.
    let mut idx = vec![0usize; rank.saturating_sub(1)];
    for r in 0..rows {
        let mut at = 0usize;
        for d in 0..rank {
            let i = if d + 1 == rank { pads[d][0] } else { idx[d] + pads[d][0] };
            at = at * out[d] + i;
        }
        if let (Some(dst), Some(src)) = (y.get_mut(at..at + inner), x.get(r * inner..(r + 1) * inner)) {
            dst.copy_from_slice(src);
        }
        for d in (0..idx.len()).rev() {
            idx[d] += 1;
            if idx[d] < shape[d] {
                break;
            }
            idx[d] = 0;
        }
    }
    Ok(y)
}

/// Concatenate along `axis`.
fn concat(parts: &[(&[f32], &Vec<usize>)], axis: usize) -> Result<Vec<f32>> {
    let Some((_, s0)) = parts.first() else { return Ok(vec![]) };
    let outer: usize = s0.get(..axis).ok_or("CONCATENATION: bad axis")?.iter().product();
    let mut y = Vec::with_capacity(parts.iter().map(|p| p.0.len()).sum());
    for o in 0..outer {
        for (x, s) in parts {
            let chunk: usize = s.get(axis..).ok_or("CONCATENATION: bad axis")?.iter().product();
            y.extend_from_slice(x.get(o * chunk..(o + 1) * chunk).ok_or("CONCATENATION: bad input")?);
        }
    }
    Ok(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_padding_matches_tensorflow() {
        // TensorFlow pads the extra row after: 128 → 64 with a 3×3 stride-2 window pads 0 before.
        assert_eq!(same(128, 3, 2), (64, 0));
        assert_eq!(same(128, 5, 2), (64, 1));
        assert_eq!(same(16, 3, 1), (16, 1));
        assert_eq!(same(7, 2, 2), (4, 0));
        assert_eq!(window(1, 4, 2, 2), Ok((2, 0)));
        assert!(window(1, 1, 2, 2).is_err());
    }

    #[test]
    fn asymmetric_conv_padding() {
        // A 3×3 stride-2 "same" conv on 4×4: pads nothing before, one after.
        let x: Vec<f32> = (0..16).map(|v| v as f32).collect();
        let ones = vec![1.0f32; 9];
        let c = Conv { wt: ones.clone(), b: vec![0.0], inp: 1, out: 1, ks: 3, stride: 2, pad: 0 };
        let y = c.forward_sized(&x, 4, 4, 2, 2, [0, 0]);
        // Top-left window rows 0–2, cols 0–2; the bottom-right one is cut by the edge.
        assert_eq!(y, vec![45.0, 39.0, 66.0, 50.0]);
        let d = Depthwise { w: ones, b: vec![0.0], c: 1, ks: 3, stride: 2 };
        assert_eq!(d.forward_sized(&x, 4, 4, 2, 2, [0, 0]), y);
    }

    #[test]
    fn pool_pad_and_concat() {
        let x: Vec<f32> = (0..16).map(|v| v as f32).collect();
        assert_eq!(max_pool(&x, [4, 4], 1, [2, 2], [2, 2], [2, 2], [0, 0]), vec![5.0, 7.0, 13.0, 15.0]);
        // Same padding on 3×3 → 2×2: the last window is cut by the edge.
        let x3: Vec<f32> = (0..9).map(|v| v as f32).collect();
        assert_eq!(max_pool(&x3, [3, 3], 1, [2, 2], [2, 2], [2, 2], [0, 0]), vec![4.0, 5.0, 7.0, 8.0]);
        // Channel padding (residual connections widen the channels with zeros).
        let p = pad(&[1.0, 2.0, 3.0, 4.0], &[1, 1, 2, 2], &[[0, 0], [0, 0], [0, 0], [0, 1]]).unwrap();
        assert_eq!(p, vec![1.0, 2.0, 0.0, 3.0, 4.0, 0.0]);
        let p = pad(&[1.0, 2.0], &[1, 2], &[[0, 0], [1, 0]]).unwrap();
        assert_eq!(p, vec![0.0, 1.0, 2.0]);
        let a = [1.0, 2.0, 3.0, 4.0];
        let b = [5.0, 6.0];
        let (sa, sb) = (vec![1, 2, 2], vec![1, 1, 2]);
        assert_eq!(concat(&[(&a, &sa), (&b, &sb)], 1).unwrap(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let (sa, sb) = (vec![2, 2], vec![2, 1]);
        assert_eq!(concat(&[(&a, &sa), (&b, &sb)], 1).unwrap(), vec![1.0, 2.0, 5.0, 3.0, 4.0, 6.0]);
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        assert!(Model::read(b"").is_err());
        assert!(Model::read(&[0xff; 64]).is_err());
        // Every prefix and a few corruptions of a hand-made buffer.
        let mut fb = vec![12, 0, 0, 0, 0, 0, 6, 0, 8, 0, 4, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        for n in 0..fb.len() {
            let _ = Model::read(&fb[..n]);
        }
        for i in 0..fb.len() {
            fb[i] ^= 0x5a;
            let _ = Model::read(&fb);
        }
    }

    /// A FlatBuffer value, to build small models in tests.
    enum V {
        Int(i32),
        Byte(u8),
        Table(Vec<(usize, V)>),
        Ints(Vec<i32>),
        Bytes(Vec<u8>),
        Tables(Vec<V>),
    }

    fn put_u32(b: &mut [u8], at: usize, v: usize) {
        b[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
    }

    /// Append `v` (children after their parent, so every offset points forward); where it starts.
    fn put(b: &mut Vec<u8>, v: &V) -> usize {
        match v {
            V::Table(fields) => {
                let slots = fields.iter().map(|f| f.0 + 1).max().unwrap_or(0);
                let vt = b.len();
                let vt_len = 4 + 2 * slots;
                b.resize(vt + vt_len, 0);
                let t = b.len();
                // Every field gets 8 bytes (the reader does not need alignment).
                let t_len = 4 + 8 * fields.len();
                b.resize(t + t_len, 0);
                b[vt..vt + 2].copy_from_slice(&(vt_len as u16).to_le_bytes());
                b[vt + 2..vt + 4].copy_from_slice(&(t_len as u16).to_le_bytes());
                b[t..t + 4].copy_from_slice(&((t - vt) as i32).to_le_bytes());
                for (k, (id, f)) in fields.iter().enumerate() {
                    let at = t + 4 + 8 * k;
                    b[vt + 4 + 2 * id..vt + 6 + 2 * id].copy_from_slice(&((at - t) as u16).to_le_bytes());
                    match f {
                        V::Int(i) => b[at..at + 4].copy_from_slice(&i.to_le_bytes()),
                        V::Byte(x) => b[at] = *x,
                        _ => {
                            let child = put(b, f);
                            put_u32(b, at, child - at);
                        }
                    }
                }
                t
            }
            V::Ints(xs) => {
                let p = b.len();
                b.extend((xs.len() as u32).to_le_bytes());
                xs.iter().for_each(|x| b.extend(x.to_le_bytes()));
                p
            }
            V::Bytes(xs) => {
                let p = b.len();
                b.extend((xs.len() as u32).to_le_bytes());
                b.extend(xs);
                p
            }
            V::Tables(ts) => {
                let p = b.len();
                b.extend((ts.len() as u32).to_le_bytes());
                let slots = b.len();
                b.resize(slots + 4 * ts.len(), 0);
                for (i, t) in ts.iter().enumerate() {
                    let child = put(b, t);
                    put_u32(b, slots + 4 * i, child - (slots + 4 * i));
                }
                p
            }
            V::Int(_) | V::Byte(_) => b.len(),
        }
    }

    /// A tensor: shape, type and its constant data (`Some(k)`: buffer `k` of the model's).
    type T = (Vec<i32>, u8, Option<usize>);
    /// An operator: code, inputs, outputs and its options' fields.
    type O = (i32, Vec<i32>, Vec<i32>, Vec<(usize, V)>);

    /// A `.tflite` file with one graph.
    fn model(bufs: Vec<Vec<u8>>, tensors: Vec<T>, ops: Vec<O>, inputs: Vec<i32>, outputs: Vec<i32>) -> Vec<u8> {
        let ts = tensors
            .into_iter()
            .enumerate()
            .map(|(i, (shape, ty, buf))| {
                let mut f = vec![(0, V::Ints(shape)), (1, V::Byte(ty)), (3, V::Bytes(format!("t{i}").into_bytes()))];
                if let Some(k) = buf {
                    f.push((2, V::Int(k as i32 + 1)));
                }
                V::Table(f)
            })
            .collect();
        let mut codes = vec![];
        let os = ops
            .into_iter()
            .map(|(code, ins, outs, opts)| {
                let idx = codes.iter().position(|&c| c == code).unwrap_or_else(|| {
                    codes.push(code);
                    codes.len() - 1
                });
                V::Table(vec![(0, V::Int(idx as i32)), (1, V::Ints(ins)), (2, V::Ints(outs)), (4, V::Table(opts))])
            })
            .collect();
        let g = V::Table(vec![(0, V::Tables(ts)), (1, V::Ints(inputs)), (2, V::Ints(outputs)), (3, V::Tables(os))]);
        let buffers = std::iter::once(V::Table(vec![])).chain(bufs.into_iter().map(|d| V::Table(vec![(0, V::Bytes(d))]))).collect();
        let codes = codes.into_iter().map(|c| V::Table(vec![(3, V::Int(c))])).collect();
        let root = V::Table(vec![(1, V::Tables(codes)), (2, V::Tables(vec![g])), (4, V::Tables(buffers))]);
        let mut b = vec![0u8; 4];
        let r = put(&mut b, &root);
        put_u32(&mut b, 0, r);
        b
    }

    fn f32s(xs: &[f32]) -> Vec<u8> {
        xs.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// Conv2DOptions / Pool2DOptions: padding (0 same, 1 valid) and stride.
    fn window_opts(padding: u8, stride: i32) -> Vec<(usize, V)> {
        vec![(0, V::Byte(padding)), (1, V::Int(stride)), (2, V::Int(stride))]
    }

    fn refused(bytes: &[u8], why: &str) {
        let e = Model::read(bytes).err().unwrap_or_default();
        assert!(e.contains(why), "refused for `{e}`, expected `{why}`");
    }

    #[test]
    fn built_models_load_and_run() {
        // A 3×3 stride-2 "same" conv on 4×4, then a RELU: as `asymmetric_conv_padding`.
        let m = model(
            vec![f32s(&[1.0; 9])],
            vec![(vec![1, 4, 4, 1], FLOAT32, None), (vec![1, 3, 3, 1], FLOAT32, Some(0)), (vec![1, 2, 2, 1], FLOAT32, None), (vec![1, 2, 2, 1], FLOAT32, None)],
            vec![(op::CONV_2D, vec![0, 1, -1], vec![2], window_opts(0, 2)), (op::RELU, vec![2], vec![3], vec![])],
            vec![0],
            vec![3],
        );
        let m = Model::read(&m).unwrap();
        let c = m.cost().unwrap();
        assert_eq!((c.macs, c.values, c.scratch, c.kernel), (2 * 2 * 9, 8, 2 * 9, 3));
        // The input, the conv's output and its patch row, alive together.
        assert_eq!(c.live, 16 + 4 + 2 * 9);
        let x: Vec<f32> = (0..16).map(|v| v as f32).collect();
        assert_eq!(m.run(&[&x]).unwrap(), vec![vec![45.0, 39.0, 66.0, 50.0]]);
    }

    /// #368: an 8 KB model whose CONV_2D needed a 512 GB scratch buffer aborted `run`.
    #[test]
    fn convolution_scratch_is_bounded() {
        let w = 1 << 25;
        let bomb = model(
            vec![vec![0; 2 * 64 * 64]],
            vec![(vec![1, 1, w, 1], FLOAT32, None), (vec![1, 64, 64, 1], FLOAT16, Some(0)), (vec![1, 1, w, 1], FLOAT32, None)],
            vec![(op::CONV_2D, vec![0, 1, -1], vec![2], window_opts(0, 1))],
            vec![0],
            vec![2],
        );
        refused(&bomb, "kernel is too large");
        // A small kernel over many channels: the patch row is `ow·3·3·cin`.
        let (w, cin) = (1 << 20, 256);
        let wide = model(
            vec![vec![0; 2 * 9 * 256]],
            vec![(vec![1, 1, w, cin], FLOAT32, None), (vec![1, 3, 3, cin], FLOAT16, Some(0)), (vec![1, 1, w, 1], FLOAT32, None)],
            vec![(op::CONV_2D, vec![0, 1, -1], vec![2], window_opts(0, 1))],
            vec![0],
            vec![2],
        );
        refused(&wide, "working space is too large");
    }

    #[test]
    fn kernel_sides_are_bounded() {
        let k = MAX_KERNEL as i32 + 1;
        let conv = model(
            vec![vec![0; 2 * (k * k) as usize]],
            vec![(vec![1, 40, 40, 1], FLOAT32, None), (vec![1, k, k, 1], FLOAT16, Some(0)), (vec![1, 40, 40, 1], FLOAT32, None)],
            vec![(op::CONV_2D, vec![0, 1, -1], vec![2], window_opts(0, 1))],
            vec![0],
            vec![2],
        );
        refused(&conv, "kernel is too large");
        let depthwise = model(
            vec![vec![0; 2 * (k * k) as usize]],
            vec![(vec![1, 40, 40, 1], FLOAT32, None), (vec![1, k, k, 1], FLOAT16, Some(0)), (vec![1, 40, 40, 1], FLOAT32, None)],
            vec![(op::DEPTHWISE_CONV_2D, vec![0, 1, -1], vec![2], window_opts(0, 1))],
            vec![0],
            vec![2],
        );
        refused(&depthwise, "kernel is too large");
        // A "same" pool takes any window: a huge one looped for billions of steps per output.
        let mut opts = window_opts(0, 1);
        opts.extend([(3, V::Int(i32::MAX)), (4, V::Int(i32::MAX))]);
        let pool = model(
            vec![],
            vec![(vec![1, 8, 8, 1], FLOAT32, None), (vec![1, 8, 8, 1], FLOAT32, None)],
            vec![(op::MAX_POOL_2D, vec![0], vec![1], opts)],
            vec![0],
            vec![1],
        );
        refused(&pool, "pooling window is too large");
    }

    #[test]
    fn multiply_adds_are_bounded() {
        // Three 16 → 16 pointwise convolutions over 4096×4096: 3·2³² multiply-adds.
        let s = vec![1, 4096, 4096, 16];
        let t = |buf| (s.clone(), FLOAT32, buf);
        let m = model(
            vec![vec![0; 2 * 16 * 16]],
            vec![t(None), (vec![16, 1, 1, 16], FLOAT16, Some(0)), t(None), t(None), t(None)],
            (0..3).map(|i| (op::CONV_2D, vec![if i == 0 { 0 } else { i + 1 }, 1, -1], vec![i + 2], window_opts(0, 1))).collect(),
            vec![0],
            vec![4],
        );
        refused(&m, "too many multiply-adds");
    }

    /// #368: 2000 RELUs over 2²⁵ values loaded fine, and one run took minutes.
    #[test]
    fn elementwise_work_is_bounded() {
        let n = 2000;
        let m = model(
            vec![],
            (0..=n).map(|_| (vec![1, 1 << 25], FLOAT32, None)).collect(),
            (0..n).map(|i| (op::RELU, vec![i], vec![i + 1], vec![])).collect(),
            vec![0],
            vec![n],
        );
        refused(&m, "too much work");
    }

    #[test]
    fn memory_alive_at_once_is_bounded() {
        // Five outputs of 2²⁷ values each, all kept, besides the input.
        let m = model(
            vec![],
            (0..6).map(|_| (vec![1, 1 << 27], FLOAT32, None)).collect(),
            (1..6).map(|i| (op::RELU, vec![0], vec![i], vec![])).collect(),
            vec![0],
            (1..6).collect(),
        );
        refused(&m, "too much memory");
    }

    #[test]
    fn shapes_computed_at_run_time_must_match_the_declared_ones() {
        // PAD to 2³⁰ × 2³⁰ while declaring a 1×1 output.
        let pads = vec![0, 1 << 30, 0, 1 << 30].into_iter().flat_map(i32::to_le_bytes).collect();
        let pad = model(
            vec![pads],
            vec![(vec![1, 1], FLOAT32, None), (vec![2, 2], INT32, Some(0)), (vec![1, 1], FLOAT32, None)],
            vec![(op::PAD, vec![0, 1], vec![2], vec![])],
            vec![0],
            vec![2],
        );
        refused(&pad, "padding disagrees");
        // Many large parts concatenated into a small declared output.
        let concat = model(
            vec![],
            vec![(vec![1, 1 << 27], FLOAT32, None), (vec![1, 2], FLOAT32, None)],
            vec![(op::CONCATENATION, vec![0; 64], vec![1], vec![(0, V::Int(1))])],
            vec![0],
            vec![1],
        );
        refused(&concat, "parts disagree");
        // A pool whose output claims more channels than its input has.
        let pool = model(
            vec![],
            vec![(vec![1, 2, 2, 1], FLOAT32, None), (vec![1, 1, 1, 1 << 20], FLOAT32, None)],
            vec![(op::MAX_POOL_2D, vec![0], vec![1], window_opts(1, 2).into_iter().chain([(3, V::Int(2)), (4, V::Int(2))]).collect())],
            vec![0],
            vec![1],
        );
        refused(&pool, "changes the channels");
        // Dimensions may not hide behind a zero.
        let zero = model(vec![], vec![(vec![0, 1 << 30, 1 << 30], FLOAT32, None)], vec![], vec![0], vec![0]);
        refused(&zero, "tensor is too large");
    }

    #[test]
    fn shared_weights_count_once_per_use() {
        // One 128 KB buffer decoded for 2100 tensors would be 2100 copies of 2¹⁶ floats.
        let m = model(vec![vec![0; 2 << 16]], (0..2100).map(|_| (vec![1, 1 << 16], FLOAT16, Some(0))).collect(), vec![], vec![], vec![]);
        refused(&m, "weights are too large");
    }

    /// The official face models (`EFFECTCRAFT_FACE_LANDMARKER` = path to `face_landmarker.task`)
    /// fit every budget with wide headroom.
    #[test]
    fn official_face_models_fit_the_budgets() {
        let Some(path) = std::env::var_os("EFFECTCRAFT_FACE_LANDMARKER") else { return };
        let bytes = std::fs::read(path).unwrap();
        for name in [crate::mediapipe::DETECTOR, crate::mediapipe::MESH] {
            let c = Model::read(&crate::pt::zip_file(&bytes, name).unwrap()).unwrap().cost().unwrap();
            println!("{name}: {c:?}");
            assert!(c.macs <= MAX_MACS / 8 && c.values <= MAX_VALUES / 8 && c.live <= MAX_LIVE / 8 && c.scratch <= MAX_SCRATCH / 8, "{name}: {c:?}");
            assert!(c.kernel <= MAX_KERNEL as u64 / 4, "{name}: {c:?}");
        }
    }
}

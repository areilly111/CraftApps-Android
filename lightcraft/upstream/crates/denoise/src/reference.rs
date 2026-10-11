//! A plain-loop interpreter of a [`Net`], slow and obviously right: the reference the parser and the GPU runner are
//! tested against (tract's answer on the real model is the other). It is not used on photos.

use crate::net::{Net, NetError, Op, TensorId};
use crate::run::{Error, TileRunner};

/// A tensor in `[channels][side][side]` order.
struct T {
    channels: usize,
    side: usize,
    data: Vec<f32>,
}

/// Run `net` on `input` (`in_channels` planes of `tile × tile`).
pub fn run(net: &Net, tile: usize, input: &[f32]) -> Result<Vec<f32>, NetError> {
    if !net.fits_tile(tile) || input.len() != net.in_channels * tile * tile {
        return Err(NetError::Invalid("the tile does not fit the network".into()));
    }
    let mut tensors: Vec<T> = vec![T { channels: net.in_channels, side: tile, data: input.to_vec() }];
    let get = |tensors: &[T], id: TensorId| -> Result<usize, NetError> {
        if id < tensors.len() { Ok(id) } else { Err(NetError::Invalid("a layer reads a tensor that does not exist yet".into())) }
    };
    for op in net.ops() {
        let out = match op {
            Op::Conv(c) => {
                let a = get(&tensors, c.src)?;
                let b = c.src2.map(|s| get(&tensors, s)).transpose()?;
                let side = tensors[a].side;
                let pad = (c.k - 1) / 2;
                let mut data = vec![0f32; c.cout * side * side];
                // the input channels in order: the first source's, then the second's
                let planes: Vec<&[f32]> =
                    std::iter::once(&tensors[a]).chain(b.map(|b| &tensors[b])).flat_map(|t| t.data.chunks(side * side)).collect();
                for co in 0..c.cout {
                    for y in 0..side {
                        for x in 0..side {
                            let mut sum = c.bias[co];
                            for (ci, plane) in planes.iter().enumerate() {
                                for ky in 0..c.k {
                                    for kx in 0..c.k {
                                        let (yy, xx) = ((y + ky) as isize - pad as isize, (x + kx) as isize - pad as isize);
                                        if yy < 0 || xx < 0 || yy >= side as isize || xx >= side as isize {
                                            continue;
                                        }
                                        sum += plane[yy as usize * side + xx as usize] * c.weight[((co * c.cin + ci) * c.k + ky) * c.k + kx];
                                    }
                                }
                            }
                            data[(co * side + y) * side + x] = leaky(sum, c.leaky);
                        }
                    }
                }
                T { channels: c.cout, side, data }
            }
            Op::ConvT2(c) => {
                let a = get(&tensors, c.src)?;
                let side = tensors[a].side;
                let out_side = 2 * side;
                let mut data = vec![0f32; c.cout * out_side * out_side];
                for co in 0..c.cout {
                    for y in 0..side {
                        for x in 0..side {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    let mut sum = c.bias[co];
                                    for ci in 0..c.cin {
                                        sum += tensors[a].data[(ci * side + y) * side + x] * c.weight[((ci * c.cout + co) * 2 + dy) * 2 + dx];
                                    }
                                    data[(co * out_side + 2 * y + dy) * out_side + 2 * x + dx] = leaky(sum, c.leaky);
                                }
                            }
                        }
                    }
                }
                T { channels: c.cout, side: out_side, data }
            }
            Op::MaxPool2 { src } => {
                let a = get(&tensors, *src)?;
                let (channels, side) = (tensors[a].channels, tensors[a].side / 2);
                let mut data = vec![0f32; channels * side * side];
                for ch in 0..channels {
                    for y in 0..side {
                        for x in 0..side {
                            let at = |dy: usize, dx: usize| tensors[a].data[(ch * 2 * side + 2 * y + dy) * 2 * side + 2 * x + dx];
                            data[(ch * side + y) * side + x] = at(0, 0).max(at(0, 1)).max(at(1, 0)).max(at(1, 1));
                        }
                    }
                }
                T { channels, side, data }
            }
            Op::DepthToSpace2 { src } => {
                let a = get(&tensors, *src)?;
                let (channels, side) = (tensors[a].channels / 4, tensors[a].side);
                let out_side = 2 * side;
                let mut data = vec![0f32; channels * out_side * out_side];
                for ch in 0..channels {
                    for y in 0..side {
                        for x in 0..side {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    data[(ch * out_side + 2 * y + dy) * out_side + 2 * x + dx] =
                                        tensors[a].data[((4 * ch + 2 * dy + dx) * side + y) * side + x];
                                }
                            }
                        }
                    }
                }
                T { channels, side: out_side, data }
            }
        };
        tensors.push(out);
    }
    tensors.pop().map(|t| t.data).ok_or_else(|| NetError::Invalid("no layers".into()))
}

fn leaky(v: f32, alpha: Option<f32>) -> f32 {
    match alpha {
        Some(a) if v < 0.0 => v * a,
        _ => v,
    }
}

/// The reference as a [`TileRunner`] (for tests of the code around the model).
pub struct ReferenceRunner {
    pub net: Net,
    pub tile: usize,
}

impl TileRunner for ReferenceRunner {
    fn run(&self, input: &[f32]) -> Result<Vec<f32>, Error> {
        run(&self.net, self.tile, input).map_err(|e| Error::Runtime(e.to_string()))
    }
}

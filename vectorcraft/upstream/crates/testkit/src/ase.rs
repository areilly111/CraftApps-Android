//! Hand-written swatch exchange (`.ase`) files for swatch library tests: version 1.0, big-endian,
//! with colors in each model and color type, and color groups.

/// One block of a swatch exchange file.
#[derive(Clone, Debug)]
pub enum Block<'a> {
    /// A color group starts (its name).
    GroupStart(&'a str),
    /// The open color group ends.
    GroupEnd,
    /// A color: its name, model (`b"RGB "`, `b"CMYK"`, `b"LAB "` or `b"Gray"`), components and
    /// color type (0 global, 1 spot, 2 process).
    Color(&'a str, &'a [u8; 4], &'a [f32], u16),
}

/// A name: a `u16` count of UTF-16 code units, the terminating zero included, then the units.
fn name(out: &mut Vec<u8>, name: &str) {
    let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
    out.extend(u16::try_from(units.len()).unwrap().to_be_bytes());
    out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
}

/// A swatch exchange file of `blocks`.
pub fn ase(blocks: &[Block]) -> Vec<u8> {
    let mut out = b"ASEF\0\x01\0\0".to_vec();
    out.extend(u32::try_from(blocks.len()).unwrap().to_be_bytes());
    for b in blocks {
        let mut body = vec![];
        let kind: u16 = match b {
            Block::GroupStart(n) => {
                name(&mut body, n);
                0xC001
            }
            Block::GroupEnd => 0xC002,
            Block::Color(n, model, values, kind) => {
                name(&mut body, n);
                body.extend(*model);
                body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
                body.extend(kind.to_be_bytes());
                0x0001
            }
        };
        out.extend(kind.to_be_bytes());
        out.extend(u32::try_from(body.len()).unwrap().to_be_bytes());
        out.extend(body);
    }
    out
}

/// A library of four colors: "Sky" (global RGB) and "Ink" (spot CMYK), then the group "Neutrals"
/// of the process colors "Mist" (Gray, 25% ink) and "Clay" (Lab 50, 20, −30).
pub fn sample() -> Vec<u8> {
    ase(&[
        Block::Color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0),
        Block::Color("Ink", b"CMYK", &[1.0, 0.5, 0.0, 0.2], 1),
        Block::GroupStart("Neutrals"),
        Block::Color("Mist", b"Gray", &[0.75], 2),
        Block::Color("Clay", b"LAB ", &[0.5, 20.0, -30.0], 2),
        Block::GroupEnd,
    ])
}

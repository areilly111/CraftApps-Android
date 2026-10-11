//! Swatch exchange files (`.ase`): read and write colour swatches so they can move between
//! documents and applications. Big-endian: a signature, version 1.0, a block count, then blocks —
//! group start/end and colour entries (UTF-16 name, colour model, f32 values, colour type).

use crate::swatch::{ColorType, Swatch, SwatchValue};
use crate::{Color, cms};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AseError {
    #[error("not a swatch exchange file")]
    Signature,
    #[error("the file ends early")]
    Truncated,
}

const COLOR: u16 = 0x0001;
#[cfg(test)]
const GROUP_START: u16 = 0xC001;
#[cfg(test)]
const GROUP_END: u16 = 0xC002;

struct R<'a> {
    b: &'a [u8],
    i: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], AseError> {
        let end = self.i.checked_add(n).ok_or(AseError::Truncated)?;
        let s = self.b.get(self.i..end).ok_or(AseError::Truncated)?;
        self.i = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, AseError> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, AseError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f32(&mut self) -> Result<f32, AseError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn name(&mut self) -> Result<String, AseError> {
        let n = self.u16()? as usize;
        let mut units = Vec::with_capacity(n);
        for _ in 0..n {
            units.push(self.u16()?);
        }
        while units.last() == Some(&0) {
            units.pop();
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

/// The colour swatches in an `.ase` file (groups are flattened).
pub fn read(bytes: &[u8]) -> Result<Vec<Swatch>, AseError> {
    let mut r = R { b: bytes, i: 0 };
    if r.take(4)? != b"ASEF" {
        return Err(AseError::Signature);
    }
    r.take(4)?; // version
    let blocks = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..blocks {
        let kind = r.u16()?;
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        // Group start/end blocks are skipped: groups are flattened.
        if kind != COLOR {
            continue;
        }
        let mut b = R { b: body, i: 0 };
        let name = b.name()?;
        let model = b.take(4)?.to_vec();
        let color = match &model[..] {
            b"CMYK" => Color::Cmyk { c: b.f32()?, m: b.f32()?, y: b.f32()?, k: b.f32()? },
            b"RGB " => Color::Rgb { r: b.f32()?, g: b.f32()?, b: b.f32()? },
            b"Gray" => Color::Gray { k: 1.0 - b.f32()? },
            b"LAB " => {
                let (l, a, bb) = (b.f32()?, b.f32()?, b.f32()?);
                let [r, g, bl] = cms::lab::lab_to_srgb(cms::Lab { l: l * 100.0, a, b: bb });
                Color::Rgb { r: r.clamp(0.0, 1.0), g: g.clamp(0.0, 1.0), b: bl.clamp(0.0, 1.0) }
            }
            _ => continue,
        };
        let color_type = if b.u16().unwrap_or(2) == 1 { ColorType::Spot } else { ColorType::Process };
        out.push(Swatch { name, value: SwatchValue::Color { color, color_type }, locked: false, named: true, hidden: false });
    }
    Ok(out)
}

/// An `.ase` file of the colour swatches (special swatches, tints and gradients are left out).
pub fn write(swatches: &[Swatch]) -> Vec<u8> {
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    for s in swatches {
        let SwatchValue::Color { color, color_type } = &s.value else { continue };
        if s.locked || s.hidden {
            continue;
        }
        let mut b = Vec::new();
        let units: Vec<u16> = s.name.encode_utf16().chain(std::iter::once(0)).collect();
        b.extend((units.len() as u16).to_be_bytes());
        for u in units {
            b.extend(u.to_be_bytes());
        }
        let (model, vals): (&[u8; 4], Vec<f32>) = match *color {
            Color::Cmyk { c, m, y, k } => (b"CMYK", vec![c, m, y, k]),
            Color::Rgb { r, g, b } => (b"RGB ", vec![r, g, b]),
            Color::Gray { k } => (b"Gray", vec![1.0 - k]),
        };
        b.extend(model);
        for v in vals {
            b.extend(v.to_bits().to_be_bytes());
        }
        b.extend((if *color_type == ColorType::Spot { 1u16 } else { 2 }).to_be_bytes());
        blocks.push(b);
    }
    let mut out = Vec::new();
    out.extend(b"ASEF");
    out.extend(1u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend((blocks.len() as u32).to_be_bytes());
    for b in blocks {
        out.extend(COLOR.to_be_bytes());
        out.extend((b.len() as u32).to_be_bytes());
        out.extend(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_lab() {
        let sw = vec![
            Swatch {
                name: "Sky".into(),
                value: SwatchValue::Color { color: Color::Rgb { r: 0.2, g: 0.5, b: 1.0 }, color_type: ColorType::Process },
                locked: false,
                named: true,
                hidden: false,
            },
            Swatch {
                name: "PANTONE-ish 1".into(),
                value: SwatchValue::Color { color: Color::Cmyk { c: 0.0, m: 0.5, y: 1.0, k: 0.0 }, color_type: ColorType::Spot },
                locked: false,
                named: true,
                hidden: false,
            },
            Swatch { name: "[Paper]".into(), value: SwatchValue::Paper { color: Color::WHITE }, locked: true, named: true, hidden: false },
        ];
        let bytes = write(&sw);
        let back = read(&bytes).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "Sky");
        assert_eq!(back[0].value, sw[0].value);
        assert_eq!(back[1].value, sw[1].value);
        assert_eq!(read(b"nope"), Err(AseError::Signature));
        assert_eq!(read(&bytes[..bytes.len() - 3]), Err(AseError::Truncated));
        // A group around a Lab colour.
        let mut f = Vec::new();
        f.extend(b"ASEF\x00\x01\x00\x00");
        f.extend(3u32.to_be_bytes());
        let mut g = Vec::new();
        g.extend(2u16.to_be_bytes());
        g.extend([0, b'G', 0, 0]);
        f.extend(GROUP_START.to_be_bytes());
        f.extend((g.len() as u32).to_be_bytes());
        f.extend(&g);
        let mut c = Vec::new();
        c.extend(2u16.to_be_bytes());
        c.extend([0, b'W', 0, 0]);
        c.extend(b"LAB ");
        for v in [1.0f32, 0.0, 0.0] {
            c.extend(v.to_bits().to_be_bytes());
        }
        c.extend(2u16.to_be_bytes());
        f.extend(COLOR.to_be_bytes());
        f.extend((c.len() as u32).to_be_bytes());
        f.extend(&c);
        f.extend(GROUP_END.to_be_bytes());
        f.extend(0u32.to_be_bytes());
        let w = read(&f).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].name, "W");
        let SwatchValue::Color { color, .. } = &w[0].value else { panic!() };
        assert!(color.to_rgb().iter().all(|v| *v > 0.98), "Lab white: {color:?}");
    }
}

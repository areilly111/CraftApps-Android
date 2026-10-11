//! Lab alternate spaces for spot colours defined in Lab.
//!
//! The PDF writer gives a Separation colour space a device alternate only, so the export writes
//! Lab spot colours with their CMYK equivalent and [`lab_alternates`] then rewrites those spaces
//! to `[/Separation /Name [/Lab …] <tint function>]`, the tint function running from paper white
//! (L 100, a = b = 0) to the colour's Lab values, and moves the cross-reference offsets after the
//! rewritten bytes.

use vectorcraft_color::cms::Lab;
use vectorcraft_color::cms::lab::D50;

use crate::patch::{Patch, Xref};

/// `name` as the PDF writer writes a name object (`/` then the bytes, irregular ones as `#XX`).
fn pdf_name(name: &str) -> Vec<u8> {
    let mut out = vec![b'/'];
    for &b in name.as_bytes() {
        let regular = !b"\0\t\n\x0C\r ()<>[]{}/%".contains(&b);
        if b != b'#' && (b'!'..=b'~').contains(&b) && regular {
            out.push(b);
        } else {
            out.extend(format!("#{b:02X}").bytes());
        }
    }
    out
}

/// A number for PDF syntax: at most 4 decimals, no trailing zeros.
fn num(v: f32) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The Separation array of spot colour `name` with a Lab alternate.
fn lab_separation(name: &str, lab: Lab) -> Vec<u8> {
    let mut out = b"[/Separation".to_vec();
    out.extend(pdf_name(name));
    let [x, y, z] = D50.map(num);
    let (l, a, b) = (num(lab.l.clamp(0.0, 100.0)), num(lab.a.clamp(-128.0, 127.0)), num(lab.b.clamp(-128.0, 127.0)));
    out.extend(
        format!(
            "[/Lab<</WhitePoint[{x} {y} {z}]/Range[-128 127 -128 127]>>]<</FunctionType 2/Domain[0 1]/Range[0 100 -128 127 -128 127]/C0[100 0 0]/C1[{l} {a} {b}]/N 1>>]"
        )
        .bytes(),
    );
    out
}

pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Rewrite the DeviceCMYK Separation spaces of `spots` (colorant name, Lab values) in `pdf` (as
/// written by the export: uncompressed objects, a cross-reference table) to Lab alternates.
/// Spaces written another way are left as they are.
pub(crate) fn lab_alternates(pdf: Vec<u8>, spots: &[(String, Lab)]) -> Vec<u8> {
    let Some(xref) = Xref::read(&pdf) else { return pdf };
    let mut patch = Patch::new(&xref);
    let mut any = false;
    for (name, lab) in spots {
        let mut head = b"[/Separation".to_vec();
        head.extend(pdf_name(name));
        head.extend(b"/DeviceCMYK<<");
        let mut from = 0;
        while let Some(start) = find(&pdf, &head, from) {
            let Some(end) = find(&pdf, b">>]", start).map(|e| e + 3) else { break };
            patch.replace(start, end, lab_separation(name, *lab));
            any = true;
            from = end;
        }
    }
    if !any {
        return pdf;
    }
    match patch.apply(&pdf, &xref) {
        Ok(out) => out,
        Err(_) => pdf,
    }
}

pub(crate) fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// The offset `startxref` gives, when it points at a cross-reference table.
pub(crate) fn xref_offset(pdf: &[u8]) -> Option<usize> {
    let sx = rfind(pdf, b"startxref")?;
    let off: usize = std::str::from_utf8(&pdf[sx + 9..]).ok()?.split_whitespace().next()?.parse().ok()?;
    pdf.get(off..)?.starts_with(b"xref").then_some(off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_written_like_the_pdf_writer_writes_them() {
        assert_eq!(pdf_name("Ink"), b"/Ink");
        assert_eq!(pdf_name("PMSish 123"), b"/PMSish#20123");
        assert_eq!(pdf_name("A#(b)"), b"/A#23#28b#29");
        assert_eq!((num(0.5), num(-0.0), num(100.0), num(-12.34567)), ("0.5".into(), "0".into(), "100".into(), "-12.3457".into()));
    }
}

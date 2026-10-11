//! Hand-written PDF files for import tests: pages with given boxes, rotation, resources and
//! content, optionally encrypted with a user password (the standard security handler, revision 2:
//! 40-bit RC4), which the PDF writer can't produce.

use std::fmt::Write as _;

/// One page: its boxes as `[x0, y0, x1, y1]` in PDF user space.
#[derive(Clone, Debug, Default)]
pub struct PdfPage {
    pub media: [f64; 4],
    pub crop: Option<[f64; 4]>,
    pub bleed: Option<[f64; 4]>,
    pub trim: Option<[f64; 4]>,
    pub art: Option<[f64; 4]>,
    pub rotate: i32,
    /// The entries of the page's resource dictionary (`/ColorSpace << … >>`).
    pub resources: String,
    /// The content stream (operators, uncompressed).
    pub content: String,
    /// More entries of the page dictionary (`/PieceInfo << … >>`).
    pub entries: String,
}

impl PdfPage {
    /// A `w` × `h` pt page drawing `content`.
    pub fn new(w: f64, h: f64, content: &str) -> Self {
        Self { media: [0.0, 0.0, w, h], content: content.into(), ..Default::default() }
    }
}

/// Padding of passwords (PDF 1.7, 7.6.3.3).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80,
    0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

fn padded(pw: &str) -> Vec<u8> {
    let mut v: Vec<u8> = pw.bytes().take(32).collect();
    v.extend_from_slice(&PAD[..32 - v.len()]);
    v
}

fn hex(b: &[u8]) -> String {
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02X}");
        s
    })
}

fn rect(r: [f64; 4]) -> String {
    format!("[{} {} {} {}]", r[0], r[1], r[2], r[3])
}

/// A PDF of `pages`; with a `password` it is encrypted and opens only with it (its owner password
/// is the same followed by `-owner`).
pub fn pdf(pages: &[PdfPage], password: Option<&str>) -> Vec<u8> {
    pdf_with(pages, &[], password)
}

/// The object number of the first `extra` object of [`pdf_with`] for a file of `pages` pages.
pub fn first_extra(pages: usize) -> usize {
    3 + 2 * pages
}

/// [`pdf`] with `extra` objects (written as given, not encrypted: streams such as functions and
/// ICC profiles that resources refer to by number, from [`first_extra`] on).
pub fn pdf_with(pages: &[PdfPage], extra: &[&str], password: Option<&str>) -> Vec<u8> {
    pdf_with_catalog(pages, extra, "", password)
}

/// [`pdf_with`] with more entries in the catalog (`/OCProperties << … >>`).
pub fn pdf_with_catalog(pages: &[PdfPage], extra: &[&str], catalog: &str, password: Option<&str>) -> Vec<u8> {
    let id = md5(b"vectorcraft test file");
    // (O, U, file key) of the standard security handler, revision 2, permissions -4.
    let crypt = password.map(|pw| {
        let owner_key = md5(&padded(&format!("{pw}-owner")));
        let o = rc4(&owner_key[..5], &padded(pw));
        let mut k = padded(pw);
        k.extend_from_slice(&o);
        k.extend_from_slice(&(-4i32).to_le_bytes());
        k.extend_from_slice(&id);
        let key = md5(&k)[..5].to_vec();
        let u = rc4(&key, &PAD);
        (o, u, key)
    });
    let mut objects: Vec<Vec<u8>> = vec![];
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    objects.push(format!("<< /Type /Catalog /Pages 2 0 R {catalog}>>").into_bytes());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    for (i, p) in pages.iter().enumerate() {
        let mut d = format!("<< /Type /Page /Parent 2 0 R /MediaBox {} /Contents {} 0 R /Resources << {} >>", rect(p.media), 4 + 2 * i, p.resources);
        for (k, r) in [("CropBox", p.crop), ("BleedBox", p.bleed), ("TrimBox", p.trim), ("ArtBox", p.art)] {
            if let Some(r) = r {
                let _ = write!(d, " /{k} {}", rect(r));
            }
        }
        if p.rotate != 0 {
            let _ = write!(d, " /Rotate {}", p.rotate);
        }
        if !p.entries.is_empty() {
            let _ = write!(d, " {}", p.entries);
        }
        d.push_str(" >>");
        objects.push(d.into_bytes());
        let num = objects.len() + 1;
        let data = match &crypt {
            Some((_, _, key)) => {
                let mut k = key.clone();
                k.extend_from_slice(&(num as u32).to_le_bytes()[..3]);
                k.extend_from_slice(&[0, 0]);
                rc4(&md5(&k)[..10], p.content.as_bytes())
            }
            None => p.content.as_bytes().to_vec(),
        };
        let mut s = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
        s.extend_from_slice(&data);
        s.extend_from_slice(b"\nendstream");
        objects.push(s);
    }
    objects.extend(extra.iter().map(|o| o.as_bytes().to_vec()));
    if let Some((o, u, _)) = &crypt {
        objects.push(format!("<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P -4 >>", hex(o), hex(u)).into_bytes());
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").bytes());
    }
    let encrypt = if crypt.is_some() { format!(" /Encrypt {} 0 R", objects.len()) } else { String::new() };
    let (size, id) = (objects.len() + 1, hex(&id));
    out.extend(format!("trailer\n<< /Size {size} /Root 1 0 R /ID [<{id}> <{id}>]{encrypt} >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    out
}

/// RC4 of `data` with `key`.
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

/// MD5 of `data` (RFC 1321).
fn md5(data: &[u8]) -> [u8; 16] {
    const SHIFT: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let mut h = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(SHIFT[(i / 16) * 4 + i % 4]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 16];
    for (o, w) in out.chunks_mut(4).zip(h) {
        o.copy_from_slice(&w.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_and_rc4_match_their_reference_vectors() {
        assert_eq!(hex(&md5(b"")), "D41D8CD98F00B204E9800998ECF8427E");
        assert_eq!(hex(&md5(b"abc")), "900150983CD24FB0D6963F7D28E17F72");
        assert_eq!(hex(&md5(&[b'a'; 100])), "36A92CC94A9E0FA21F625F8BFB007ADF");
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "BBF316E8D940AF0AD3");
    }
}

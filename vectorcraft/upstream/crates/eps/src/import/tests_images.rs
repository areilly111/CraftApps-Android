//! Image data read as the PostScript Language Reference (3rd ed.) defines it: several data sources
//! read in turn (§4.10 "Images", `image` and `colorimage` in chapter 8), rows padded to a whole
//! byte at every bit depth and odd widths (§4.10), and Flate predictors undone row by row (§3.13,
//! `FlateDecode` parameters) (#498).

use super::tests::read;
use super::tests_generators::pixel;

/// Each pixel of the first image, row by row.
fn pixels(r: &crate::Imported, w: u32, h: u32) -> Vec<[u8; 4]> {
    assert!(!r.preview && r.warnings.is_empty(), "{:?}", r.warnings);
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| pixel(&r.document, x, y)).collect()
}

/// A CMYK image whose four data sources are procedures reading one file, a row of one ink each:
/// the image operator calls them in turn, so each gets its own rows. Read one after the other,
/// the cyan source took the first four rows of all four inks and the image came out in
/// horizontal bands (#498).
#[test]
fn procedures_reading_one_file_are_read_in_turn() {
    // 3 × 2 pixels; rows: C M Y K of row 0, then of row 1.
    let rows = ["ff0000", "00ff00", "000000", "000000", "000000", "000000", "ffff00", "0000ff"];
    let r = read(&format!(
        "/DeviceCMYK setcolorspace 30 20 scale /f currentfile /ASCIIHexDecode filter def \
         << /ImageType 1 /Width 3 /Height 2 /BitsPerComponent 8 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [3 0 0 -2 0 2] \
         /MultipleDataSources true /DataSource [ {{f 3 string readstring pop}} {{f 3 string readstring pop}} \
         {{f 3 string readstring pop}} {{f 3 string readstring pop}} ] >> image\n{}>",
        rows.concat()
    ));
    let cyan = [0, 174, 239, 255];
    let magenta = [236, 0, 140, 255];
    let yellow = [255, 242, 0, 255];
    let black = [35, 31, 32, 255];
    let white = [255, 255, 255, 255];
    let px = pixels(&r, 3, 2);
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 40);
    let want = [cyan, magenta, white, yellow, yellow, black];
    assert!(px.iter().zip(want).all(|(a, b)| near(*a, b)), "{px:?}");

    // The operand form: `{r} {g} {b} true 3 colorimage`, procedures reading one file.
    let r = read(
        "30 20 scale /f currentfile /ASCIIHexDecode filter def \
         3 2 8 [3 0 0 -2 0 2] {f 3 string readstring pop} {f 3 string readstring pop} {f 3 string readstring pop} true 3 colorimage\n\
         ff0000 00ff00 0000ff 102030 405060 708090>",
    );
    assert_eq!(pixels(&r, 3, 2), [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [16, 64, 112, 255], [32, 80, 128, 255], [48, 96, 144, 255]]);
}

/// Separate strings and files as the sources: each is read for its own plane, whatever the row.
#[test]
fn separate_sources_keep_their_planes() {
    let r = read("30 20 scale 3 2 8 [3 0 0 -2 0 2] <ff0000102030> <00ff00405060> <0000ff708090> true 3 colorimage");
    assert_eq!(pixels(&r, 3, 2), [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [16, 64, 112, 255], [32, 80, 128, 255], [48, 96, 144, 255]]);
}

/// Rows of an odd width at 1, 2, 4, 12 and 16 bits a sample start at a byte (their last byte
/// padded), in one source and in several.
#[test]
fn rows_of_every_depth_start_at_a_byte() {
    // 3 pixels a row, two rows: 0, max, mid / max, 0, 0 (grey), each row padded.
    let cases = [
        (1, "a0 80"),                  // 101(00000) 100(00000)
        (2, "cc c0"),                  // 11 00 11 (00), 11 00 00 (00)
        (4, "f0 80 f0 00"),            // f 0 8 (0), f 0 0 (0)
        (12, "fff0008000 fff0000000"), // fff 000 800 (0), fff 000 000 (0)
        (16, "ffff00008000 ffff00000000"),
    ];
    for (bpc, data) in cases {
        let data = data.replace(' ', "");
        let r = read(&format!("30 20 scale 3 2 {bpc} [3 0 0 -2 0 2] <{data}> image"));
        let px: Vec<u8> = pixels(&r, 3, 2).iter().map(|p| p[0]).collect();
        let want: &[u8] = match bpc {
            1 => &[255, 0, 255, 255, 0, 0],
            2 => &[255, 0, 255, 255, 0, 0],
            4 => &[255, 0, 136, 255, 0, 0],
            12 => &[255, 0, 128, 255, 0, 0],
            _ => &[255, 0, 128, 255, 0, 0],
        };
        assert_eq!(px, want, "{bpc} bits");
    }
    // Several 4-bit sources, odd width: each plane's rows padded on their own.
    let r = read("30 20 scale 3 2 4 [3 0 0 -2 0 2] <f0f000f0> <0f00f000> <00f00ff0> true 3 colorimage");
    let (on, off) = (255, 0);
    let want = [[on, off, off], [off, on, off], [on, off, on], [off, on, off], [off, off, on], [on, off, on]];
    assert_eq!(pixels(&r, 3, 2), want.map(|[r, g, b]| [r, g, b, 255]));
}

/// A PNG-predicted Flate image of an odd width: every row filter (None, Sub, Up, Average, Paeth)
/// undone on its own row.
#[test]
fn predicted_rows_of_an_odd_width() {
    // 3 RGB pixels a row, five rows; each row predicted with its own filter.
    let raw: Vec<[u8; 9]> = (0..5u8).map(|y| std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(y.wrapping_mul(53)))).collect();
    let mut encoded = vec![];
    let mut prev = [0u8; 9];
    for (kind, row) in raw.iter().enumerate() {
        encoded.push(kind as u8);
        for i in 0..9 {
            let a = if i >= 3 { row[i - 3] } else { 0 };
            let b = prev[i];
            let c = if i >= 3 { prev[i - 3] } else { 0 };
            let pred = match kind {
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            encoded.push(row[i].wrapping_sub(pred));
        }
        prev = *row;
    }
    let r = read(&format!(
        "/DeviceRGB setcolorspace 30 50 scale << /ImageType 1 /Width 3 /Height 5 /BitsPerComponent 8 /Decode [0 1 0 1 0 1] /ImageMatrix [3 0 0 -5 0 5] \
         /DataSource currentfile /ASCII85Decode filter << /Predictor 15 /Colors 3 /Columns 3 >> /FlateDecode filter >> image\n{}",
        crate::ps::ascii85(&crate::ps::deflate(&encoded))
    ));
    let want: Vec<[u8; 4]> = raw.iter().flat_map(|row| (0..3).map(move |x| [row[3 * x], row[3 * x + 1], row[3 * x + 2], 255])).collect();
    assert_eq!(pixels(&r, 3, 5), want);
}

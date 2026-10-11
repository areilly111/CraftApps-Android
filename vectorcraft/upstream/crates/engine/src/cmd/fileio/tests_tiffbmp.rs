//! TIFF, BMP and Targa export through `document.export`: the options reach the files, impossible
//! ones are refused, and the files open again.

use serde_json::{Value, json};

use super::*;

/// A 60 × 40 pt document with a black (100 % K) rectangle on a transparent corner.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 40, "height": 40})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("document.export", &p).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A TIFF's first directory: tag → (type, count, value or offset), in the file's byte order.
fn tags(f: &[u8]) -> std::collections::HashMap<u16, (u16, u32, u32)> {
    let big = &f[..2] == b"MM";
    let u16_at = |at: usize| {
        let b = [f[at], f[at + 1]];
        if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let u32_at = |at: usize| {
        let b = f[at..at + 4].try_into().unwrap();
        if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    };
    let dir = u32_at(4) as usize;
    (0..u16_at(dir) as usize)
        .map(|i| {
            let e = dir + 2 + i * 12;
            let (ty, count) = (u16_at(e + 2), u32_at(e + 4));
            // A lone SHORT sits in the first half of the value field.
            let value = if ty == 3 && count == 1 { u32::from(u16_at(e + 8)) } else { u32_at(e + 8) };
            (u16_at(e), (ty, count, value))
        })
        .collect()
}

#[test]
fn tiff_options_reach_the_file() {
    let mut s = session();
    let tiff = |s: &mut Session, p: Value| export(s, merge(json!({"format": "tiff"}), p));
    let rgb = tiff(&mut s, json!({}));
    assert_eq!(&rgb[..4], b"II*\0", "little-endian by default");
    let t = tags(&rgb);
    assert_eq!((t[&259].2, t[&262].2, t[&277].2), (5, 2, 4), "LZW, RGB, an alpha channel for the transparent corner");
    assert!(t.contains_key(&34675), "the sRGB profile");
    let img = image::load_from_memory_with_format(&rgb, image::ImageFormat::Tiff).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (60, 40));
    assert_eq!(img.get_pixel(5, 5).0[3], 0, "transparent where nothing is drawn");
    let black = img.get_pixel(40, 20).0;
    assert!(black[3] == 255 && black[0] < 80, "{black:?}");

    let mm = tiff(&mut s, json!({"byteOrder": "big", "lzw": false, "embedIcc": false, "background": "white", "ppi": 300}));
    assert_eq!(&mm[..4], b"MM\0*");
    let t = tags(&mm);
    assert_eq!((t[&259].2, t[&277].2, t[&296].2), (1, 3, 2), "uncompressed, opaque RGB, inches");
    assert!(!t.contains_key(&34675));
    let x_res = t[&282].2 as usize;
    assert_eq!(&mm[x_res..x_res + 8], [300u32.to_be_bytes(), 1u32.to_be_bytes()].concat(), "300/1 ppi");
    assert!(tiff(&mut s, json!({"background": "white"})).len() < mm.len() / 3 * 4, "LZW is smaller");

    // CMYK: four samples of ink; the black rectangle is black ink alone, the rest no ink.
    let cmyk = tiff(&mut s, json!({"colorModel": "cmyk", "lzw": false}));
    let t = tags(&cmyk);
    assert_eq!((t[&262].2, t[&277].2, t[&332].2), (5, 4, 1));
    let strip = t[&273];
    let first = if strip.1 == 1 { strip.2 } else { u32::from_le_bytes(cmyk[strip.2 as usize..][..4].try_into().unwrap()) } as usize;
    let px = |x: usize, y: usize| &cmyk[first + (y * 60 + x) * 4..][..4];
    assert_eq!((px(40, 0), px(5, 0)), (&[0, 0, 0, 255][..], &[0, 0, 0, 0][..]));

    let gray = tiff(&mut s, json!({"colorModel": "gray"}));
    assert_eq!((tags(&gray)[&262].2, tags(&gray)[&277].2), (1, 1), "grey, no alpha");
    assert!(s.execute("document.export", &json!({"format": "tiff", "byteOrder": "middle"})).is_err());
    // The file opens again.
    let r = s.execute("document.open", &json!({"name": "back.tif", "dataBase64": vectorcraft_format::base64_encode(&mm)})).unwrap();
    assert_eq!(r["format"], "tiff");
    for file in [&cmyk, &gray, &rgb] {
        s.execute("document.open", &json!({"name": "back.tif", "dataBase64": vectorcraft_format::base64_encode(file)})).unwrap();
    }
}

#[test]
fn bmp_options_reach_the_file() {
    let mut s = session();
    let bmp = |s: &mut Session, p: Value| export(s, merge(json!({"format": "bmp"}), p));
    let bits = |f: &[u8]| u16::from_le_bytes([f[28], f[29]]);
    let plain = bmp(&mut s, json!({}));
    assert_eq!((&plain[..2], bits(&plain)), (&b"BM"[..], 24));
    for depth in [1, 4, 8, 16, 32] {
        assert_eq!(bits(&bmp(&mut s, json!({"depth": depth}))), depth);
    }
    let rle = bmp(&mut s, json!({"depth": 8, "rle": true}));
    assert_eq!(u32::from_le_bytes(rle[30..34].try_into().unwrap()), 1, "RLE8");
    let flipped = bmp(&mut s, json!({"flipRows": true}));
    assert_eq!(i32::from_le_bytes(flipped[22..26].try_into().unwrap()), -40, "top-down rows");
    let os2 = bmp(&mut s, json!({"fileFormat": "os2", "depth": 4}));
    assert_eq!(u32::from_le_bytes(os2[14..18].try_into().unwrap()), 12, "OS/2 core header");
    let gray = image::load_from_memory_with_format(&bmp(&mut s, json!({"colorModel": "gray", "depth": 8})), image::ImageFormat::Bmp).unwrap();
    assert!(gray.to_rgb8().pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
    for bad in [
        json!({"depth": 2}),
        json!({"depth": 24, "rle": true}),
        json!({"depth": 8, "rle": true, "flipRows": true}),
        json!({"fileFormat": "os2", "depth": 32}),
        json!({"fileFormat": "amiga"}),
        json!({"colorModel": "cmyk"}),
    ] {
        let e = s.execute("document.export", &merge(json!({"format": "bmp"}), bad.clone())).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{bad}: {e}");
    }
    let r = s.execute("document.open", &json!({"name": "back.bmp", "dataBase64": vectorcraft_format::base64_encode(&rle)})).unwrap();
    assert_eq!(r["format"], "bmp");
}

#[test]
fn targa_depth_reaches_the_file() {
    let mut s = session();
    for (depth, alpha) in [(16, 1), (24, 0), (32, 8)] {
        let tga = export(&mut s, json!({"format": "tga", "depth": depth}));
        assert_eq!((tga[16], tga[17]), (depth, alpha));
        assert_eq!(u16::from_le_bytes([tga[12], tga[13]]), 60);
    }
    let tga = export(&mut s, json!({"format": "tga"}));
    assert_eq!(tga[16], 24, "24 bits by default");
    for bad in [json!({"depth": 8}), json!({"colorModel": "gray"})] {
        assert!(s.execute("document.export", &merge(json!({"format": "tga"}), bad.clone())).is_err(), "{bad}");
    }
}

#[test]
fn the_formats_are_listed_and_write_one_file_per_artboard() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 30, "height": 20, "artboards": 2})).unwrap();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let find = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap().clone();
    let (tiff, bmp, tga) = (find("tiff"), find("bmp"), find("tga"));
    assert_eq!((tiff["write"].as_bool(), tiff["read"].as_bool()), (Some(true), Some(true)));
    assert_eq!((bmp["write"].as_bool(), tga["write"].as_bool(), tga["read"].as_bool()), (Some(true), Some(true), Some(false)));
    for (f, keys) in [
        (&tiff, &["colorModel", "lzw", "byteOrder", "embedIcc", "ppi", "antiAlias"][..]),
        (&bmp, &["colorModel", "depth", "fileFormat", "rle", "flipRows", "reduction", "dither"]),
        (&tga, &["depth", "ppi", "antiAlias", "background"]),
    ] {
        for k in keys {
            assert!(f["options"].get(*k).is_some(), "{} takes {k}", f["id"]);
        }
    }
    assert_eq!(format_for_name("a.tga").map(|f| f.id), Some("tga"));
    let r = s.execute("document.export", &json!({"format": "tiff", "useArtboards": true})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(files, ["Untitled-1-Artboard-1.tif", "Untitled-1-Artboard-2.tif"]);
}

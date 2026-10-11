//! Import options: the pages picked, the box each artboard gets (Crop To), passwords and `info`.

use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::pdf::{PdfPage, pdf};

use crate::*;

/// A 600 × 800 page with every box set and a 200 × 100 rectangle at (120, 150) (PDF space).
fn boxed_page() -> PdfPage {
    PdfPage {
        crop: Some([50.0, 50.0, 550.0, 750.0]),
        bleed: Some([40.0, 40.0, 560.0, 760.0]),
        trim: Some([60.0, 60.0, 540.0, 740.0]),
        art: Some([100.0, 100.0, 300.0, 400.0]),
        ..PdfPage::new(600.0, 800.0, "0 0 1 rg 120 150 200 100 re f")
    }
}

fn open(bytes: &[u8], opts: ImportOptions) -> Document {
    import_with_report(bytes, &opts).unwrap().document
}

fn art(d: &Document) -> Rect {
    d.art_bounds().unwrap()
}

fn close(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 1e-6)
}

#[test]
fn each_crop_box_sizes_the_artboard_and_places_the_art_in_it() {
    let bytes = pdf(&[boxed_page()], None);
    // (box, artboard size, the rectangle's top-left on the artboard).
    let cases = [
        (CropTo::Media, (600.0, 800.0), (120.0, 550.0)),
        (CropTo::Crop, (500.0, 700.0), (70.0, 500.0)),
        (CropTo::Bleed, (520.0, 720.0), (80.0, 510.0)),
        (CropTo::Trim, (480.0, 680.0), (60.0, 490.0)),
        (CropTo::Art, (200.0, 300.0), (20.0, 150.0)),
        (CropTo::Bounding, (200.0, 100.0), (0.0, 0.0)),
    ];
    for (crop, (w, h), (x, y)) in cases {
        let d = open(&bytes, ImportOptions { crop, ..Default::default() });
        let ab = d.artboards[0].rect;
        assert!((ab.width() - w).abs() < 1e-6 && (ab.height() - h).abs() < 1e-6, "{crop:?}: {ab:?}");
        let r = art(&d);
        assert!(close(r, Rect::new(ab.x0 + x, ab.y0 + y, ab.x0 + x + 200.0, ab.y0 + y + 100.0)), "{crop:?}: art {r:?} on {ab:?}");
    }
    // The default is the crop box, as before.
    assert_eq!(ImportOptions::default().crop, CropTo::Crop);
}

#[test]
fn missing_boxes_fall_back_to_the_crop_box_and_boxes_are_clipped_to_the_media_box() {
    let page = PdfPage { crop: Some([10.0, 10.0, 290.0, 390.0]), art: Some([-50.0, 0.0, 100.0, 100.0]), ..PdfPage::new(300.0, 400.0, "") };
    let bytes = pdf(&[page], None);
    for crop in [CropTo::Bleed, CropTo::Trim] {
        let ab = open(&bytes, ImportOptions { crop, ..Default::default() }).artboards[0].rect;
        assert_eq!((ab.width(), ab.height()), (280.0, 380.0), "{crop:?}");
    }
    let ab = open(&bytes, ImportOptions { crop: CropTo::Art, ..Default::default() }).artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (100.0, 100.0));
    // A page without art keeps its crop box for Bounding.
    let ab = open(&bytes, ImportOptions { crop: CropTo::Bounding, ..Default::default() }).artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (280.0, 380.0));
}

#[test]
fn a_rotated_page_rotates_its_boxes() {
    let page = PdfPage { rotate: 90, ..boxed_page() };
    let d = open(&pdf(&[page], None), ImportOptions { crop: CropTo::Art, ..Default::default() });
    let ab = d.artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (300.0, 200.0));
    // The rectangle turns with the page (clockwise): PDF (x, y) shows at (y − 100, x − 100) on
    // the art box's artboard.
    assert!(close(art(&d), Rect::new(50.0, 20.0, 150.0, 220.0)), "{:?}", art(&d));
}

#[test]
fn pages_pick_which_pages_import_and_in_what_order() {
    let pages: Vec<PdfPage> = (1..=4).map(|i| PdfPage::new(100.0 * i as f64, 100.0, "0 g 0 0 10 10 re f")).collect();
    let bytes = pdf(&pages, None);
    let d = open(&bytes, ImportOptions { pages: Some(vec![1, 2]), ..Default::default() });
    let widths: Vec<f64> = d.artboards.iter().map(|a| a.rect.width()).collect();
    assert_eq!(widths, [200.0, 300.0]);
    let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_deref().unwrap_or_default()).collect();
    assert_eq!(names, ["Page 2", "Page 3"]);
    assert_eq!(d.artboards.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Artboard 1", "Artboard 2"]);
    // Artboards sit side by side with the gap between them.
    assert_eq!(d.artboards[1].rect.x0, d.artboards[0].rect.x1 + 36.0);
    let d = open(&bytes, ImportOptions { pages: Some(vec![3, 0]), max_pages: Some(1), ..Default::default() });
    assert_eq!(d.artboards.len(), 1);
    assert_eq!(d.artboards[0].rect.width(), 400.0);
    assert_eq!(import_with_report(&bytes, &ImportOptions { pages: Some(vec![4]), ..Default::default() }).unwrap_err(), PdfError::BadPage(5, 4));
    assert_eq!(import_with_report(&bytes, &ImportOptions { pages: Some(vec![]), ..Default::default() }).unwrap_err(), PdfError::NoPages);
}

#[test]
fn bounding_boxes_of_several_pages_do_not_overlap() {
    let page = |x: f64| PdfPage::new(300.0, 300.0, &format!("0 g {x} 10 50 50 re f"));
    let d = open(&pdf(&[page(240.0), page(0.0)], None), ImportOptions { crop: CropTo::Bounding, ..Default::default() });
    let (a, b) = (d.artboards[0].rect, d.artboards[1].rect);
    assert_eq!((a.width(), b.width()), (50.0, 50.0));
    assert!(b.x0 >= a.x1 + 36.0, "{a:?} {b:?}");
}

#[test]
fn art_of_neighbouring_artboards_off_a_page_is_left_to_its_own_page() {
    // Page 2 also draws page 1's square, off to its left (outside its box), as files with several
    // artboards are written: only page 1 keeps it, so it doesn't come back twice.
    let pages = [PdfPage::new(100.0, 100.0, "0 g 10 10 20 20 re f"), PdfPage::new(100.0, 100.0, "0 g -90 10 20 20 re f 1 0 0 rg 50 10 20 20 re f")];
    let d = open(&pdf(&pages, None), ImportOptions::default());
    let count = |i: usize| d.layers[i].children().map_or(0, |c| c.len());
    assert_eq!((count(0), count(1)), (1, 1));
    // A single page keeps its art off the page (the pasteboard).
    let d = open(&pdf(&pages[1..], None), ImportOptions::default());
    assert_eq!(d.layers[0].children().map_or(0, |c| c.len()), 2);
}

#[test]
fn an_encrypted_pdf_opens_with_its_password_only() {
    let bytes = pdf(&[PdfPage::new(200.0, 100.0, "1 0 0 rg 10 10 50 50 re f")], Some("secret"));
    assert_eq!(import_with_report(&bytes, &ImportOptions::default()).unwrap_err(), PdfError::NeedsPassword);
    let wrong = ImportOptions { password: Some("guess".into()), ..Default::default() };
    assert_eq!(import_with_report(&bytes, &wrong).unwrap_err(), PdfError::WrongPassword);
    assert_eq!(info(&bytes, None).unwrap_err(), PdfError::NeedsPassword);
    let d = open(&bytes, ImportOptions { password: Some("secret".into()), ..Default::default() });
    let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!() };
    assert_eq!(children.len(), 1, "the decrypted page has its rectangle");
    assert!(close(art(&d), Rect::new(10.0, 40.0, 60.0, 90.0)), "{:?}", art(&d));
    // A file that isn't encrypted ignores a password.
    let plain = pdf(&[PdfPage::new(200.0, 100.0, "")], None);
    assert!(import_with_report(&plain, &ImportOptions { password: Some("x".into()), ..Default::default() }).is_ok());
}

#[test]
fn info_lists_pages_with_their_size_rotation_and_boxes() {
    let bytes = pdf(&[boxed_page(), PdfPage { rotate: 270, ..PdfPage::new(100.0, 50.0, "") }], None);
    let i = info(&bytes, None).unwrap();
    assert_eq!(i.pages.len(), 2);
    let p = &i.pages[0];
    assert_eq!((p.width, p.height, p.rotation), (500.0, 700.0, 0));
    let b = |c: CropTo| p.boxes.iter().find(|(k, _)| *k == c).map(|(_, r)| *r).unwrap();
    assert_eq!(b(CropTo::Media), Rect::new(0.0, 0.0, 600.0, 800.0));
    assert_eq!(b(CropTo::Trim), Rect::new(60.0, 60.0, 540.0, 740.0));
    assert!(p.boxes.iter().all(|(k, _)| *k != CropTo::Bounding));
    assert_eq!((i.pages[1].width, i.pages[1].height, i.pages[1].rotation), (50.0, 100.0, 270));
    assert!(matches!(info(b"not a pdf", None), Err(PdfError::Parse(_))));
}

#[test]
fn a_page_without_area_gets_the_size_viewers_give_it() {
    let bytes = pdf(&[PdfPage { media: [0.0, 0.0, 0.0, 0.0], ..PdfPage::new(0.0, 0.0, "") }], None);
    let i = info(&bytes, None).unwrap();
    let (w, h) = (i.pages[0].width, i.pages[0].height);
    assert!(w > 0.0 && h > 0.0);
    for crop in [CropTo::Crop, CropTo::Media, CropTo::Art] {
        let ab = open(&bytes, ImportOptions { crop, ..Default::default() }).artboards[0].rect;
        assert_eq!((ab.width(), ab.height()), (w, h), "{crop:?}");
    }
}

#[test]
fn postscript_is_refused_as_such() {
    let ps = b"%!PS\n0 0 moveto 10 10 lineto stroke\nshowpage\n";
    assert_eq!(import(ps).unwrap_err(), PdfError::PostScript);
    assert_eq!(info(ps, None).unwrap_err(), PdfError::PostScript);
    assert!(is_postscript(&[0xC5, 0xD0, 0xD3, 0xC6, 0, 0]) && !is_postscript(b"%PDF-1.7"));
}

//! Type redraws in a font that becomes available while it is on screen (fonts added, Refresh Font
//! List).

use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Point;

use super::*;

/// The bundled Source Serif 4 renamed `family` (as long as the original name).
fn serif_as(family: &str) -> Vec<u8> {
    let mut data = include_bytes!("../../../assets/fonts/SourceSerif4-Regular.ttf").to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Serif 4"), utf16(family)), (b"Source Serif 4".to_vec(), family.as_bytes().to_vec())] {
        assert_eq!(from.len(), to.len());
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

#[test]
fn type_redraws_when_its_font_becomes_available() {
    const FAMILY: &str = "Arrival Serif4";
    let mut d = Document::new(300.0, 150.0);
    let style = CharStyle { size: 60.0, font_family: FAMILY.into(), ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 100.0), "Hamburg", style))));
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_ne!(before, after, "drawn in the font that arrived");
    assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

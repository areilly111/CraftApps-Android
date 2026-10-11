//! Object › Object Layer Options: show or hide the layers (optional content) of a placed PDF.

use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_selection, targets};
use crate::Session;

fn placed_pdf(s: &Session, id: designcraft_doc::ItemId) -> Option<std::sync::Arc<Vec<u8>>> {
    let d = &s.active()?.doc;
    let g = d.item(id)?.graphic()?;
    let a = d.assets.get(&g.asset)?;
    designcraft_render::is_pdf(&a.data).then(|| a.data.clone())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "object.pdfLayers", "Object Layers", [], None, "{id?} → [{name, visible, default}] — the layers of the selected placed PDF", has_selection, |s, p| {
            let id = super::id_param(p, "id").or_else(|| s.active().and_then(|d| d.selection.items.first().copied())).ok_or_else(|| bad("object.pdfLayers", "no selection"))?;
            let data = placed_pdf(s, id).ok_or_else(|| bad("object.pdfLayers", "not a placed PDF"))?;
            let hidden = s.doc()?.doc.item(id).map(|i| i.pdf_hidden_layers.clone()).unwrap_or_default();
            Ok(Value::Array(
                designcraft_render::pdf_layers(&data)
                    .into_iter()
                    .map(|(name, on)| json!({"visible": if hidden.is_empty() { on } else { !hidden.contains(&name) }, "name": name, "default": on}))
                    .collect(),
            ))
        }),
        cmd!(
            "object.layerOptions",
            "Object Layer Options…",
            ["Object"],
            None,
            "{ids?, hidden: [layer name]} — layers of placed PDFs to hide ([] = the file's own settings)",
            has_selection,
            |s, p| {
                let ids = targets(s, p)?;
                let hidden: Vec<String> = p
                    .get("hidden")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                if ids.iter().all(|id| placed_pdf(s, *id).is_none()) {
                    return Err(bad("object.layerOptions", "select a placed PDF"));
                }
                s.edit(|d, _| {
                    for id in &ids {
                        if let Some(it) = d.item_mut(*id) {
                            it.pdf_hidden_layers = hidden.clone();
                        }
                    }
                    Ok(json!({"hidden": hidden}))
                })
            }
        ),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use crate::Session;

    /// A one-page PDF (100×100 pt) with two layers: "Blue" (left half) and "Red" (right half).
    pub(crate) fn two_layer_pdf() -> Vec<u8> {
        let content = "/OC /a BDC 0 0 1 rg 0 0 50 100 re f EMC /OC /b BDC 1 0 0 rg 50 0 50 100 re f EMC";
        let objs = [
            "<</Type/Catalog/Pages 2 0 R/OCProperties<</OCGs[5 0 R 6 0 R]/D<</Order[5 0 R 6 0 R]>>>>>>".to_string(),
            "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Resources<</Properties<</a 5 0 R/b 6 0 R>>>>/Contents 4 0 R>>".to_string(),
            format!("<</Length {}>>\nstream\n{content}\nendstream", content.len()),
            "<</Type/OCG/Name(Blue)>>".to_string(),
            "<</Type/OCG/Name(Red)>>".to_string(),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let x = out.len();
        let mut t = format!("xref\n0 {}\n0000000000 65535 f\r\n", objs.len() + 1);
        for o in offs {
            t.push_str(&format!("{o:010} 00000 n\r\n"));
        }
        t.push_str(&format!("trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1));
        out.extend_from_slice(t.as_bytes());
        out
    }

    #[test]
    fn placed_pdf_layers_hide() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let pdf = two_layer_pdf();
        let r = s
            .execute("file.place", &json!({"base64": super::super::file::base64_encode(&pdf), "name": "art.pdf", "x": 100, "y": 100, "width": 100}))
            .unwrap();
        let id = r["id"].clone();
        let l = s.execute("object.pdfLayers", &json!({"id": id})).unwrap();
        assert_eq!(l.as_array().unwrap().len(), 2);
        assert_eq!(l[0]["name"], "Blue");
        let px = |s: &Session, x: u32| {
            let d = s.doc().unwrap().doc.clone();
            let mut rr = designcraft_render::Renderer::new();
            rr.threads = 0;
            rr.render_page(&d, &s.cache, 0, 1.0, false, &Default::default()).unwrap().pixel(x, 150)
        };
        assert!(px(&s, 125)[2] > 200 && px(&s, 175)[0] > 200, "both layers show");
        s.execute("object.layerOptions", &json!({"ids": [id], "hidden": ["Red"]})).unwrap();
        let right = px(&s, 175);
        assert!(right[0] > 200 && right[1] > 200 && right[2] > 200, "the red layer is hidden: {right:?}");
        assert!(px(&s, 125)[2] > 200, "blue still shows");
        assert_eq!(s.execute("object.pdfLayers", &json!({"id": id})).unwrap()[1]["visible"], false);
        // PDF export carries the choice (the graphic goes out as an image).
        let r = s.execute("file.exportPdf", &json!({})).unwrap();
        assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("layer")), "{r}");
    }
}

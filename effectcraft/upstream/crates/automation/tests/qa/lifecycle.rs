//! Scenario 6 — project lifecycle: save and reopen, Save a Copy As XML, Collect Files, Reduce
//! Project, undo/redo across 50 operations, a jump across history branches, render templates
//! and a watch folder.

use serde_json::json;

use crate::harness::{Qa, mean_diff};

#[test]
fn project_lifecycle() {
    let mut qa = Qa::new("lifecycle");
    // Footage to collect later.
    std::fs::create_dir_all(qa.dir.join("media")).unwrap();
    image::RgbaImage::from_pixel(32, 32, image::Rgba([10, 200, 90, 255])).save(qa.dir.join("media/logo.png")).unwrap();
    let logo = qa.exec("file.import", json!({"paths": [qa.path("media/logo.png")]}))["items"][0].clone();

    let main = qa.exec("comp.new", json!({"name": "Main", "width": 96, "height": 54, "frameRate": 12, "duration": 1}))["comp"].clone();
    qa.exec("layer.addItem", json!({"item": logo.clone()}));
    let unused = qa.exec("comp.new", json!({"name": "Unused", "width": 64, "height": 64}))["comp"].clone();
    qa.exec("layer.newSolid", json!({"name": "Junk", "color": "#ff0000"}));
    qa.exec("comp.open", json!({"comp": main.clone()}));

    // 50 edits, one undo step each. Levels of Undo defaults to 32 (as in After Effects).
    assert_eq!(qa.exec("prefs.get", json!({"key": "general.undoLevels"})), json!(32));
    qa.exec("prefs.set", json!({"key": "general.undoLevels", "value": 99}));
    let box_id = qa.exec("layer.newSolid", json!({"name": "Box", "color": "#2040ff", "width": 20, "height": 20}))["layer"].clone();
    let steps_before = qa.tool("get_project", json!({}))["undo"].as_array().unwrap().len();
    for i in 0..50 {
        qa.tool("set_property", json!({"layer": box_id.clone(), "path": "transform/position", "value": [10 + i, 27]}));
    }
    let s = qa.tool("get_project", json!({}));
    let n = s["undo"].as_array().unwrap().len();
    assert_eq!(n, (steps_before + 50).min(99), "50 more undo steps recorded");
    let end = qa.frame(None, 0.0);
    let u = qa.tool("undo", json!({"steps": 50}));
    assert_eq!(u["steps"], 50);
    let pos = qa.tool("get_property", json!({"layer": box_id.clone(), "path": "transform/position"}))["value"].clone();
    assert_eq!(pos, json!([48.0, 27.0, 0.0]), "back to the solid's default position");
    let r = qa.tool("redo", json!({"steps": 50}));
    assert_eq!(r["steps"], 50);
    assert!(mean_diff(&end, &qa.frame(None, 0.0)) < 1e-9, "redo restores the same frame");

    // Branching history: undo 5, make a different edit, then jump back to the old branch's tip.
    qa.tool("undo", json!({"steps": 5}));
    qa.tool("set_property", json!({"layer": box_id.clone(), "path": "transform/opacity", "value": 25}));
    let h = qa.tool("history", json!({}));
    let nodes = h.as_array().unwrap_or_else(|| h["states"].as_array().expect("history list"));
    let branch_tip = nodes.iter().filter(|n| n["depth"].as_u64().unwrap_or(0) > 0).max_by_key(|n| n["index"].as_u64()).expect("a branch").clone();
    qa.tool("history", json!({"goto": branch_tip["id"].clone()}));
    let pos = qa.tool("get_property", json!({"layer": box_id.clone(), "path": "transform/position"}))["value"].clone();
    assert_eq!(pos, json!([59.0, 27.0, 0.0]), "the old branch's last edit is back");
    let op = qa.tool("get_property", json!({"layer": box_id.clone(), "path": "transform/opacity"}))["value"].as_f64().unwrap();
    assert_eq!(op, 100.0, "and the other branch's edit is not on this line");

    // Save, reopen, compare.
    let proj = qa.path("show.ecproj");
    let saved = qa.tool("save_project", json!({"path": proj}));
    assert_eq!(saved["dirty"], false);
    let before = qa.frame(Some(main.clone()), 0.0);
    qa.tool("open_project", json!({"new": true}));
    let reopened = qa.tool("open_project", json!({"path": proj}));
    assert_eq!(reopened["path"], json!(proj));
    let main2 = reopened["items"].as_array().unwrap().iter().find(|i| i["name"] == "Main").unwrap()["id"].clone();
    assert!(mean_diff(&before, &qa.frame(Some(main2.clone()), 0.0)) < 1e-9, "the reopened project renders the same");

    // Save a Copy As XML: reopens to the same frame and leaves the session's path alone.
    let xml = qa.path("show.ecprojx");
    qa.exec("file.saveCopyAsXml", json!({"path": xml}));
    assert!(std::fs::read_to_string(&xml).unwrap().trim_start().starts_with('<'));
    assert_eq!(qa.tool("get_project", json!({}))["path"], json!(proj));
    qa.tool("open_project", json!({"path": xml}));
    assert!(mean_diff(&before, &qa.frame(Some(main2.clone()), 0.0)) < 1e-9, "the XML copy renders the same");
    qa.tool("open_project", json!({"path": proj}));

    // Collect Files: the project and its footage in one folder that opens on its own.
    let collected = qa.path("collected");
    let c = qa.exec("file.collectFiles", json!({"folder": collected}));
    assert_eq!((c["copied"].clone(), c["errors"].clone()), (json!(1), json!([])), "Collect Files makes its folder: {c}");
    let files: Vec<String> = walk(&qa.dir.join("collected"));
    assert!(files.iter().any(|f| f.ends_with(".ecproj")) && files.iter().any(|f| f.ends_with("logo.png")), "{files:?}");
    std::fs::remove_file(qa.dir.join("media/logo.png")).unwrap();
    let cproj = files.iter().find(|f| f.ends_with(".ecproj")).unwrap().clone();
    let reo = qa.tool("open_project", json!({"path": cproj}));
    let cm = reo["items"].as_array().unwrap().iter().find(|i| i["name"] == "Main").unwrap()["id"].clone();
    assert!(mean_diff(&before, &qa.frame(Some(cm), 0.0)) < 1e-9, "the collected project finds its footage");

    // Reduce Project to Main: Unused goes, the footage Main uses stays.
    qa.exec("project.select", json!({"items": [main2.clone()]}));
    qa.exec("file.reduceProject", json!({}));
    let names: Vec<String> = qa.tool("get_project", json!({}))["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"Main".to_string()) && names.contains(&"logo.png".to_string()) && !names.contains(&"Unused".to_string()), "{names:?}");
    let _ = unused;

    // Render templates: save an output-module template, queue with it, render.
    qa.exec("renderQueue.saveTemplate", json!({"kind": "outputModule", "name": "QA PNG", "params": {"format": "png", "channels": "rgba"}}));
    let tpl = qa.exec("renderQueue.templates", json!({}));
    assert!(tpl.to_string().contains("QA PNG"), "{tpl}");
    qa.exec("renderQueue.add", json!({"comp": "Main", "output": qa.path("out/main_[##].png")}));
    qa.exec("renderQueue.applyTemplate", json!({"index": 1, "outputModule": "QA PNG"}));
    let list = qa.exec("renderQueue.list", json!({}));
    assert_eq!(list["items"][0]["output"]["format"], "PngSequence", "{list}");
    assert_eq!(list["items"][0]["output"]["channels"], "Rgba");
    let done = qa.exec("renderQueue.render", json!({"wait": true}));
    assert_eq!(done["items"][0]["status"], "Done", "{done}");
    assert_eq!(walk(&qa.dir.join("out")).iter().filter(|f| f.ends_with(".png")).count(), 12);

    // Watch folder: a project with a queued render dropped into the folder is rendered.
    let watch = qa.dir.join("watch");
    std::fs::create_dir_all(&watch).unwrap();
    qa.exec("renderQueue.setRender", json!({"index": 1, "render": true}));
    qa.exec("renderQueue.setOutput", json!({"index": 1, "path": watch.join("w_[##].png").to_string_lossy()}));
    qa.tool("save_project", json!({"path": watch.join("job.ecproj").to_string_lossy()}));
    // Starting the watch renders what is already there; a poll then finds nothing new.
    let started = qa.exec("file.watchFolder", json!({"folder": watch.to_string_lossy()}));
    assert_eq!(started["rendered"][0]["state"], "done", "{started}");
    let polled = qa.exec("file.watchFolder.poll", json!({}));
    assert_eq!(polled["rendered"], json!([]), "{polled}");
    assert!(watch.join("job.ecproj.status.json").exists() || walk(&watch).iter().any(|f| f.ends_with(".status.json")));
    assert_eq!(walk(&watch).iter().filter(|f| f.ends_with(".png")).count(), 12, "{:?}", walk(&watch));
    qa.exec("file.watchFolder", json!({"folder": watch.to_string_lossy(), "stop": true}));
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    out
}

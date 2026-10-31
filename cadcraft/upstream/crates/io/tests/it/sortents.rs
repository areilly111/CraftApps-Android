//! Draw order from DXF files (#439): a SORTENTSTABLE reorders its space by sort handle, and the
//! order CADCraft writes (the order of the entity records) reads back unchanged.

use cadcraft_doc::*;

fn line(h: &str, y: f64) -> String {
    format!("0\nLINE\n5\n{h}\n330\n1F\n8\n0\n10\n0\n20\n{y}\n30\n0\n11\n10\n21\n{y}\n31\n0\n")
}

fn dxf(objects: &str) -> String {
    let mut s = String::from("0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nBLOCK_RECORD\n0\nBLOCK_RECORD\n5\n1F\n2\n*Model_Space\n0\nENDTAB\n0\nENDSEC\n");
    s.push_str("0\nSECTION\n2\nENTITIES\n");
    s.push_str(&line("A1", 0.0));
    s.push_str(&line("A2", 1.0));
    s.push_str(&line("A3", 2.0));
    s.push_str("0\nENDSEC\n0\nSECTION\n2\nOBJECTS\n");
    s.push_str(objects);
    s.push_str("0\nENDSEC\n0\nEOF\n");
    s
}

fn order(d: &Drawing) -> Vec<String> {
    d.model.iter().map(|e| e.handle.hex()).collect()
}

#[test]
fn sortentstable_sets_the_draw_order() {
    // A1 is drawn by sort handle A4 (over A2 and A3); A3 by 10 (under everything).
    let table = "0\nSORTENTSTABLE\n5\nB0\n330\nB1\n100\nAcDbSortentsTable\n330\n1F\n331\nA1\n5\nA4\n331\nA3\n5\n10\n";
    let d = cadcraft_io::read_dxf(dxf(table).as_bytes()).unwrap();
    assert_eq!(order(&d), ["A3", "A2", "A1"]);
    // Written back in draw order, without a table, the order survives.
    let back = cadcraft_io::read_dxf(cadcraft_io::write_dxf(&d).as_bytes()).unwrap();
    assert_eq!(order(&back), ["A3", "A2", "A1"]);
}

#[test]
fn broken_sortentstables_are_ignored() {
    for table in [
        "0\nSORTENTSTABLE\n5\nB0\n331\nA1\n5\nA4\n",
        "0\nSORTENTSTABLE\n100\nAcDbSortentsTable\n330\nFFFF\n331\nA1\n5\nA4\n",
        "0\nSORTENTSTABLE\n100\nAcDbSortentsTable\n330\n1F\n331\nzz\n5\nA4\n331\nA2\n5\n\n5\nA9\n",
        "0\nSORTENTSTABLE\n100\nAcDbSortentsTable\n",
    ] {
        let d = cadcraft_io::read_dxf(dxf(table).as_bytes()).unwrap();
        assert_eq!(order(&d), ["A1", "A2", "A3"], "{table:?}");
    }
}

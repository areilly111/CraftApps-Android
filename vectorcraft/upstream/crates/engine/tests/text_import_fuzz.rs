//! Untrusted text files placed with File → Place never crash: any bytes, with any Text Import
//! Options, place as area type or fail with an error.
//!
//! `PROPTEST_CASES=20000 cargo test -p vectorcraft-engine --test text_import_fuzz` runs a deeper search.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::json;
use vectorcraft_testkit::catch_quiet;

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn any_bytes_place_as_text_without_panics(
        bytes in prop::collection::vec(prop_oneof![any::<u8>(), Just(b'\r'), Just(b'\n'), Just(b' ')], 0..400),
        bom in prop::sample::select(vec![&b""[..], b"\xEF\xBB\xBF", b"\xFF\xFE", b"\xFE\xFF"]),
        ansi in any::<bool>(),
        mac in any::<bool>(),
        lines in any::<bool>(),
        paras in any::<bool>(),
        spaces in prop::option::of(0u64..8),
    ) {
        let data = [bom, &bytes[..]].concat();
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let text = json!({
                "characterSet": if ansi { "ansi" } else { "unicode" },
                "platform": if mac { "mac" } else { "windows" },
                "removeLineReturns": lines,
                "removeParagraphReturns": paras,
                "replaceSpaces": spaces,
            });
            let p = json!({"name": "fuzz.txt", "dataBase64": vectorcraft_format::base64_encode(&data), "text": text, "thumbnail": 8});
            let _ = s.execute("file.place.info", &p);
            let _ = s.execute("file.place", &p);
        });
        prop_assert!(r.is_ok(), "panicked: {:?}", r.err());
    }
}

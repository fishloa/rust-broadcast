//! Golden for `dvb_ta::base64_encode`, generated from unmodified `main`.
use scte35_splice::dvb_ta::base64_encode;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

#[test]
fn base64_encode_matches_golden() {
    // RFC 4648 §10 test vectors plus every length mod 3 of a binary blob.
    let vectors: [&[u8]; 9] = [
        b"",
        b"f",
        b"fo",
        b"foo",
        b"foob",
        b"fooba",
        b"foobar",
        &[0xFC, 0x30, 0x0A, 0x00, 0xFF, 0xFE, 0xFB],
        &[0xFB, 0xEF, 0xBE, 0x3F, 0xFF],
    ];
    let mut out = String::new();
    for v in vectors {
        out.push_str(&format!(
            "{}\t{}\n",
            v.len(),
            String::from_utf8(base64_encode(v)).unwrap()
        ));
    }
    check("base64.txt", &out);
}

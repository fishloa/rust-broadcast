//! Goldens for DATERANGE rendering and RFC 3339 formatting, generated from
//! unmodified `main`. `GOLDEN_BLESS=<dir>` writes.
use std::fs;
use std::path::{Path, PathBuf};

use timed_metadata::DateRange;
use timed_metadata::anchor::format_rfc3339_ms;
use timed_metadata::daterange::{Scte35Attr, Scte35Cue};

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

fn range(cue: Scte35Cue, raw: Vec<u8>) -> DateRange {
    DateRange {
        id: "2002".to_string(),
        start_date: "2018-10-29T10:38:00.000Z".to_string(),
        class: None,
        duration: None,
        planned_duration: Some(24.0),
        scte35: Some(Scte35Attr { cue, raw }),
        extra_attrs: Vec::new(),
    }
}

#[test]
fn daterange_tag_lines_match_golden() {
    let long: Vec<u8> = (0u8..17).map(|i| i.wrapping_mul(0x0F)).collect();
    let cases = [
        range(Scte35Cue::Out, vec![0xFC, 0x30, 0x21]),
        range(Scte35Cue::In, vec![0x00]),
        range(Scte35Cue::Cmd, vec![0x0A, 0xA0, 0x0F, 0xFF, 0x01, 0xBE]),
        range(Scte35Cue::Out, long),
    ];
    let mut out = String::new();
    for dr in cases {
        out.push_str(&dr.to_tag_line().unwrap());
        out.push('\n');
    }
    check("daterange.txt", &out);
}

#[test]
fn rfc3339_formatting_matches_golden() {
    let epoch_ms: [i64; 12] = [
        0,
        1,
        999,
        1_000,
        86_400_000,
        -1,
        -86_400_000,
        951_782_400_000,   // 2000-02-29T00:00:00Z (leap day)
        4_107_542_400_000, // 2100-03-01T00:00:00Z (2100 is not a leap year)
        1_700_000_000_123,
        253_402_300_799_999, // 9999-12-31T23:59:59.999Z
        -62_135_596_800_000, // 0001-01-01T00:00:00Z
    ];
    let mut out = String::new();
    for ms in epoch_ms {
        out.push_str(&format!("{ms}\t{}\n", format_rfc3339_ms(ms)));
    }
    check("rfc3339.txt", &out);
}

//! `content_genre_name` against every row of EN 300 468 Table 29
//! (`private/specs-verified/dvb/dvb-si/enums/en_300_468/29-content-nibble-level-1-and-2-assignments.md`,
//! §6.2.9): all 256 `(nibble_1, nibble_2)` pairs must be covered by exactly one
//! row and carry that row's description ("reserved for future use" is the
//! crate's `"reserved"`).

use dvb_si::descriptors::content::content_genre_name;

fn parse_nibble_range(cell: &str) -> (u8, u8) {
    let hex = |s: &str| u8::from_str_radix(s.trim().trim_start_matches("0x"), 16).expect("hex nibble");
    match cell.split_once(" to ") {
        Some((lo, hi)) => (hex(lo), hex(hi)),
        None => (hex(cell), hex(cell)),
    }
}

#[test]
fn every_nibble_pair_matches_table_29() {
    let Some(md) = test_fixtures::spec_string(
        "dvb/dvb-si/enums/en_300_468/29-content-nibble-level-1-and-2-assignments.md",
        "dvb-si content_genre_table",
    ) else {
        return;
    };
    let mut covered = [[0u8; 16]; 16];
    for line in md.lines() {
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').collect();
        if cells.len() != 3 || !cells[0].trim().starts_with("0x") {
            continue;
        }
        let (n1_lo, n1_hi) = parse_nibble_range(cells[0]);
        let (n2_lo, n2_hi) = parse_nibble_range(cells[1]);
        let description = cells[2].trim();
        let want = if description.starts_with("reserved") {
            "reserved"
        } else {
            description
        };
        for n1 in n1_lo..=n1_hi {
            for n2 in n2_lo..=n2_hi {
                assert_eq!(
                    content_genre_name(n1, n2),
                    want,
                    "nibbles ({n1:#X}, {n2:#X})"
                );
                covered[usize::from(n1)][usize::from(n2)] += 1;
            }
        }
    }
    for n1 in 0..16 {
        for n2 in 0..16 {
            assert_eq!(covered[n1][n2], 1, "({n1:#X}, {n2:#X}) rows in Table 29");
        }
    }
}

#[test]
fn nibbles_above_four_bits_are_unknown() {
    assert_eq!(content_genre_name(0x10, 0), "unknown");
    assert_eq!(content_genre_name(0x1, 0x10), "unknown");
}

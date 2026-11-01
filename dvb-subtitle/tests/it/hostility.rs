//! No-panic hostility sweep (audit S3): real subtitle `PES_data_field`s from
//! the committed M6 capture are truncated at every length and mutated at every
//! byte position, then parsed (and re-serialized when they still parse). The
//! contract is "`Ok` or `Err`, never a panic": subtitle segments carry
//! attacker-controlled lengths and pixel-code strings, which is where a
//! panicking slice/index on a parsed length would hide.

use broadcast_common::{Parse, Serialize};
use dvb_subtitle::PesDataField;
/// Distinct real fields swept (bounded so the test stays fast).
const MAX_SEEDS: usize = 16;

fn exercise(bytes: &[u8]) -> bool {
    match PesDataField::parse(bytes) {
        Ok(f) => {
            let _ = f.try_to_bytes();
            true
        }
        Err(_) => false,
    }
}

#[test]
fn truncation_and_mutation_of_real_data_fields_never_panic() {
    let seeds = crate::support::data_fields(MAX_SEEDS);
    assert!(!seeds.is_empty(), "no real subtitle data fields found");
    let mut parsed_after_mutation = 0usize;
    for seed in &seeds {
        assert!(exercise(seed), "seed must parse");
        for n in 0..=seed.len() {
            exercise(&seed[..n]);
        }
        for pos in 0..seed.len() {
            for v in [0x00u8, 0x01, 0x0F, 0x7F, 0x80, 0xFF, seed[pos] ^ 0x55] {
                let mut m = seed.clone();
                m[pos] = v;
                if exercise(&m) {
                    parsed_after_mutation += 1;
                }
            }
        }
    }
    assert!(parsed_after_mutation > 100, "{parsed_after_mutation}");
}

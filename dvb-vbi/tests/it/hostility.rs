//! No-panic hostility sweep (audit S3): the committed EN 301 775 data field
//! (one of every typed data unit) is truncated at every length and mutated at
//! every byte position with every value. The contract is "`Ok` or `Err`,
//! never a panic"; whatever still parses is re-serialized.

use std::fs;

use broadcast_common::Serialize as _;
use dvb_vbi::DataField;

fn exercise(b: &[u8]) -> bool {
    match DataField::parse(b) {
        Ok(f) => {
            let reserialized = f.try_to_bytes().expect("reserializes");
            assert_eq!(reserialized.len(), f.serialized_len());
            true
        }
        Err(_) => false,
    }
}

#[test]
fn truncation_and_every_value_mutation_never_panic() {
    let seed = fs::read(test_fixtures::path("subtitles/vbi-data-field.bin"))
        .expect("fixture vbi-data-field.bin must be committed");
    assert!(exercise(&seed));
    for n in 0..=seed.len() {
        exercise(&seed[..n]);
    }
    let mut ok_after = 0usize;
    for pos in 0..seed.len() {
        for v in 0u8..=255 {
            let mut m = seed.clone();
            m[pos] = v;
            if exercise(&m) {
                ok_after += 1;
            }
        }
    }
    assert!(ok_after > 100, "{ok_after}");
}

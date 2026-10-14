//! No-panic hostility sweep (audit S3): the committed EN 301 775 data field
//! (one of every typed data unit) is truncated at every length and mutated at
//! every byte position with every value. The contract is "`Ok` or `Err`,
//! never a panic"; whatever still parses is re-serialized.

use std::fs;

use dvb_vbi::DataField;

fn exercise(b: &[u8]) -> bool {
    match DataField::parse(b) {
        Ok(f) => {
            let mut buf = vec![0u8; f.serialized_len()];
            let _ = f.serialize_into(&mut buf);
            true
        }
        Err(_) => false,
    }
}

#[test]
fn truncation_and_every_value_mutation_never_panic() {
    let seed = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-vbi/vbi_data_field.bin"
    ))
    .expect("fixture vbi_data_field.bin must be committed");
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

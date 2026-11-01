//! No-panic hostility sweep (audit S3): every real `splice_info_section` seed
//! is truncated at every length and mutated at every byte position with a set
//! of boundary values, with the CRC_32 REPAIRED after each mutation so the
//! corrupt body actually reaches the inner command / descriptor parsers
//! (a bad CRC would be rejected up front and test nothing below it). The
//! contract is only "returns `Ok` or `Err`, never panics, never allocates
//! unboundedly"; parse -> serialize of whatever still parses must not panic
//! either.

use base64::{Engine, engine::general_purpose::STANDARD};
use broadcast_common::{Parse, Serialize, crc32_mpeg2};
use scte35_splice::SpliceInfoSection;

/// Real sections: the SCTE 35 §14 samples and the published vectors the other
/// tests in this crate decode (CRC-valid).
const SEEDS: &[&str] = &[
    "/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg==",
    "/DARAAAAAAAAAP/wAAAAAHpPv/8=",
    "/DAvAAAAAAAA///wBQb+dGKQoAAZAhdDVUVJSAAAjn+fCAgAAAAALKChijUCAKnMZ1g=",
    "/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=",
    "/DAvAAAAAAAA///wBQb+rvF8TAAZAhdDVUVJSAAAB3+fCAgAAAAALKVslxEAAMSHai4=",
    "/DBhAAAAAAAA///wBQb+qM1E7QBLAhdDVUVJSAAArX+fCAgAAAAALLLXnTUCAAIXQ1VFSUgAACZ/nwgIAAAAACyy150RAAACF0NVRUlIAAAnf58ICAAAAAAsstezEAAAihiGnw==",
];

fn repair(mut b: Vec<u8>) -> Vec<u8> {
    if b.len() >= 4 {
        let n = b.len() - 4;
        let crc = crc32_mpeg2::compute(&b[..n]);
        b[n..].copy_from_slice(&crc.to_be_bytes());
    }
    b
}

fn exercise(bytes: &[u8]) {
    if let Ok(sec) = SpliceInfoSection::parse(bytes) {
        let _ = sec.try_to_bytes();
        // Walk what the typed accessors expose too.
        let _ = format!("{sec:?}");
    }
}

#[test]
fn truncation_and_crc_repaired_mutation_never_panic() {
    let mut parsed_after_mutation = 0usize;
    for seed in SEEDS {
        let bytes = STANDARD.decode(seed).unwrap();
        assert!(SpliceInfoSection::parse(&bytes).is_ok(), "seed must parse");
        // every prefix, with and without a repaired CRC
        for n in 0..=bytes.len() {
            exercise(&bytes[..n]);
            exercise(&repair(bytes[..n].to_vec()));
        }
        // every single-byte mutation, CRC repaired
        for pos in 0..bytes.len().saturating_sub(4) {
            for v in [0x00u8, 0x01, 0x7F, 0x80, 0xFE, 0xFF, bytes[pos] ^ 0x55] {
                let mut m = bytes.clone();
                m[pos] = v;
                let m = repair(m);
                if SpliceInfoSection::parse(&m).is_ok() {
                    parsed_after_mutation += 1;
                }
                exercise(&m);
            }
        }
    }
    // The sweep must actually reach the inner parsers (a repaired mutation
    // that still parses proves the CRC repair works).
    assert!(parsed_after_mutation > 100, "{parsed_after_mutation}");
}

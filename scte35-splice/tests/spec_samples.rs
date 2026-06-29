//! Every worked sample message from ANSI/SCTE 35 2023r1 §14 (PDF pp.112-121),
//! run through the crate end-to-end. Each base64 string is verbatim from the
//! spec's §14 (transcribed into `scte35-splice/docs/scte_35.md`, machine-verified
//! to decode to its stated byte length + match the spec's hex/CRC).
//!
//! For each: `parse` must succeed (which validates the splice_info_section
//! CRC_32 — a genuine message), and `serialize` must reproduce the input
//! byte-for-byte (the CRC is recomputed on serialize). This is the strongest
//! authoritative-vector coverage available: the spec's own messages.

use base64::{engine::general_purpose::STANDARD, Engine};
use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;

/// (label, base64) for all eight §14 sample messages.
const SAMPLES: &[(&str, &str)] = &[
    ("§14.1 time_signal – Placement Opportunity Start",
     "/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg=="),
    ("§14.2 splice_insert",
     "/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo="),
    ("§14.3 time_signal – Placement Opportunity End",
     "/DAvAAAAAAAA///wBQb+dGKQoAAZAhdDVUVJSAAAjn+fCAgAAAAALKChijUCAKnMZ1g="),
    ("§14.4 time_signal – Program Start/End",
     "/DBIAAAAAAAA///wBQb+ek2ItgAyAhdDVUVJSAAAGH+fCAgAAAAALMvDRBEAAAIXQ1VFSUgAABl/nwgIAAAAACyk26AQAACZcuND"),
    ("§14.5 time_signal – Program Overlap Start",
     "/DAvAAAAAAAA///wBQb+rr//ZAAZAhdDVUVJSAAACH+fCAgAAAAALKVs9RcAAJUdsKg="),
    ("§14.6 time_signal – Program Blackout Override / Program End",
     "/DBIAAAAAAAA///wBQb+ky44CwAyAhdDVUVJSAAACn+fCAgAAAAALKCh4xgAAAIXQ1VFSUgAAAl/nwgIAAAAACygoYoRAAC0IX6w"),
    ("§14.7 time_signal – Program End",
     "/DAvAAAAAAAA///wBQb+rvF8TAAZAhdDVUVJSAAAB3+fCAgAAAAALKVslxEAAMSHai4="),
    ("§14.8 time_signal – Program Start/End - Placement Opportunity End",
     "/DBhAAAAAAAA///wBQb+qM1E7QBLAhdDVUVJSAAArX+fCAgAAAAALLLXnTUCAAIXQ1VFSUgAACZ/nwgIAAAAACyy150RAAACF0NVRUlIAAAnf58ICAAAAAAsstezEAAAihiGnw=="),
];

#[test]
fn all_section14_samples_parse_and_round_trip() {
    for (label, b64) in SAMPLES {
        let bytes = STANDARD
            .decode(b64)
            .unwrap_or_else(|e| panic!("{label}: base64 decode: {e}"));
        // parse validates the CRC_32 (a wrong CRC is rejected).
        let section = SpliceInfoSection::parse(&bytes)
            .unwrap_or_else(|e| panic!("{label}: parse/CRC failed: {e:?}"));
        assert_eq!(bytes[0], 0xFC, "{label}: table_id");
        // serialize must reproduce the wire bytes exactly (recomputes CRC).
        assert_eq!(
            section.to_bytes(),
            bytes,
            "{label}: round-trip is not byte-identical"
        );
    }
}

/// Each sample also re-parses equal after a serialize cycle (serialize → parse).
#[test]
fn all_section14_samples_reparse_equal() {
    for (label, b64) in SAMPLES {
        let bytes = STANDARD.decode(b64).unwrap();
        let a = SpliceInfoSection::parse(&bytes).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let reser = a.to_bytes();
        let b =
            SpliceInfoSection::parse(&reser).unwrap_or_else(|e| panic!("{label}: reparse {e:?}"));
        assert_eq!(a, b, "{label}: serialize → parse not equal");
    }
}

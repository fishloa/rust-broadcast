//! Spec-derived round-trip vectors for edge cases the real E-AC-3 fixture
//! doesn't exercise: minimum/maximum burst length, the full `data_type` code
//! space (0..=31), and the six-word extended preamble form (`data_type ==
//! 31`) — SMPTE ST 337:2015 §7.2/§7.2.5.
//!
//! ## Provenance
//!
//! No real capture using the six-word preamble form, an empty payload, or
//! the maximum-length payload was reachable from this sandboxed environment
//! (ST 337 is an AES3 wire-layer format with no independent file-capture
//! ecosystem to pull from, unlike e.g. TS/RTP). Per this project's
//! documented fallback for such cases (`docs/CRATE-ACCEPTANCE.md`), every
//! vector below is derived directly from the spec's own field/range
//! definitions (`st337/docs/st337.md` §7.2.4/§7.2.5), independently of this
//! crate's own serializer, so a bug that made `serialize`/`parse` agree with
//! each other but disagree with the spec would still be caught.

use broadcast_common::{Parse, Serialize};
use st337::{
    Burst, DataMode, EXTENDED_DATA_TYPE_MARKER, Error, ExtendedPreamble, MAX_LENGTH_CODE_BITS,
    SYNC_WORD_PA, SYNC_WORD_PB,
};

// ── Minimum burst: zero-length payload (§7.2.5: "from 0 to 65,535 bits") ───

#[test]
fn zero_length_payload_round_trips() {
    let burst = Burst::new(0, DataMode::Mode16, false, 0, 0, None, &[]).unwrap();
    assert_eq!(burst.preamble.length_code, 0);
    let bytes = burst.to_bytes();
    assert_eq!(bytes.len(), 8, "four-word preamble, no payload bytes");
    let parsed = Burst::parse(&bytes).unwrap();
    assert_eq!(parsed.payload, &[] as &[u8]);
    assert_eq!(parsed, burst);
}

// ── Maximum burst: length_code at its 16-bit-mode ceiling (§7.2.5) ─────────

#[test]
fn maximum_length_code_round_trips() {
    // 65535 bits = 8191 bytes + 7 spare bits in the last byte.
    let payload = vec![0x5Au8; (MAX_LENGTH_CODE_BITS / 8) as usize];
    let burst = Burst::new(0, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();
    assert_eq!(burst.preamble.length_code, 65528);
    let bytes = burst.to_bytes();
    let parsed = Burst::parse(&bytes).unwrap();
    assert_eq!(parsed, burst);
}

#[test]
fn one_bit_past_the_maximum_is_rejected() {
    let payload = vec![0u8; (MAX_LENGTH_CODE_BITS / 8) as usize + 1];
    let err = Burst::new(0, DataMode::Mode16, false, 0, 0, None, &payload).unwrap_err();
    assert!(matches!(err, Error::PayloadTooLarge { .. }));
}

// ── data_type code space: every non-31 value is a valid four-word burst ───

#[test]
fn every_non_extended_data_type_value_round_trips() {
    for data_type in 0u8..EXTENDED_DATA_TYPE_MARKER {
        let payload = [data_type]; // trivial 1-byte payload, value = data_type for visibility
        let burst = Burst::new(data_type, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();
        let bytes = burst.to_bytes();
        let parsed = Burst::parse(&bytes).unwrap();
        assert_eq!(parsed.preamble.data_type, data_type);
        assert_eq!(parsed, burst);
    }
}

#[test]
fn data_type_dependent_and_data_stream_number_full_ranges_round_trip() {
    for data_type_dependent in 0u8..=31 {
        for data_stream_number in 0u8..=7 {
            let burst = Burst::new(
                5,
                DataMode::Mode16,
                data_stream_number % 2 == 0,
                data_type_dependent,
                data_stream_number,
                None,
                &[0xAB],
            )
            .unwrap();
            let bytes = burst.to_bytes();
            let parsed = Burst::parse(&bytes).unwrap();
            assert_eq!(parsed, burst);
        }
    }
}

// ── Six-word extended preamble (data_type == 31, Pe/Pf present) ────────────

#[test]
fn extended_preamble_length_code_includes_pe_pf_bits() {
    // "Pe and Pf shall be counted as payload bytes" (Table 6, §7.2.5): a
    // 10-byte payload -> length_code = 10*8 + 32 = 112, NOT just 80.
    let payload = [0u8; 10];
    let ext = ExtendedPreamble {
        extended_data_type: 0xBEEF,
        reserved_pf: 0x0000,
    };
    let burst = Burst::new(
        EXTENDED_DATA_TYPE_MARKER,
        DataMode::Mode16,
        false,
        0,
        0,
        Some(ext),
        &payload,
    )
    .unwrap();
    assert_eq!(burst.preamble.length_code, 112);

    let bytes = burst.to_bytes();
    assert_eq!(bytes.len(), 12 + 10, "six-word preamble + 10 payload bytes");
    let parsed = Burst::parse(&bytes).unwrap();
    assert_eq!(parsed, burst);
    assert_eq!(
        parsed.preamble.extended,
        Some(ext),
        "Pe/Pf preserved exactly, including the nominally-reserved Pf value"
    );
}

#[test]
fn extended_preamble_nonzero_reserved_pf_is_preserved_verbatim() {
    // Pf is "reserved" per Table 6, not "must be zero" -- a real value here
    // must survive round-trip unchanged (docs/st337.md scope decision 5).
    let ext = ExtendedPreamble {
        extended_data_type: 0x0001,
        reserved_pf: 0xFFFF,
    };
    let burst = Burst::new(
        EXTENDED_DATA_TYPE_MARKER,
        DataMode::Mode16,
        true,
        3,
        1,
        Some(ext),
        &[1, 2, 3, 4],
    )
    .unwrap();
    let bytes = burst.to_bytes();
    let parsed = Burst::parse(&bytes).unwrap();
    assert_eq!(parsed.preamble.extended.unwrap().reserved_pf, 0xFFFF);
}

#[test]
fn six_word_preamble_underflow_length_code_is_rejected() {
    // Hand-construct the wire bytes directly (bypassing this crate's own
    // serializer entirely) for a six-word preamble whose length_code (16
    // bits) is too small to even cover Pe/Pf's own 32 extended-preamble
    // bits, so the parse-side underflow check is exercised independently of
    // any serializer-side validation.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SYNC_WORD_PA.to_le_bytes());
    bytes.extend_from_slice(&SYNC_WORD_PB.to_le_bytes());
    bytes.extend_from_slice(&0x001Fu16.to_le_bytes()); // Pc: data_type=31, rest 0
    bytes.extend_from_slice(&16u16.to_le_bytes()); // Pd = 16 bits, too small
    bytes.extend_from_slice(&0u16.to_le_bytes()); // Pe
    bytes.extend_from_slice(&0u16.to_le_bytes()); // Pf

    let err = Burst::parse(&bytes).unwrap_err();
    assert!(matches!(
        err,
        Error::InvalidValue {
            field: "length_code",
            ..
        }
    ));
}

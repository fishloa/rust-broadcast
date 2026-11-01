//! Adversarial review (branch `review/dvbsi-descriptors`): failing regression
//! tests for defects found in the dvb-si descriptor changes since v11.1.0.
//! Each test states the invariant it pins; every one FAILS on the reviewed
//! `main` (9fe7a022) and must pass once the defect is fixed. Inputs are real
//! wire bytes built from the cited syntax tables.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::extension::{
    Cp, ExtensionDescriptor, S2XSatelliteDeliverySystem, S2Xv2SatelliteDeliverySystem,
    SupplementaryAudio,
};
use dvb_si::descriptors::linkage::LinkageDescriptor;
use dvb_si::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
use dvb_si::descriptors::nordig::NordigLogicalChannelV1;
use dvb_si::descriptors::service::ServiceDescriptor;
use dvb_si::descriptors::short_event::ShortEventDescriptor;
use dvb_si::descriptors::telephone::TelephoneDescriptor;
use dvb_si::error::Error;

fn ser<T: Serialize<Error = Error>>(d: &T) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; d.serialized_len()];
    let n = d.serialize_into(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// The workspace invariant for a tolerant parser: whatever `parse` accepts
/// must re-serialize byte-identically (or `parse` must reject it).
fn assert_rejects_or_round_trips<'a, T>(bytes: &'a [u8])
where
    T: Parse<'a, Error = Error> + Serialize<Error = Error> + core::fmt::Debug,
{
    if let Ok(d) = T::parse(bytes) {
        let out = ser(&d).expect("a parsed value must serialize");
        assert_eq!(
            out, bytes,
            "parse accepted the input but serialize is not byte-identical: {d:?}"
        );
    }
}

// ── 1. Silent `& MASK` on typed reserved fields (the #1129/#1197 class) ─────
// CHANGELOG [Unreleased] (#1197) claims every remaining single-field
// `(value & MASK)` in a serializer is a checked `fit_mask`; these five sites
// still mask a hand-set over-range reserved value into a different value.

/// nordig.rs:152 `(entry.reserved & 1) << 6`: reserved = 2 is written as 0.
#[test]
fn nordig_v1_reserved_over_range_is_rejected_not_masked() {
    // tag 0x83, one entry: service 0x0001, visible=1 reserved=1 LCN 5.
    let bytes = [0x83, 4, 0x00, 0x01, 0xC0, 0x05];
    let mut d = NordigLogicalChannelV1::parse(&bytes).unwrap();
    assert_eq!(ser(&d).unwrap(), bytes);
    d.entries[0].reserved = 2;
    let r = ser(&d);
    assert!(
        matches!(r, Err(Error::FieldOverflow(_))),
        "reserved=2 must be FieldOverflow, got {r:?}"
    );
}

/// extension/cp.rs:89 `self.reserved_future_use & 0b111`.
#[test]
fn cp_reserved_future_use_over_range_is_rejected_not_masked() {
    // CP_system_id 0x1234, reserved_future_use 111, CP_PID 0x0100.
    let sel = [0x12, 0x34, 0xE1, 0x00];
    let mut d = Cp::parse(&sel).unwrap();
    assert_eq!(ser(&d).unwrap(), sel);
    d.reserved_future_use = 0b1000;
    let r = ser(&d);
    assert!(
        matches!(r, Err(Error::FieldOverflow(_))),
        "reserved_future_use=8 must be FieldOverflow, got {r:?}"
    );
}

/// extension/supplementary_audio.rs:117 `(self.reserved_future_use & 1) << 1`.
#[test]
fn supplementary_audio_reserved_future_use_over_range_is_rejected_not_masked() {
    // mix_type 1, editorial_classification 1, reserved 1, no language.
    let sel = [0x86];
    let mut d = SupplementaryAudio::parse(&sel).unwrap();
    assert_eq!(ser(&d).unwrap(), sel);
    d.reserved_future_use = 2;
    let r = ser(&d);
    assert!(
        matches!(r, Err(Error::FieldOverflow(_))),
        "reserved_future_use=2 must be FieldOverflow, got {r:?}"
    );
}

/// extension/s2x_satellite_delivery_system.rs:488
/// `(reserved_zero_future_use_1 & 1) << PACKED_RZFU_1_SHIFT`.
#[test]
fn s2x_reserved_zero_future_use_1_over_range_is_rejected_not_masked() {
    let ok = S2XSatelliteDeliverySystem::default();
    ser(&ok).expect("default must serialize");
    let mut d = S2XSatelliteDeliverySystem::default();
    d.reserved_zero_future_use_1 = 2;
    let r = ser(&d);
    assert!(
        matches!(r, Err(Error::FieldOverflow(_))),
        "reserved_zero_future_use_1=2 must be FieldOverflow, got {r:?}"
    );
}

/// extension/s2xv2_satellite_delivery_system.rs:619
/// `None => self.reserved_zero_future_use_1 & 1`.
#[test]
fn s2xv2_reserved_zero_future_use_1_over_range_is_rejected_not_masked() {
    let ok = S2Xv2SatelliteDeliverySystem::default();
    ser(&ok).expect("default must serialize");
    let mut d = S2Xv2SatelliteDeliverySystem::default();
    assert_eq!(d.scrambling_sequence_selector, None);
    d.reserved_zero_future_use_1 = 2;
    let r = ser(&d);
    assert!(
        matches!(r, Err(Error::FieldOverflow(_))),
        "reserved_zero_future_use_1=2 must be FieldOverflow, got {r:?}"
    );
}

// ── 2. Wire bits lost on parse -> serialize ─────────────────────────────────

/// EN 300 468 Table 119 (SH_delivery_system_descriptor): the modulation flags
/// byte is `modulation_type(1) interleaver_presence(1) interleaver_type(1)
/// reserved_future_use(5)`; `interleaver_type` is on the wire even when
/// `interleaver_presence == 0`. Parse drops it (`interleaver: None`) and
/// serialize writes 0, so `3F` comes back as `1F`.
#[test]
fn sh_delivery_interleaver_type_bit_survives_when_no_interleaver() {
    // tag 0x7F, len 5, tag_extension 0x05, diversity_mode 0000 + rfu 1111,
    // flags: TDM, presence 0, interleaver_type 1, rfu 11111 = 0x3F,
    // TDM: pol 00 roll 01 mode 01 code_rate 0100 symbol_rate 00000 rfu 1.
    let bytes = [0x7F, 0x05, 0x05, 0x0F, 0x3F, 0x15, 0x01];
    let d = ExtensionDescriptor::parse(&bytes).expect("valid SH descriptor");
    assert_eq!(ser(&d).unwrap(), bytes, "interleaver_type bit lost: {d:?}");
}

/// EN 300 468 Table 65 (extended event linkage): `original_network_id_flag`
/// and `service_id_flag` are always on the wire; with `target_id_type == 3`
/// (user_defined_id) parse discards them and serialize writes 0 (`8E` -> `8C`).
#[test]
fn extended_event_linkage_flags_survive_with_user_defined_target() {
    // TSID 1, ONID 2, SID 3, linkage_type 0x0E, loop_length 5:
    // target_event_id 0x1234, listed=1 simulcast=0 link_type=00
    // target_id_type=11 onid_flag=1 sid_flag=0 (0x8E), user_defined_id 0xABCD.
    let bytes = [
        0x4A, 13, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x0E, 5, 0x12, 0x34, 0x8E, 0xAB, 0xCD,
    ];
    assert_rejects_or_round_trips::<LinkageDescriptor<'_>>(&bytes);
}

// ── 3. Trailing bytes accepted by parse and silently dropped ────────────────
// CHANGELOG [Unreleased]: "Tail bytes that parse used to drop silently are now
// kept in typed `trailing` fields" - these four still drop them.

/// telephone_descriptor (EN 300 468 Table 100), all lengths 0 + one extra byte.
#[test]
fn telephone_trailing_byte_is_not_silently_dropped() {
    let bytes = [0x57, 4, 0xE5, 0x80, 0x80, 0xAA];
    assert_rejects_or_round_trips::<TelephoneDescriptor<'_>>(&bytes);
}

/// audio_preselection_descriptor (EN 300 468 Table 110 / GY/T 422 Table 1):
/// one minimal preselection + one byte after the loop.
#[test]
fn audio_preselection_trailing_byte_is_not_silently_dropped() {
    let bytes = [0x7F, 5, 0x19, 0x08, 0x08, 0x00, 0xAA];
    assert_rejects_or_round_trips::<ExtensionDescriptor<'_>>(&bytes);
}

/// service_descriptor (EN 300 468 Table 86) + one byte after service_name.
#[test]
fn service_trailing_byte_is_not_silently_dropped() {
    let bytes = [0x48, 6, 0x01, 1, b'P', 1, b'N', 0xFF];
    assert_rejects_or_round_trips::<ServiceDescriptor<'_>>(&bytes);
}

/// short_event_descriptor (EN 300 468 Table 87) + one byte after text.
#[test]
fn short_event_trailing_byte_is_not_silently_dropped() {
    let bytes = [0x4D, 8, b'e', b'n', b'g', 1, b'A', 1, b'B', 0xFF];
    assert_rejects_or_round_trips::<ShortEventDescriptor<'_>>(&bytes);
}

// ── 4. serialize -> parse asymmetry ──────────────────────────────────────────

/// `Mpeg2ExtensionBody::Other { extension_descriptor_tag: 0x08, .. }` is
/// serialized as an MPEG-H 3D audio descriptor, which parses back as the typed
/// `MpegH3dAudio` variant: serialize -> parse is not equal. Serialize must
/// reject an `Other` carrying a typed tag (or parse must keep `Other`).
#[test]
fn mpeg2_extension_other_with_typed_tag_is_not_silently_retyped() {
    let bytes = [0x3F, 0x04, 0x08, 0x10, 0x7F, 0xC1];
    let typed = Mpeg2ExtensionDescriptor::parse(&bytes).unwrap();
    let Mpeg2ExtensionBody::MpegH3dAudio(_) = &typed.body else {
        panic!("fixture must parse as MPEG-H")
    };
    let other = Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::Other {
        extension_descriptor_tag: 0x08,
        data: vec![0x10, 0x7F, 0xC1],
    });
    match ser(&other) {
        Err(_) => {}
        Ok(out) => assert_eq!(
            Mpeg2ExtensionDescriptor::parse(&out).unwrap(),
            other,
            "serialize -> parse changed the value"
        ),
    }
}

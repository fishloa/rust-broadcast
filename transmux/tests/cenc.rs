//! Real-fixture round-trip tests for CENC encryption boxes (#429).
//!
//! These tests navigate the container hierarchy in a real CENC MP4 file
//! (`fixtures/mp4/cenc.mp4`) and verify byte-exact round-trip for each box.
//!
//! The fixture was authored by ffmpeg with `cenc-aes-ctr` scheme and carries
//! `tenc`/`senc`/`saiz`/`saio`/`schm`/`frma`/`sinf`.

use broadcast_common::{Parse, Serialize};
use transmux::{
    OriginalFormatBox, ProtectionSchemeInfoBox, ProtectionSystemSpecificHeaderBox,
    SampleAuxInfoOffsetsBox, SampleAuxInfoSizesBox, SampleEncryptionBox, SchemeInformationBox,
    SchemeTypeBox, TrackEncryptionBox, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
};

fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == fourcc)
        .unwrap_or_else(|| {
            panic!(
                "{} four-CC must be present",
                std::str::from_utf8(fourcc).unwrap()
            )
        });
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start..start + size]
}

#[test]
fn tenc_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let tenc_bytes = find_box(&data, b"tenc");

    let tenc = TrackEncryptionBox::parse_box(tenc_bytes).expect("parse tenc");
    assert_eq!(tenc.version, 0);
    assert_eq!(tenc.default_is_protected, 1);
    assert_eq!(tenc.default_per_sample_iv_size, 8);
    let expected_kid = [
        0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3,
        0xb8,
    ];
    assert_eq!(tenc.default_kid, expected_kid);
    assert_eq!(tenc.default_constant_iv, None);

    // Byte-exact round-trip
    let mut out = vec![0u8; tenc.serialized_len()];
    let n = tenc.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], tenc_bytes, "tenc round-trip byte-exact");
}

#[test]
fn tenc_mutation_changes_bytes() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let tenc_bytes = find_box(&data, b"tenc");
    let mut tenc = TrackEncryptionBox::parse_box(tenc_bytes).unwrap();
    let original = {
        let mut buf = vec![0u8; tenc.serialized_len()];
        tenc.serialize_into(&mut buf).unwrap();
        buf
    };
    tenc.default_is_protected = 0;
    let mutated = {
        let mut buf = vec![0u8; tenc.serialized_len()];
        tenc.serialize_into(&mut buf).unwrap();
        buf
    };
    assert_ne!(mutated, original, "mutation must change bytes");
}

#[test]
fn schm_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let schm_bytes = find_box(&data, b"schm");

    // Parse full box manually: version+flags then body
    let version = schm_bytes[8];
    let flags = u32::from_be_bytes([0, schm_bytes[9], schm_bytes[10], schm_bytes[11]]);
    let schm = SchemeTypeBox::parse_body(&schm_bytes[12..], version, flags).expect("parse schm");
    assert_eq!(schm.version, 0);
    assert_eq!(&schm.scheme_type, b"cenc");
    assert_eq!(schm.scheme_version, 0x0001_0000);

    let mut out = vec![0u8; schm.serialized_len()];
    let n = schm.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], schm_bytes, "schm round-trip byte-exact");
}

#[test]
fn frma_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let frma_bytes = find_box(&data, b"frma");

    let frma = OriginalFormatBox::parse(frma_bytes).expect("parse frma");
    assert_eq!(&frma.data_format, b"avc1");

    let mut out = vec![0u8; frma.serialized_len()];
    let n = frma.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], frma_bytes, "frma round-trip byte-exact");
}

#[test]
fn senc_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let senc_bytes = find_box(&data, b"senc");

    let version = senc_bytes[8];
    let flags = u32::from_be_bytes([0, senc_bytes[9], senc_bytes[10], senc_bytes[11]]);
    assert_eq!(
        flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
        SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
        "UseSubSampleEncryption must be set"
    );

    // Per-sample IV size from tenc: 8
    let senc =
        SampleEncryptionBox::parse_body(&senc_bytes[12..], version, flags, 8).expect("parse senc");
    assert_eq!(senc.version, 0);
    assert_eq!(senc.entries.len(), 15, "15 samples");
    // Each entry has an 8-byte IV
    assert_eq!(senc.entries[0].initialization_vector.len(), 8);
    // Subsamples: 5 ranges per sample (from hex)
    assert!(!senc.entries[0].subsamples.is_empty());

    let mut out = vec![0u8; senc.serialized_len()];
    let n = senc.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], senc_bytes, "senc round-trip byte-exact");
}

#[test]
fn senc_mutation_changes_bytes() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let senc_bytes = find_box(&data, b"senc");
    let version = senc_bytes[8];
    let flags = u32::from_be_bytes([0, senc_bytes[9], senc_bytes[10], senc_bytes[11]]);
    let mut senc = SampleEncryptionBox::parse_body(&senc_bytes[12..], version, flags, 8).unwrap();
    let original = {
        let mut buf = vec![0u8; senc.serialized_len()];
        senc.serialize_into(&mut buf).unwrap();
        buf
    };
    senc.entries[0].initialization_vector[0] ^= 1;
    let mutated = {
        let mut buf = vec![0u8; senc.serialized_len()];
        senc.serialize_into(&mut buf).unwrap();
        buf
    };
    assert_ne!(mutated, original, "IV mutation must change bytes");
}

#[test]
fn saiz_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let saiz_bytes = find_box(&data, b"saiz");

    let saiz = SampleAuxInfoSizesBox::parse_box(saiz_bytes).expect("parse saiz");
    assert_eq!(saiz.version, 0);
    assert_eq!(saiz.default_sample_info_size, 0);
    assert_eq!(saiz.sample_info_sizes.len(), 15);
    assert_eq!(saiz.sample_info_sizes[0], 0x28);

    let mut out = vec![0u8; saiz.serialized_len()];
    let n = saiz.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], saiz_bytes, "saiz round-trip byte-exact");
}

#[test]
fn saio_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let saio_bytes = find_box(&data, b"saio");

    let saio = SampleAuxInfoOffsetsBox::parse_box(saio_bytes).expect("parse saio");
    assert_eq!(saio.version, 0);
    assert_eq!(saio.offsets.len(), 1);
    assert_eq!(saio.offsets[0], 0x5DA6);

    let mut out = vec![0u8; saio.serialized_len()];
    let n = saio.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], saio_bytes, "saio round-trip byte-exact");
}

#[test]
fn sinf_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let sinf_bytes = find_box(&data, b"sinf");

    let sinf = ProtectionSchemeInfoBox::parse(sinf_bytes).expect("parse sinf");
    assert_eq!(&sinf.original_format.data_format, b"avc1");
    let schm = sinf.scheme_type.as_ref().expect("sinf must have schm");
    assert_eq!(&schm.scheme_type, b"cenc");
    let schi = sinf.scheme_info.as_ref().expect("sinf must have schi");
    let tenc = schi.tenc.as_ref().expect("schi must have tenc");
    assert_eq!(tenc.default_is_protected, 1);
    assert_eq!(tenc.default_per_sample_iv_size, 8);

    let mut out = vec![0u8; sinf.serialized_len()];
    let n = sinf.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], sinf_bytes, "sinf round-trip byte-exact");
}

#[test]
fn pssh_spec_vector_round_trip() {
    // Widevine system ID
    let system_id = [
        0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21,
        0xed,
    ];
    let data = vec![0x08, 0x01, 0x12, 0x10, 0x00, 0x00, 0x00, 0x00];
    let pssh = ProtectionSystemSpecificHeaderBox {
        version: 0,
        system_id,
        kids: Vec::new(),
        data: data.clone(),
    };

    let mut buf = vec![0u8; pssh.serialized_len()];
    let n = pssh.serialize_into(&mut buf).unwrap();

    // Parse back from full box bytes
    let version = buf[8];
    let _flags = u32::from_be_bytes([0, buf[9], buf[10], buf[11]]);
    let parsed = ProtectionSystemSpecificHeaderBox::parse_body(&buf[12..], version).unwrap();
    assert_eq!(parsed.version, 0);
    assert_eq!(parsed.system_id, system_id);
    assert_eq!(parsed.data, data);
    assert!(parsed.kids.is_empty());

    // Byte-exact round-trip
    let mut out2 = vec![0u8; parsed.serialized_len()];
    let n2 = parsed.serialize_into(&mut out2).unwrap();
    assert_eq!(&out2[..n2], &buf[..n]);
}

#[test]
fn schi_round_trip() {
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/cenc.mp4"
    ))
    .expect("fixture must exist");
    let schi_bytes = find_box(&data, b"schi");

    let schi = SchemeInformationBox::parse(schi_bytes).expect("parse schi");
    let tenc = schi.tenc.as_ref().expect("schi must have tenc");
    assert_eq!(tenc.default_is_protected, 1);

    let mut out = vec![0u8; schi.serialized_len()];
    let n = schi.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], schi_bytes, "schi round-trip byte-exact");
}

//! Real ffmpeg-oracle byte-exact round-trip tests for the new codec config boxes
//! (#436/#437/#431/#432): AV1 `av1C`, Opus `dOps`, FLAC `dfLa`, VP9 `vpcC`,
//! AC-4 `dac4`, and HE-AAC ASC SBR/PS signaling.
//!
//! Each config box is extracted from the real fixture (or built from the recorded
//! oracle bytes for AC-4, whose source `.mp4` is non-redistributable), then
//! byte-exact round-tripped. A mutation-proof per box asserts that changing a
//! field changes the serialized bytes (no raw passthrough).
//!
//! Fixtures: `fixtures/mp4/{av1,opus,flac,vp9}.mp4`, `fixtures/ts/heaac/heaac_v{1,2}.mp4`.
//! Oracle refs: `fixtures/mp4/FMP4-GAPS-ORACLE.md`, `fixtures/ts/ac4/DAC4-ORACLE.md`.

use broadcast_common::{Parse, Serialize};
use transmux::{
    Ac4SpecificBox, AudioSpecificConfig, Av1ConfigurationBox, EsdsBox, FlacSpecificBox,
    OpusSpecificBox, Vp9ConfigurationBox,
};

/// Linear-scan for a box four-CC and return the box body (bytes after the 8-byte header).
fn find_box_body<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == four_cc.as_slice())
        .expect("box four-CC must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start + 8..start + size]
}

/// Return the full box bytes (including the 8-byte header) for a four-CC.
fn find_full_box<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == four_cc.as_slice())
        .expect("box four-CC must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start..start + size]
}

fn fixture(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {path} must exist: {e}"))
}

fn roundtrip<T: Serialize<Error = transmux::Error>>(v: &T, oracle: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; v.serialized_len()];
    let n = v.serialize_into(&mut buf).expect("serialize");
    buf.truncate(n);
    assert_eq!(buf.as_slice(), oracle, "serialized bytes must equal oracle");
    buf
}

// ---------------------------------------------------------------------------
// AV1 av1C — fixtures/mp4/av1.mp4 (#436)
// ---------------------------------------------------------------------------

#[test]
fn av1c_round_trip_and_fields() {
    let data = fixture("mp4/av1.mp4");
    let body = find_box_body(&data, b"av1C");
    assert_eq!(
        hex(body),
        "81000c000a0b000000043cffbc02f80040",
        "av1C oracle body"
    );

    let cfg = Av1ConfigurationBox::parse(body).expect("parse av1C");
    assert_eq!(cfg.version, 1, "record version");
    assert_eq!(cfg.seq_profile, 0, "seq_profile");
    assert_eq!(cfg.seq_level_idx_0, 0, "seq_level_idx_0");
    assert!(!cfg.seq_tier_0, "seq_tier_0");
    assert!(cfg.chroma_subsampling_x, "chroma_subsampling_x");
    assert!(cfg.chroma_subsampling_y, "chroma_subsampling_y");
    assert_eq!(cfg.bit_depth(), 8, "8-bit");
    assert!(cfg.initial_presentation_delay_minus_one.is_none());
    assert!(!cfg.config_obus.is_empty(), "Sequence Header OBU present");
    assert!(
        cfg.rfc6381().starts_with("av01.0.00M.08"),
        "{}",
        cfg.rfc6381()
    );

    roundtrip(&cfg, body);
}

#[test]
fn av1c_mutation_changes_bytes() {
    let data = fixture("mp4/av1.mp4");
    let body = find_box_body(&data, b"av1C");
    let mut cfg = Av1ConfigurationBox::parse(body).unwrap();
    cfg.seq_profile = 2;
    let mut buf = vec![0u8; cfg.serialized_len()];
    cfg.serialize_into(&mut buf).unwrap();
    assert_ne!(
        buf.as_slice(),
        body,
        "mutating seq_profile must change bytes"
    );
}

// ---------------------------------------------------------------------------
// Opus dOps — fixtures/mp4/opus.mp4 (#437)
// ---------------------------------------------------------------------------

#[test]
fn dops_round_trip_and_fields() {
    let data = fixture("mp4/opus.mp4");
    let body = find_box_body(&data, b"dOps");
    assert_eq!(hex(body), "000101380000bb80000000", "dOps oracle body");

    let cfg = OpusSpecificBox::parse(body).expect("parse dOps");
    assert_eq!(cfg.version, 0);
    assert_eq!(cfg.output_channel_count, 1, "1 channel");
    assert_eq!(cfg.pre_skip, 0x0138, "big-endian pre_skip");
    assert_eq!(cfg.input_sample_rate, 48000, "big-endian 0xbb80 = 48000");
    assert_eq!(cfg.output_gain, 0);
    assert_eq!(cfg.channel_mapping_family, 0);
    assert!(cfg.channel_mapping.is_none());
    assert_eq!(cfg.rfc6381(), "Opus");

    roundtrip(&cfg, body);
}

#[test]
fn dops_mutation_changes_bytes() {
    let data = fixture("mp4/opus.mp4");
    let body = find_box_body(&data, b"dOps");
    let mut cfg = OpusSpecificBox::parse(body).unwrap();
    cfg.pre_skip = 0x1234;
    let mut buf = vec![0u8; cfg.serialized_len()];
    cfg.serialize_into(&mut buf).unwrap();
    assert_ne!(buf.as_slice(), body, "mutating pre_skip must change bytes");
}

// ---------------------------------------------------------------------------
// FLAC dfLa — fixtures/mp4/flac.mp4 (#437)
// ---------------------------------------------------------------------------

#[test]
fn dfla_round_trip_and_fields() {
    let data = fixture("mp4/flac.mp4");
    let body = find_box_body(&data, b"dfLa");

    let cfg = FlacSpecificBox::parse(body).expect("parse dfLa");
    assert_eq!(cfg.version, 0, "FullBox version 0");
    assert_eq!(cfg.flags, 0);
    assert!(!cfg.blocks.is_empty(), "≥1 metadata block");
    let first = &cfg.blocks[0];
    assert_eq!(first.block_type, 0, "first block is STREAMINFO");
    assert!(first.last, "single STREAMINFO block is last");
    assert_eq!(first.data.len(), 34, "STREAMINFO is 34 bytes");
    assert_eq!(cfg.streaminfo().map(|s| s.len()), Some(34));
    assert_eq!(cfg.rfc6381(), "fLaC");

    roundtrip(&cfg, body);
}

#[test]
fn dfla_mutation_changes_bytes() {
    let data = fixture("mp4/flac.mp4");
    let body = find_box_body(&data, b"dfLa");
    let mut cfg = FlacSpecificBox::parse(body).unwrap();
    cfg.blocks[0].data[0] ^= 0xFF;
    let mut buf = vec![0u8; cfg.serialized_len()];
    cfg.serialize_into(&mut buf).unwrap();
    assert_ne!(
        buf.as_slice(),
        body,
        "mutating STREAMINFO must change bytes"
    );
}

// ---------------------------------------------------------------------------
// VP9 vpcC — fixtures/mp4/vp9.mp4 (#437)
// ---------------------------------------------------------------------------

#[test]
fn vpcc_round_trip_and_fields() {
    let data = fixture("mp4/vp9.mp4");
    let body = find_box_body(&data, b"vpcC");
    assert_eq!(hex(body), "010000000014820202020000", "vpcC oracle body");

    let cfg = Vp9ConfigurationBox::parse(body).expect("parse vpcC");
    assert_eq!(cfg.version, 1, "FullBox v1");
    assert_eq!(cfg.flags, 0);
    assert_eq!(cfg.profile, 0);
    assert_eq!(cfg.level, 20);
    assert_eq!(cfg.bit_depth, 8);
    assert_eq!(cfg.chroma_subsampling, 1);
    assert!(!cfg.video_full_range_flag);
    assert_eq!(cfg.colour_primaries, 2);
    assert_eq!(cfg.transfer_characteristics, 2);
    assert_eq!(cfg.matrix_coefficients, 2);
    assert!(
        cfg.codec_initialization_data.is_empty(),
        "MUST be empty for VP9"
    );
    assert_eq!(cfg.rfc6381(), "vp09.00.20.08");

    roundtrip(&cfg, body);
}

#[test]
fn vpcc_mutation_changes_bytes() {
    let data = fixture("mp4/vp9.mp4");
    let body = find_box_body(&data, b"vpcC");
    let mut cfg = Vp9ConfigurationBox::parse(body).unwrap();
    cfg.level = 41;
    let mut buf = vec![0u8; cfg.serialized_len()];
    cfg.serialize_into(&mut buf).unwrap();
    assert_ne!(buf.as_slice(), body, "mutating level must change bytes");
}

// ---------------------------------------------------------------------------
// AC-4 dac4 — recorded oracle bytes (#431; source mp4 non-redistributable)
// ---------------------------------------------------------------------------

/// The `dac4` (AC4SpecificBox = ac4_dsi_v1) oracle body from
/// `fixtures/ts/ac4/DAC4-ORACLE.md` (29 bytes, real Dolby AC-4 init segment).
const DAC4_ORACLE: &[u8] = &[
    0x20, 0xa4, 0x01, 0x40, 0x00, 0x00, 0x00, 0x1f, 0xff, 0xff, 0xff, 0xe0, 0x01, 0x0f, 0xf8, 0x80,
    0x00, 0x00, 0x42, 0x00, 0x00, 0x25, 0x01, 0x00, 0x00, 0x00, 0x30, 0x08, 0x00,
];

#[test]
fn dac4_round_trip_and_fields() {
    assert_eq!(DAC4_ORACLE.len(), 29, "dac4 oracle is 29 bytes");

    let cfg = Ac4SpecificBox::parse(DAC4_ORACLE).expect("parse dac4");
    assert_eq!(cfg.ac4_dsi.len(), 29, "opaque ac4_dsi preserved");
    assert_eq!(cfg.rfc6381(), "ac-4");

    roundtrip(&cfg, DAC4_ORACLE);

    // Wrapping in the ac-4 sample entry and re-parsing must preserve the dac4 body.
    let track = transmux::TrackSpec::new(
        1,
        48000,
        transmux::CodecConfig::Ac4 {
            config: cfg.clone(),
            channel_count: 2,
            sample_rate: 48000,
            sample_size: 16,
        },
    );
    let init = transmux::build_init_segment(&[track], 48000).expect("init segment");
    let dac4_body = find_box_body(&init, b"dac4");
    assert_eq!(dac4_body, DAC4_ORACLE, "dac4 in ac-4 entry == oracle");
    assert!(
        init.windows(4).any(|w| w == b"ac-4"),
        "ac-4 sample entry present"
    );
}

#[test]
fn dac4_mutation_changes_bytes() {
    let mut cfg = Ac4SpecificBox::parse(DAC4_ORACLE).unwrap();
    cfg.ac4_dsi[0] ^= 0xFF;
    let mut buf = vec![0u8; cfg.serialized_len()];
    cfg.serialize_into(&mut buf).unwrap();
    assert_ne!(
        buf.as_slice(),
        DAC4_ORACLE,
        "mutating dsi must change bytes"
    );
}

// ---------------------------------------------------------------------------
// HE-AAC ASC SBR/PS signaling — fixtures/ts/heaac/heaac_v{1,2}.mp4 (#432)
// ---------------------------------------------------------------------------

/// Extract the AudioSpecificConfig (DecoderSpecificInfo) bytes from an esds box.
fn asc_from_esds(esds_box: &[u8]) -> Vec<u8> {
    let esds = EsdsBox::parse_box(esds_box).expect("parse esds");
    esds.es_descriptor
        .decoder_config
        .expect("decoder config")
        .decoder_specific_info
        .expect("DecoderSpecificInfo")
        .data
}

#[test]
fn heaac_v1_sbr_signaling_and_esds_round_trip() {
    let data = fixture("ts/heaac/heaac_v1.mp4");
    let esds_box = find_full_box(&data, b"esds");

    // Byte-exact esds round-trip.
    let esds = EsdsBox::parse_box(esds_box).expect("parse esds v1");
    let mut buf = vec![0u8; esds.serialized_len()];
    let n = esds.serialize_into(&mut buf).unwrap();
    buf.truncate(n);
    assert_eq!(buf.as_slice(), esds_box, "esds v1 round-trip byte-exact");

    // ASC oracle + SBR detection.
    let asc_bytes = asc_from_esds(esds_box);
    assert_eq!(hex(&asc_bytes), "139056e5a0", "ASC v1 oracle");
    let asc = AudioSpecificConfig::parse(&asc_bytes).expect("parse ASC v1");
    let sig = asc.heaac_signaling();
    assert!(sig.sbr_present, "SBR must be signaled (HE-AAC v1)");
    assert!(!sig.ps_present, "PS not signaled in v1");
    assert_eq!(asc.rfc6381(), "mp4a.40.5", "HE-AAC v1 rfc6381");

    // ASC bytes themselves round-trip byte-exact.
    let mut ab = vec![0u8; asc.serialized_len()];
    let an = asc.serialize_into(&mut ab).unwrap();
    ab.truncate(an);
    assert_eq!(ab.as_slice(), asc_bytes.as_slice(), "ASC v1 round-trip");

    // C2 (#1008): the real fixture uses backward-compatible (trailing sync
    // extension) signaling, so its top-level audioObjectType is already the
    // core AAC-LC codec (2) — to_adts_header must emit ADTS profile 1 (LC),
    // matching ffprobe's classification of this exact file (`profile=HE-AAC`,
    // codec_name=aac; MP4Box: "AAC LC (AOT=2 backward compatible)").
    let adts = asc.to_adts_header(200).unwrap();
    let parsed_adts = transmux::parse_adts_header(&adts).unwrap();
    assert_eq!(
        parsed_adts.profile, 1,
        "ADTS profile must be LC (AOT 2 - 1)"
    );
}

#[test]
fn heaac_v2_ps_signaling_and_esds_round_trip() {
    let data = fixture("ts/heaac/heaac_v2.mp4");
    let esds_box = find_full_box(&data, b"esds");

    let esds = EsdsBox::parse_box(esds_box).expect("parse esds v2");
    let mut buf = vec![0u8; esds.serialized_len()];
    let n = esds.serialize_into(&mut buf).unwrap();
    buf.truncate(n);
    assert_eq!(buf.as_slice(), esds_box, "esds v2 round-trip byte-exact");

    let asc_bytes = asc_from_esds(esds_box);
    // Real esds DSI carries an explicit psPresentFlag byte (`80`) beyond the
    // 6-byte oracle in the doc; round-trip against the real 7 bytes.
    assert_eq!(
        hex(&asc_bytes),
        "138856e5a54880",
        "ASC v2 oracle (real DSI)"
    );
    let asc = AudioSpecificConfig::parse(&asc_bytes).expect("parse ASC v2");
    let sig = asc.heaac_signaling();
    assert!(sig.sbr_present, "SBR present in v2 too");
    assert!(sig.ps_present, "PS must be signaled (HE-AAC v2)");
    assert_eq!(asc.rfc6381(), "mp4a.40.29", "HE-AAC v2 rfc6381");

    let mut ab = vec![0u8; asc.serialized_len()];
    let an = asc.serialize_into(&mut ab).unwrap();
    ab.truncate(an);
    assert_eq!(ab.as_slice(), asc_bytes.as_slice(), "ASC v2 round-trip");

    // C2 (#1008): same as v1 — real-world backward-compatible signaling has
    // core AOT 2 (LC) at the top, so this fixture alone can't reach the buggy
    // hierarchical-AOT branch, but it does confirm the fix doesn't regress
    // the (far more common) backward-compatible path.
    let adts = asc.to_adts_header(200).unwrap();
    let parsed_adts = transmux::parse_adts_header(&adts).unwrap();
    assert_eq!(
        parsed_adts.profile, 1,
        "ADTS profile must be LC (AOT 2 - 1)"
    );
}

/// C2 (#1008): explicit **hierarchical** SBR/PS signaling — `audioObjectType
/// == 5` (SBR) or `29` (PS) at the TOP of the ASC, ISO/IEC 14496-3 §1.6.2.1 +
/// Amd 1/2, transcribed in `transmux/docs/codec/heaac-asc.md`. Before the fix,
/// `to_adts_header` used `self.audio_object_type.raw()` directly: AOT 5 gives
/// `(5-1)&3 == 0` and AOT 29 gives `(29-1)&3 == 0` — both silently become ADTS
/// profile 0 (AAC **Main**) instead of the actual core codec.
///
/// No local tool emits this exact wire form: macOS AudioToolbox (`aac_at`,
/// used for `fixtures/ts/heaac/heaac_v{1,2}.mp4` above) only ever writes the
/// backward-compatible (trailing sync-extension) form, and `libfdk_aac` isn't
/// built into this ffmpeg. Per the workspace fixture-first rule, these bytes
/// are hand-built directly from the cited spec table's field order/widths
/// (not from our own serializer), with a plain AAC-LC core (AOT 2) and a
/// disabled GASpecificConfig tail (`frameLengthFlag`/`dependsOnCoreCoder`/
/// `extensionFlag` all 0) so no encoder-specific extension bits are guessed.
#[test]
fn heaac_explicit_hierarchical_adts_profile_is_core_aot_not_extension_aot() {
    // aot(5)=5 sfi(4)=3(48000) cv(4)=2(stereo) ext_sfi(4)=6(24000) core_aot(5)=2(LC) GASpecificConfig(3)=0
    let sbr_bytes: [u8; 4] = [0x29, 0x93, 0x08, 0x00];
    let asc = AudioSpecificConfig::parse(&sbr_bytes).unwrap();
    assert_eq!(
        asc.audio_object_type.raw(),
        5,
        "top-level AOT is the SBR extension type"
    );
    let adts = asc.to_adts_header(200).unwrap();
    let parsed = transmux::parse_adts_header(&adts).unwrap();
    assert_eq!(
        parsed.profile, 1,
        "core AOT is AAC-LC (2) -> ADTS profile 1, not Main (0)"
    );

    // aot(5)=29 sfi(4)=3(48000) cv(4)=2(stereo) ext_sfi(4)=6(24000) core_aot(5)=2(LC) GASpecificConfig(3)=0
    let ps_bytes: [u8; 4] = [0xe9, 0x93, 0x08, 0x00];
    let asc = AudioSpecificConfig::parse(&ps_bytes).unwrap();
    assert_eq!(
        asc.audio_object_type.raw(),
        29,
        "top-level AOT is the PS extension type"
    );
    let adts = asc.to_adts_header(200).unwrap();
    let parsed = transmux::parse_adts_header(&adts).unwrap();
    assert_eq!(
        parsed.profile, 1,
        "core AOT is AAC-LC (2) -> ADTS profile 1, not Main (0)"
    );
}

/// C2 (#1008): a core AOT that ADTS cannot express (outside 1..=4, e.g.
/// ER AAC LD = 23) must be rejected, not silently wrapped into an unrelated
/// profile (`(23-1)&3 == 2` would previously have emitted AAC-SSR).
#[test]
fn heaac_hierarchical_with_unrepresentable_core_aot_is_rejected() {
    // aot(5)=5 sfi(4)=3 cv(4)=2 ext_sfi(4)=6 core_aot(5)=23(ER AAC LD) GASpecificConfig(3)=0
    let bytes: [u8; 4] = [0x29, 0x93, 0x5c, 0x00];
    let asc = AudioSpecificConfig::parse(&bytes).unwrap();
    assert!(asc.to_adts_header(200).is_err());
}

/// C2 (#1008): `sampling_frequency_index == Escape` is a 24-bit explicit
/// rate, forbidden in ADTS's 4-bit field — must be rejected, not copied in
/// as the literal escape value `0xF`.
#[test]
fn heaac_escape_sampling_frequency_is_rejected_for_adts() {
    let mut asc = AudioSpecificConfig::parse(&[0x12, 0x08]).unwrap();
    asc.sampling_frequency_index = transmux::SamplingFrequencyIndex::Escape;
    asc.sampling_frequency = Some(48000);
    assert!(asc.to_adts_header(200).is_err());
}

#[test]
fn plain_aac_lc_stays_mp4a_40_2() {
    // A plain AAC-LC ASC (no SBR/PS) must not be mis-detected as HE-AAC.
    let asc = AudioSpecificConfig::parse(&[0x12, 0x08]).unwrap();
    let sig = asc.heaac_signaling();
    assert!(!sig.sbr_present);
    assert!(!sig.ps_present);
    assert_eq!(asc.rfc6381(), "mp4a.40.2");
}

/// Lowercase hex of a byte slice.
fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for &x in b {
        write!(s, "{x:02x}").unwrap();
    }
    s
}

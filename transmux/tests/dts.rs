//! Spec-vector gate for DTS fMP4 carriage — `dtsc`/`dtsh`/`dtsl`/`dtse` +
//! `ddts` DTSSpecificBox (ETSI TS 102 114 §E.2.2.3, #437).
//!
//! **Real-fixture gate deferred**: ffmpeg's mp4 muxer does not emit `ddts`
//! boxes (no encoder). The oracle bytes below are hand-computed from the
//! §E.2.2.3.1 field layout and verified against the spec bit-arithmetic.
//! When a real DTS-in-MP4 fixture becomes available, add a byte-exact
//! `real_fixture_round_trip` test here and remove this note.

use broadcast_common::{Parse, Serialize};
use transmux::{
    CodecConfig, DDTS_BODY_LEN, DTSC_FOURCC, DtsSpecificBox, SampleEntryVariant, TrackSpec,
    box_iter, build_init_segment,
};

// ---------------------------------------------------------------------------
// Hand-computed spec vector
// ---------------------------------------------------------------------------
//
// Field values (ETSI TS 102 114 §E.2.2.3.1):
//   DTSSamplingFrequency = 48000       → 0x0000_BB80
//   maxBitrate           = 1_509_000   → 0x0017_0688  (1509000 = 0x170688)
//   avgBitrate           = 754_500     → 0x000B_8344  (754500  = 0x0B8344)
//   pcmSampleDepth       = 16          → 0x10
//
// Packed 32-bit word (MSB first, bits annotated):
//   [31:30] FrameDuration=1       → 01
//   [29:25] StreamConstruction=2  → 00010
//   [24]    CoreLFEPresent=1      → 1
//   [23:18] CoreLayout=9          → 001001
//   [17:4]  CoreSize=0x0200       → 00001000000000
//   [3]     StereoDownmix=0       → 0
//   [2:0]   RepresentationType=3  → 011
//
//   = 01 00010 1 001001 00001000000000 0 011
//   = 0100_0101_0010_0100_0010_0000_0000_0011
//   = 0x45_24_20_03
//
// ChannelLayout = 0x0003            → 0x00_03
//
// Flags byte:
//   [7] MultiAssetFlag=0, [6] LBRDurationMod=0, [5] ReservedBoxPresent=0
//   → 0x00
//
// Full 20-byte body (Python-verified):
//   00 00 BB 80   (DTSSamplingFrequency = 48000)
//   00 17 06 88   (maxBitrate = 1_509_000 = 0x170688)
//   00 0B 83 44   (avgBitrate = 754_500   = 0x0B8344)
//   10             (pcmSampleDepth = 16)
//   45 24 20 03   (packed word)
//   00 03          (ChannelLayout)
//   00             (flags)

#[rustfmt::skip]
const DDTS_ORACLE: [u8; DDTS_BODY_LEN] = [
    0x00, 0x00, 0xBB, 0x80, // DTSSamplingFrequency = 48000
    0x00, 0x17, 0x06, 0x88, // maxBitrate = 1_509_000 = 0x170688
    0x00, 0x0B, 0x83, 0x44, // avgBitrate = 754_500   = 0x0B8344
    0x10,                   // pcmSampleDepth = 16
    0x45, 0x24, 0x20, 0x03, // packed word: FD=1 SC=2 LFE=1 CL=9 CS=0x200 SD=0 RT=3
    0x00, 0x03,             // ChannelLayout = 0x0003
    0x00,                   // flags: MultiAsset=0 LBR=0 RsvdBox=0 Rsvd=0
];

fn spec_vector() -> DtsSpecificBox {
    DtsSpecificBox {
        dts_sampling_frequency: 48_000,
        max_bitrate: 1_509_000,
        avg_bitrate: 754_500,
        pcm_sample_depth: 16,
        frame_duration: 1,
        stream_construction: 2,
        core_lfe_present: true,
        core_layout: 9,
        core_size: 0x0200,
        stereo_downmix: false,
        representation_type: 3,
        channel_layout: 0x0003,
        multi_asset_flag: false,
        lbr_duration_mod: false,
        reserved_box_present: false,
    }
}

// ---------------------------------------------------------------------------
// Gate 1: serialize → byte-identical to hand-computed oracle
// ---------------------------------------------------------------------------

#[test]
fn ddts_serialize_matches_oracle() {
    let ddts = spec_vector();
    let mut buf = [0u8; DDTS_BODY_LEN];
    ddts.serialize_into(&mut buf).unwrap();
    assert_eq!(
        &buf[..],
        &DDTS_ORACLE[..],
        "DDTSSpecificBox serialize does not match hand-computed spec vector"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: parse → byte-identical round-trip
// ---------------------------------------------------------------------------

#[test]
fn ddts_parse_round_trip() {
    let parsed = DtsSpecificBox::parse(&DDTS_ORACLE).unwrap();

    // Check all fields individually (proves no raw-passthrough).
    assert_eq!(parsed.dts_sampling_frequency, 48_000);
    assert_eq!(parsed.max_bitrate, 1_509_000);
    assert_eq!(parsed.avg_bitrate, 754_500);
    assert_eq!(parsed.pcm_sample_depth, 16);
    assert_eq!(parsed.frame_duration, 1);
    assert_eq!(parsed.stream_construction, 2);
    assert!(parsed.core_lfe_present);
    assert_eq!(parsed.core_layout, 9);
    assert_eq!(parsed.core_size, 0x0200);
    assert!(!parsed.stereo_downmix);
    assert_eq!(parsed.representation_type, 3);
    assert_eq!(parsed.channel_layout, 0x0003);
    assert!(!parsed.multi_asset_flag);
    assert!(!parsed.lbr_duration_mod);
    assert!(!parsed.reserved_box_present);

    // Re-serialize → byte-identical.
    let mut buf = [0u8; DDTS_BODY_LEN];
    parsed.serialize_into(&mut buf).unwrap();
    assert_eq!(
        &buf[..],
        &DDTS_ORACLE[..],
        "re-serialized bytes do not match oracle"
    );
}

// ---------------------------------------------------------------------------
// Gate 3: parse == build from fields (proves symmetric contract)
// ---------------------------------------------------------------------------

#[test]
fn ddts_parse_equals_built() {
    let parsed = DtsSpecificBox::parse(&DDTS_ORACLE).unwrap();
    let built = spec_vector();
    assert_eq!(
        parsed, built,
        "parsed DtsSpecificBox must equal spec_vector()"
    );
}

// ---------------------------------------------------------------------------
// Gate 4: mutation → bytes change (proves no raw-passthrough serialize)
// ---------------------------------------------------------------------------

#[test]
fn ddts_mutation_changes_bytes() {
    let orig = spec_vector();
    let mut buf_orig = [0u8; DDTS_BODY_LEN];
    orig.serialize_into(&mut buf_orig).unwrap();

    // Mutate pcmSampleDepth: 16 → 24.
    let mut m1 = orig.clone();
    m1.pcm_sample_depth = 24;
    let mut buf_m1 = [0u8; DDTS_BODY_LEN];
    m1.serialize_into(&mut buf_m1).unwrap();
    assert_ne!(
        &buf_orig[..],
        &buf_m1[..],
        "pcm_sample_depth mutation must change bytes"
    );

    // Mutate CoreLFEPresent: true → false.
    let mut m2 = orig.clone();
    m2.core_lfe_present = false;
    let mut buf_m2 = [0u8; DDTS_BODY_LEN];
    m2.serialize_into(&mut buf_m2).unwrap();
    assert_ne!(
        &buf_orig[..],
        &buf_m2[..],
        "core_lfe_present mutation must change bytes"
    );

    // Mutate ChannelLayout.
    let mut m3 = orig.clone();
    m3.channel_layout = 0x00FF;
    let mut buf_m3 = [0u8; DDTS_BODY_LEN];
    m3.serialize_into(&mut buf_m3).unwrap();
    assert_ne!(
        &buf_orig[..],
        &buf_m3[..],
        "channel_layout mutation must change bytes"
    );

    // Mutate MultiAssetFlag: false → true.
    let mut m4 = orig.clone();
    m4.multi_asset_flag = true;
    let mut buf_m4 = [0u8; DDTS_BODY_LEN];
    m4.serialize_into(&mut buf_m4).unwrap();
    assert_ne!(
        &buf_orig[..],
        &buf_m4[..],
        "multi_asset_flag mutation must change bytes"
    );
}

// ---------------------------------------------------------------------------
// Gate 5: boundary — two-element CoreSize range check
// ---------------------------------------------------------------------------

#[test]
fn ddts_core_size_boundary() {
    let mut ddts = spec_vector();

    // Min value: 0
    ddts.core_size = 0;
    let mut buf = [0u8; DDTS_BODY_LEN];
    ddts.serialize_into(&mut buf).unwrap();
    let parsed = DtsSpecificBox::parse(&buf).unwrap();
    assert_eq!(parsed.core_size, 0);

    // Max value: 0x3FFF (14-bit mask)
    ddts.core_size = 0x3FFF;
    ddts.serialize_into(&mut buf).unwrap();
    let parsed2 = DtsSpecificBox::parse(&buf).unwrap();
    assert_eq!(parsed2.core_size, 0x3FFF);
}

// ---------------------------------------------------------------------------
// Gate 6: build_init_segment with CodecConfig::Dts → parse moov → dtsc ddts
// ---------------------------------------------------------------------------

/// Find a box body by FourCC within arbitrary nested box bytes.
/// For audio sample entries, skips the 28-byte AudioSampleEntry fixed prefix.
fn find_ddts_body(data: &[u8]) -> Option<&[u8]> {
    const AUDIO_ENTRY_FIXED: usize = 28;

    for item in box_iter(data) {
        let (bx, _) = item.ok()?;
        // If this is a DTS sample entry, iterate config boxes after fixed prefix.
        if matches!(&bx.header.box_type.0, b"dtsc" | b"dtsh" | b"dtsl" | b"dtse") {
            let config = &bx.body[AUDIO_ENTRY_FIXED.min(bx.body.len())..];
            for ch in box_iter(config) {
                let (cb, _) = ch.ok()?;
                if &cb.header.box_type.0 == b"ddts" {
                    return Some(cb.body);
                }
            }
        }
        // Recurse into containers.
        if let Some(found) = find_ddts_body(bx.body) {
            return Some(found);
        }
    }
    None
}

#[test]
fn dtsc_init_segment_ddts_round_trip() {
    let ddts = spec_vector();
    let sample_rate: u32 = 48_000;

    let tracks = vec![TrackSpec {
        track_id: 1,
        timescale: sample_rate,
        config: CodecConfig::Dts {
            config: ddts.clone(),
            codec_fourcc: DTSC_FOURCC,
            channel_count: 2,
            sample_rate,
            sample_size: 16,
        },
    }];

    let init = build_init_segment(&tracks, 1000).unwrap();

    // Navigate into moov → trak → mdia → minf → stbl → stsd → dtsc → ddts.
    let ddts_body = find_ddts_body(&init).expect("ddts box must be present in init segment");

    // Byte-exact match with the oracle.
    assert_eq!(
        ddts_body,
        &DDTS_ORACLE[..],
        "ddts body in init segment must match oracle"
    );

    // Re-parse the ddts body → equal to the original.
    let re_parsed = DtsSpecificBox::parse(ddts_body).unwrap();
    assert_eq!(
        re_parsed, ddts,
        "ddts re-parsed from init segment must equal original"
    );
}

// ---------------------------------------------------------------------------
// Gate 7: SampleEntryVariant::Dts parses back from the stsd bytes
// ---------------------------------------------------------------------------

#[test]
fn stsd_round_trip_dts_entry() {
    use transmux::{MovieBox, StblChild};

    let ddts = spec_vector();
    let sample_rate: u32 = 48_000;

    let tracks = vec![TrackSpec {
        track_id: 1,
        timescale: sample_rate,
        config: CodecConfig::Dts {
            config: ddts.clone(),
            codec_fourcc: DTSC_FOURCC,
            channel_count: 2,
            sample_rate,
            sample_size: 16,
        },
    }];

    let init = build_init_segment(&tracks, 1000).unwrap();

    // Parse the moov.
    let moov = {
        let mut off = 0usize;
        let mut found = None;
        while off + 8 <= init.len() {
            let sz = u32::from_be_bytes([init[off], init[off + 1], init[off + 2], init[off + 3]])
                as usize;
            if sz < 8 {
                break;
            }
            if &init[off + 4..off + 8] == b"moov" {
                found = Some(&init[off..off + sz]);
                break;
            }
            off += sz;
        }
        MovieBox::parse(found.expect("moov must be present in init")).unwrap()
    };

    // Navigate to stsd.
    let trak = &moov.tracks[0];
    let stbl = trak
        .mdia
        .as_ref()
        .unwrap()
        .minf
        .as_ref()
        .unwrap()
        .stbl
        .as_ref()
        .unwrap();
    let stsd = stbl
        .children
        .iter()
        .find_map(|c| {
            if let StblChild::Stsd(s) = c {
                Some(s)
            } else {
                None
            }
        })
        .expect("stbl must contain stsd");

    // The first entry must be SampleEntryVariant::Dts.
    match stsd
        .entries
        .first()
        .expect("stsd must have at least one entry")
    {
        SampleEntryVariant::Dts(dts_entry) => {
            assert_eq!(dts_entry.channelcount, 2);
            assert_eq!(dts_entry.samplesize, 16);
            // ddts config box must be present.
            let has_ddts = dts_entry
                .config_boxes
                .iter()
                .any(|b| &b.box_type == b"ddts");
            assert!(has_ddts, "DtsSampleEntry must contain a ddts child box");
        }
        other => panic!(
            "expected SampleEntryVariant::Dts, got {:?}",
            core::mem::discriminant(other)
        ),
    }
}

// ---------------------------------------------------------------------------
// Gate 8: rfc6381 strings
// ---------------------------------------------------------------------------

#[test]
fn rfc6381_codes() {
    assert_eq!(DtsSpecificBox::rfc6381(b"dtsc"), "dtsc");
    assert_eq!(DtsSpecificBox::rfc6381(b"dtsh"), "dtsh");
    assert_eq!(DtsSpecificBox::rfc6381(b"dtsl"), "dtsl");
    assert_eq!(DtsSpecificBox::rfc6381(b"dtse"), "dtse");
}

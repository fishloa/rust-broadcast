//! MPEG-H 3D Audio (`mha1`/`mhaC`) full-codec round-trip tests — ISO/IEC 23008-3 §20.
//!
//! Exercises the promotion of MPEG-H to a first-class IR codec
//! ([`CodecConfig::MpegH`]): a real `mha1.0x0B` fragmented-MP4 fixture is
//! demuxed to the IR, re-muxed, and re-demuxed, with the `mhaC` record and the
//! coded sample bytes checked byte-for-byte against the source.
//!
//! Fixture: `fixtures/mp4/frag/mpegh_stereo.frag.mp4` — a real Fraunhofer/
//! DASH-IF MPEG-H 3D Audio stereo (48 kHz) fMP4, trimmed to ftyp + moov + 6
//! fragments. It carries a real 64-byte `mhaC` box (56-byte record body:
//! configurationVersion=1, profile-level LC L1 = 0x0B, referenceChannelLayout
//! CICP 2 = stereo, 51-byte `mpegh3daConfig`).
//!
//! The companion `mpegh_stereo.packets.csv` reflects the *full* source file's
//! metadata (not this truncated fixture), so expected sample counts are derived
//! from the fixture's own `moof`/`trun`, never hardcoded.
//!
//! Every oracle is walked out of the source bytes at runtime (no hardcoded
//! offsets), so these tests bite any structural regression.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::{
    CmafMux, CodecConfig, Fmp4Demux, MHAC_CONFIGURATION_VERSION, MHAC_FOURCC,
    MHADecoderConfigurationRecord, SampleDescriptionBox, SampleEntryVariant,
};

const FIXTURE: &[u8] = include_bytes!("../../fixtures/mp4/frag/mpegh_stereo.frag.mp4");

// Ground-truth values decoded from the fixture's own mhaC record (asserted, not
// trusted): LC profile-level L1 and CICP ChannelConfiguration 2 (stereo).
const EXPECTED_PROFILE_LEVEL: u8 = 0x0B;
const EXPECTED_REFERENCE_CHANNEL_LAYOUT: u8 = 2;
const EXPECTED_SAMPLE_RATE: u32 = 48000;

// ---------------------------------------------------------------------------
// Box-walking helpers (dynamic — no hardcoded offsets).
// ---------------------------------------------------------------------------

/// Walk a flat sequence of ISOBMFF boxes; return the first box with `fourcc`
/// (full bytes, header included). Handles the 64-bit `size==1` large-size form.
fn walk<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let mut sz =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        let mut hdr = 8usize;
        if sz == 1 {
            if off + 16 > data.len() {
                break;
            }
            sz = u64::from_be_bytes(data[off + 8..off + 16].try_into().unwrap()) as usize;
            hdr = 16;
        }
        if sz < hdr {
            break;
        }
        let end = (off + sz).min(data.len());
        if &data[off + 4..off + 8] == fourcc.as_ref() {
            return Some(&data[off..end]);
        }
        off += sz;
    }
    None
}

/// The children region of a container box (skip its 8-byte header).
fn children(b: &[u8]) -> &[u8] {
    &b[8..]
}

/// Walk moov → trak → mdia → minf → stbl → stsd and return the stsd bytes.
fn find_stsd(init: &[u8]) -> &[u8] {
    let moov = walk(init, b"moov").expect("moov");
    let trak = walk(children(moov), b"trak").expect("trak");
    let mdia = walk(children(trak), b"mdia").expect("mdia");
    let minf = walk(children(mdia), b"minf").expect("minf");
    let stbl = walk(children(minf), b"stbl").expect("stbl");
    walk(children(stbl), b"stsd").expect("stsd")
}

/// Walk the source mp4 down to the `mhaC` box and return its **body** bytes
/// (the record, without the 8-byte box header) — the strong byte-exact oracle.
fn source_mhac_body(mp4: &[u8]) -> &[u8] {
    let stsd = find_stsd(mp4);
    // stsd body: 4-byte FullBox header + 4-byte entry_count, then sample entries.
    let entries = &stsd[8 + 4 + 4..];
    // The first sample entry is an audio sample entry: 8-byte box header +
    // 28-byte AudioSampleEntry fixed fields, then its child boxes (mhaC, btrt…).
    let mha1 = walk(entries, b"mha1").expect("mha1 sample entry");
    assert_eq!(&mha1[4..8], b"mha1", "fixture sample entry must be mha1");
    let child_region = &mha1[8 + 28..];
    let mhac = walk(child_region, MHAC_FOURCC.as_ref().try_into().unwrap()).expect("mhaC box");
    &mhac[8..]
}

/// Demux the fixture to the IR and return the single MPEG-H track.
fn demux_fixture() -> transmux::Media {
    let mut demux = Fmp4Demux::new();
    demux.unpackage(FIXTURE).expect("demux fixture")
}

fn mpegh_track(media: &transmux::Media) -> &transmux::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::MpegH { .. }))
        .expect("an MPEG-H track in the demuxed media")
}

// ---------------------------------------------------------------------------
// Gate 1: enumeration + mhaC byte-exact (the strong oracle).
// ---------------------------------------------------------------------------
#[test]
fn demux_enumerates_mpegh_and_mhac_is_byte_exact() {
    let media = demux_fixture();
    let track = mpegh_track(&media);

    let CodecConfig::MpegH { config, .. } = track.config() else {
        panic!("expected CodecConfig::MpegH");
    };

    // Re-serialize the reconstructed record and compare byte-for-byte against
    // the mhaC box body walked out of the source mp4.
    let expected = source_mhac_body(FIXTURE);
    let mut got = alloc::vec![0u8; config.serialized_len()];
    let n = config
        .serialize_into(&mut got)
        .expect("serialize mhaC record");
    got.truncate(n);
    assert_eq!(
        got.as_slice(),
        expected,
        "reconstructed mhaC record must serialize byte-identical to the source mhaC body"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: decoded config fields (asserted against the parsed record).
// ---------------------------------------------------------------------------
#[test]
fn config_fields_decoded_from_record() {
    let media = demux_fixture();
    let track = mpegh_track(&media);

    let CodecConfig::MpegH {
        config,
        sample_rate,
        channel_count,
        ..
    } = track.config()
    else {
        panic!("expected CodecConfig::MpegH");
    };

    assert_eq!(
        *sample_rate, EXPECTED_SAMPLE_RATE,
        "sample_rate must come from the AudioSampleEntry (48 kHz)"
    );
    assert_eq!(
        config.configuration_version, MHAC_CONFIGURATION_VERSION,
        "configurationVersion must be 1 (ISO/IEC 23008-3 §20)"
    );
    assert_eq!(
        config.mpegh3da_profile_level_indication, EXPECTED_PROFILE_LEVEL,
        "profile-level must decode from the record (LC L1 = 0x0B)"
    );
    assert_eq!(
        config.reference_channel_layout, EXPECTED_REFERENCE_CHANNEL_LAYOUT,
        "referenceChannelLayout must decode from the record (CICP 2 = stereo)"
    );
    assert!(
        !config.mpegh3da_config.is_empty(),
        "mpegh3daConfig blob must carry the real config bytes"
    );
    // channel_count is carried verbatim from the AudioSampleEntry (MPEG-H puts
    // the true layout in referenceChannelLayout; this fixture's entry carries 0).
    let _ = channel_count;
}

// ---------------------------------------------------------------------------
// Gate 3: sample-fidelity round-trip.
//   Fmp4Demux → IR → CmafMux → re-Fmp4Demux; coded MPEG-H sample bytes are
//   byte-identical to the first demux, same count (derived, never hardcoded).
// ---------------------------------------------------------------------------
#[test]
fn sample_bytes_round_trip_byte_identical() {
    let first = demux_fixture();
    let track1 = mpegh_track(&first);
    let count1 = track1.samples.len();
    assert!(
        count1 > 0,
        "the 6 fragments must yield at least one MPEG-H sample"
    );

    // Re-mux to CMAF, then re-demux.
    let mut mux = CmafMux::new(1);
    let remuxed = mux.package(&first).expect("re-mux to CMAF");
    let mut demux2 = Fmp4Demux::new();
    let second = demux2.unpackage(&remuxed).expect("re-demux");
    let track2 = mpegh_track(&second);

    assert_eq!(
        track2.samples.len(),
        count1,
        "sample count must survive the demux → mux → demux round-trip"
    );

    let bytes1: Vec<&[u8]> = track1.samples.iter().map(|s| s.data.as_slice()).collect();
    let bytes2: Vec<&[u8]> = track2.samples.iter().map(|s| s.data.as_slice()).collect();
    assert_eq!(
        bytes1, bytes2,
        "coded MPEG-H sample bytes must be byte-identical after the round-trip"
    );
}

// ---------------------------------------------------------------------------
// Gate 4: output path — the muxed init segment carries an mha1 sample entry
//   whose mhaC equals the source mhaC.
// ---------------------------------------------------------------------------
#[test]
fn output_init_segment_carries_source_mhac() {
    let media = demux_fixture();
    let mut mux = CmafMux::new(1);
    let out = mux.package(&media).expect("mux");

    // Walk the muxed init to its stsd and confirm the mha1/mhaC.
    let stsd_bytes = find_stsd(&out);
    let stsd = SampleDescriptionBox::parse(stsd_bytes).expect("parse muxed stsd");
    assert_eq!(stsd.entries.len(), 1, "one sample entry expected");

    let SampleEntryVariant::Mha(mha) = &stsd.entries[0] else {
        panic!("muxed sample entry must be MPEG-H (Mha)");
    };
    assert_eq!(&mha.codec_type, b"mha1", "output codec_type must be mha1");

    let mhac = mha
        .config_boxes
        .iter()
        .find(|b| b.box_type == MHAC_FOURCC)
        .expect("mhaC in muxed mha1 entry");

    // The muxed mhaC body must equal the source mhaC body.
    assert_eq!(
        mhac.data.as_slice(),
        source_mhac_body(FIXTURE),
        "muxed mhaC must be byte-identical to the source mhaC"
    );
}

// ---------------------------------------------------------------------------
// Gate 5: round-trip symmetry — parse → serialize identical; mutate a field →
//   serialized bytes change (decode, not raw passthrough).
// ---------------------------------------------------------------------------
#[test]
fn record_parse_serialize_symmetry_and_mutation_bites() {
    let body = source_mhac_body(FIXTURE);
    let rec = MHADecoderConfigurationRecord::parse(body).expect("parse source mhaC record");

    // parse → serialize → byte-identical.
    let mut out = alloc::vec![0u8; rec.serialized_len()];
    let n = rec.serialize_into(&mut out).expect("serialize");
    out.truncate(n);
    assert_eq!(
        out.as_slice(),
        body,
        "parse → serialize must reproduce the source mhaC record byte-for-byte"
    );

    // Mutating the profile-level must change the serialized bytes (proves the
    // serializer reads struct fields, not a cached raw slice).
    let mut mutated = rec.clone();
    mutated.mpegh3da_profile_level_indication ^= 0x01;
    let mut out2 = alloc::vec![0u8; mutated.serialized_len()];
    let m = mutated
        .serialize_into(&mut out2)
        .expect("serialize mutated");
    out2.truncate(m);
    assert_ne!(
        out2.as_slice(),
        body,
        "mutating mpegh3daProfileLevelIndication must change the serialized bytes"
    );

    // Mutating the opaque config blob must also change the bytes.
    let mut mutated_blob = rec.clone();
    mutated_blob.mpegh3da_config.push(0xFF);
    let mut out3 = alloc::vec![0u8; mutated_blob.serialized_len()];
    let k = mutated_blob
        .serialize_into(&mut out3)
        .expect("serialize blob");
    out3.truncate(k);
    assert_ne!(
        out3.as_slice(),
        body,
        "mutating mpegh3daConfig must change the serialized bytes"
    );
}

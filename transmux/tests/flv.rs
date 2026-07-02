//! FLV (Flash Video) spoke integration tests (issue #513).
//!
//! Exercises [`transmux::FlvDemux`] ([`Unpackage`]) and [`transmux::FlvMux`]
//! ([`Package`]) against the committed real fixture `fixtures/flv/av.flv`
//! (H.264 + AAC) and its ffprobe oracle `fixtures/flv/av.packets.csv`.
//!
//! Gates (each bites — see the per-test comments for what a regression breaks):
//! 1. Enumeration: 2 tracks, AVC 320×240 + AAC.
//! 2. avcC + ASC: avcC byte-identical to the same-source `.ref.mp4`; ASC rate/channels.
//! 3. Timestamp/keyframe oracle: 75 video + 131 audio; per-sample PTS/DTS vs CSV.
//! 4. Sample fidelity + FLV round-trip: demux→mux→demux byte-identical AUs.
//! 5. Cross-hub: FLV → IR → CmafMux carries avc1/avcC + mp4a and matching NALs.

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::init_segment::{MovieBox, SampleEntryVariant, StblChild};
use transmux::{CmafMux, CodecConfig, FlvDemux, FlvMux};

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");
const CSV: &str = include_str!("../../fixtures/flv/av.packets.csv");
const REF_MP4: &[u8] = include_bytes!("../../fixtures/ts/demux-oracle/h264_aac.ref.mp4");

/// One oracle packet row.
#[derive(Debug, Clone, Copy)]
struct Pkt {
    is_video: bool,
    pts: i64,
    dts: i64,
    size: usize,
    keyframe: bool,
}

fn oracle() -> Vec<Pkt> {
    CSV.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split(',').collect();
            Pkt {
                is_video: c[0] == "video",
                pts: c[2].parse().unwrap(),
                dts: c[3].parse().unwrap(),
                size: c[5].parse().unwrap(),
                keyframe: c[6] == "1",
            }
        })
        .collect()
}

/// Walk a top-level box by four-CC in a byte stream.
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        let ty = &data[off + 4..off + 8];
        let end = if size == 0 { data.len() } else { off + size };
        if ty == fourcc {
            return Some(&data[off..end]);
        }
        if size < 8 {
            break;
        }
        off = end;
    }
    None
}

/// Walk the `moov` of an fMP4 and return the video track's `avcC` box body
/// (serialized decoder-config-record bytes, after the 8-byte box header).
fn ref_mp4_avcc(mp4: &[u8]) -> Vec<u8> {
    let moov = find_top_box(mp4, b"moov").expect("moov in ref.mp4");
    let movie = MovieBox::parse(moov).expect("parse ref.mp4 moov");
    for trak in &movie.tracks {
        let Some(stbl) = trak
            .mdia
            .as_ref()
            .and_then(|m| m.minf.as_ref())
            .and_then(|m| m.stbl.as_ref())
        else {
            continue;
        };
        let Some(stsd) = stbl.children.iter().find_map(|c| match c {
            StblChild::Stsd(s) => Some(s),
            _ => None,
        }) else {
            continue;
        };
        if let Some(SampleEntryVariant::Avc1(avc1)) = stsd.entries.first() {
            let mut body = vec![0u8; avc1.config.config.serialized_len()];
            let n = avc1.config.config.serialize_into(&mut body).unwrap();
            body.truncate(n);
            return body;
        }
    }
    panic!("no avc1 sample entry in ref.mp4");
}

// ---------------------------------------------------------------------------
// Test 1 — Enumeration
// ---------------------------------------------------------------------------

#[test]
fn enumerate_two_tracks_avc_320x240_and_aac() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv");

    assert_eq!(media.tracks.len(), 2, "must enumerate 2 tracks (AVC + AAC)");

    match media.tracks[0].config() {
        CodecConfig::Avc { width, height, .. } => {
            // Bites: if the SPS is not decoded from the avcC, dims are 0.
            assert_eq!((*width, *height), (320, 240), "AVC dims from SPS");
        }
        other => panic!("track 0 must be AVC, got {other:?}"),
    }
    assert!(
        matches!(media.tracks[1].config(), CodecConfig::Aac { .. }),
        "track 1 must be AAC"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — avcC + ASC
// ---------------------------------------------------------------------------

#[test]
fn avcc_matches_ref_mp4_and_asc_decodes() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv");

    // avcC from the FLV-demuxed config, serialized to its box body.
    let CodecConfig::Avc { config, .. } = media.tracks[0].config() else {
        panic!("track 0 must be AVC");
    };
    let mut flv_avcc = alloc_body(config.config.serialized_len());
    let n = config.config.serialize_into(&mut flv_avcc).unwrap();
    flv_avcc.truncate(n);

    // avcC walked from the same-source .ref.mp4 (byte-identical decoder config).
    let ref_avcc = ref_mp4_avcc(REF_MP4);
    // Bites: any drift in profile/level/SPS/PPS bytes breaks this equality.
    assert_eq!(
        flv_avcc, ref_avcc,
        "FLV-demuxed avcC must be byte-identical to the ref.mp4 avcC"
    );

    // ASC channels/rate decode correctly.
    let CodecConfig::Aac {
        esds,
        channel_count,
        sample_rate,
        ..
    } = media.tracks[1].config()
    else {
        panic!("track 1 must be AAC");
    };
    let asc_bytes = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .unwrap()
        .decoder_specific_info
        .as_ref()
        .unwrap()
        .data
        .clone();
    let asc = transmux::AudioSpecificConfig::parse(&asc_bytes).expect("parse ASC");
    // The ASC (0x12 0x08) is the authority: AAC-LC, SFI 4 (44100 Hz), 1 channel.
    assert_eq!(asc.channel_configuration.raw(), 1, "ASC channels = 1");
    assert_eq!(*channel_count, 1, "config channel_count = 1");
    assert_eq!(*sample_rate, 44100, "ASC sample rate = 44100 Hz");
}

// ---------------------------------------------------------------------------
// Test 3 — Timestamp / keyframe oracle
// ---------------------------------------------------------------------------

#[test]
fn timestamps_and_keyframes_match_oracle() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv");

    let ora = oracle();
    let vid_ora: Vec<Pkt> = ora.iter().copied().filter(|p| p.is_video).collect();
    let aud_ora: Vec<Pkt> = ora.iter().copied().filter(|p| !p.is_video).collect();

    let vid = &media.tracks[0].samples;
    let aud = &media.tracks[1].samples;
    // Bites: wrong tag typing / dropped frames changes counts.
    assert_eq!(vid.len(), 75, "video sample count");
    assert_eq!(aud.len(), 131, "audio sample count");
    assert_eq!(vid_ora.len(), 75);
    assert_eq!(aud_ora.len(), 131);

    // Reconstruct DTS from the forward-delta durations, relative to the track's
    // first DTS (the IR carries relative timing; the oracle is absolute so we
    // compare each against the oracle's own first-DTS baseline — this bites on
    // every per-sample delta AND the composition offset).
    check_timing(vid, &vid_ora, "video");
    check_timing(aud, &aud_ora, "audio");

    // Video keyframe flag = FLV FrameType==1. Oracle has exactly the same set.
    for (i, (s, p)) in vid.iter().zip(&vid_ora).enumerate() {
        assert_eq!(s.is_sync, p.keyframe, "video sample {i} keyframe flag");
    }
    // Bites: keyframes present at all + the right count (3 per the oracle).
    let kf = vid.iter().filter(|s| s.is_sync).count();
    assert_eq!(kf, 3, "exactly 3 video keyframes");
}

fn check_timing(samples: &[transmux::Sample], ora: &[Pkt], kind: &str) {
    let base_dts = ora[0].dts;
    let mut dts = 0i64; // relative to track start
    for (i, (s, p)) in samples.iter().zip(ora).enumerate() {
        // Per-sample payload length matches the ffprobe oracle `size` column
        // (video: length-prefixed NALs; audio: raw AAC AU). Bites on any
        // off-by-one in tag body slicing.
        assert_eq!(s.data.len(), p.size, "{kind} sample {i} payload size");
        let exp_dts_rel = p.dts - base_dts;
        assert_eq!(dts, exp_dts_rel, "{kind} sample {i} DTS (relative)");
        // PTS = DTS + composition offset.
        let exp_pts_rel = p.pts - base_dts;
        assert_eq!(
            dts + s.composition_offset as i64,
            exp_pts_rel,
            "{kind} sample {i} PTS (= DTS + composition offset)"
        );
        // Advance by the forward-delta duration (exact for all but the last).
        if i + 1 < samples.len() {
            dts += s.duration as i64;
        }
    }
}

// ---------------------------------------------------------------------------
// Test 4 — Sample fidelity + FLV round-trip
// ---------------------------------------------------------------------------

#[test]
fn flv_round_trip_preserves_samples_and_timing() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv");

    let mut mux = FlvMux::new();
    let flv2 = mux.package(&media).expect("mux to FLV");

    let mut demux2 = FlvDemux::new();
    let media2 = demux2.unpackage(&flv2).expect("re-demux FLV");

    assert_eq!(media2.tracks.len(), 2, "round-trip track count");
    for (a, b) in media.tracks.iter().zip(&media2.tracks) {
        assert_eq!(
            a.samples.len(),
            b.samples.len(),
            "track {} sample count preserved",
            a.track_id()
        );
        // Bites: raw-passthrough mux would drop the AVCPacketType/CompositionTime
        // framing; here we require the NAL / AAC payload bytes to survive.
        for (i, (sa, sb)) in a.samples.iter().zip(&b.samples).enumerate() {
            assert_eq!(sa.data, sb.data, "track {} sample {i} bytes", a.track_id());
            assert_eq!(
                sa.composition_offset,
                sb.composition_offset,
                "track {} sample {i} composition offset",
                a.track_id()
            );
            assert_eq!(
                sa.is_sync,
                sb.is_sync,
                "track {} sample {i} sync flag",
                a.track_id()
            );
        }
    }

    // Video NAL payloads (all 75) survive byte-identically; each is 4-byte
    // length-prefixed and self-consistent.
    let vid = &media.tracks[0].samples;
    assert_eq!(vid.len(), 75);
    for s in vid {
        // Length-prefixed NALs must sum exactly to the sample length.
        let nals = transmux::iter_length_prefixed_nals(&s.data).expect("length-prefixed NALs");
        let total: usize = nals.iter().map(|n| n.len() + 4).sum();
        assert_eq!(
            total,
            s.data.len(),
            "video sample is well-formed length-prefixed"
        );
    }
    assert_eq!(media.tracks[1].samples.len(), 131);
}

// ---------------------------------------------------------------------------
// Test 5 — Cross-hub: FLV → IR → CmafMux
// ---------------------------------------------------------------------------

#[test]
fn cross_hub_flv_to_cmaf() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv");
    let flv_nals: Vec<Vec<u8>> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect();

    let mut cmaf = CmafMux::new(1);
    let seg = cmaf.package(&media).expect("CMAF package");

    // Init moov must carry avc1/avcC (video) and mp4a (audio).
    let moov = find_top_box(&seg, b"moov").expect("moov in CMAF");
    let movie = MovieBox::parse(moov).expect("parse moov");
    assert_eq!(movie.tracks.len(), 2, "CMAF moov has 2 tracks");

    let mut saw_avc1 = false;
    let mut saw_mp4a = false;
    for trak in &movie.tracks {
        let stbl = trak
            .mdia
            .as_ref()
            .and_then(|m| m.minf.as_ref())
            .and_then(|m| m.stbl.as_ref())
            .expect("stbl");
        let stsd = stbl
            .children
            .iter()
            .find_map(|c| match c {
                StblChild::Stsd(s) => Some(s),
                _ => None,
            })
            .expect("stsd");
        match stsd.entries.first().expect("entry") {
            SampleEntryVariant::Avc1(avc1) => {
                saw_avc1 = true;
                // avcC present inside the avc1 sample entry.
                assert!(!avc1.config.config.sps.is_empty(), "avcC has SPS");
            }
            SampleEntryVariant::Mp4a(_) => saw_mp4a = true,
            _ => {}
        }
    }
    assert!(saw_avc1, "CMAF must carry avc1/avcC");
    assert!(saw_mp4a, "CMAF must carry mp4a");

    // The video NAL payloads in the CMAF mdat equal the FLV-demuxed ones (the
    // IR passed them through unchanged: CmafMux copies Sample.data into mdat).
    let mdat = find_top_box(&seg, b"mdat").expect("mdat in CMAF");
    let mdat_body = &mdat[8..];
    // The first FLV video NAL sample must appear verbatim at the mdat head
    // (video is emitted first in track order by CmafMux).
    assert!(
        mdat_body.starts_with(&flv_nals[0]),
        "first FLV video sample NAL bytes appear verbatim in the CMAF mdat"
    );
}

/// Allocate a zeroed serialization buffer.
fn alloc_body(len: usize) -> Vec<u8> {
    vec![0u8; len]
}

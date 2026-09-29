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
use bytes::Bytes;
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
        assert_eq!(
            s.flags.is_sync, p.keyframe,
            "video sample {i} keyframe flag"
        );
    }
    // Bites: keyframes present at all + the right count (3 per the oracle).
    let kf = vid.iter().filter(|s| s.flags.is_sync).count();
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
            dts + s.composition_offset() as i64,
            exp_pts_rel,
            "{kind} sample {i} PTS (= DTS + composition offset)"
        );
        // Advance by the forward-delta duration (exact for all but the last).
        if i + 1 < samples.len() {
            dts += s.duration.unwrap_or(0) as i64;
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
                sa.composition_offset(),
                sb.composition_offset(),
                "track {} sample {i} composition offset",
                a.track_id()
            );
            assert_eq!(
                sa.flags.is_sync,
                sb.flags.is_sync,
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
    let flv_nals: Vec<Bytes> = media.tracks[0]
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

// ---------------------------------------------------------------------------
// Test 6 — streaming RTMP payloads (issue #934): FLV-shaped, not TS-shaped
// ---------------------------------------------------------------------------

/// `transmux::flv_sequence_header_payloads`/`flv_frame_payloads` are the API
/// `multimux`'s RTMP push output uses (issue #934) instead of muxing every
/// push output with `TsMux` and shipping raw MPEG-2 TS as an RTMP video
/// message — a payload no RTMP server can decode. Asserts the actual byte
/// **shape**: an RTMP `send_video` payload must look like FLV `VIDEODATA`
/// (`FrameType`/`CodecID` nibble + `AVCPacketType`), never start with the TS
/// sync byte `0x47` (0x47's nibbles — `4`/`7` — aren't a `FrameType` this
/// crate emits (1/2) or `CodecID` 7's low nibble alone, so this is a precise
/// negative check, not a coincidence of one byte value).
#[test]
fn streaming_payloads_are_flv_shaped_not_ts_shaped() {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("demux av.flv");

    // --- Sequence headers: sent once, up front ---
    let headers = transmux::flv_sequence_header_payloads(&media).expect("build sequence headers");
    assert_eq!(headers.len(), 2, "one video + one audio sequence header");
    let vid_hdr = headers
        .iter()
        .find(|p| p.kind == transmux::FlvPayloadKind::Video)
        .expect("video sequence header present");
    let aud_hdr = headers
        .iter()
        .find(|p| p.kind == transmux::FlvPayloadKind::Audio)
        .expect("audio sequence header present");

    // VideoTagHeader: FrameType=keyframe(1)<<4 | CodecID=AVC(7) = 0x17;
    // AVCPacketType=SEQUENCE_HEADER(0). THE BITE: the old defect (TsMux
    // output as an RTMP video payload) starts with `0x47` (TS sync byte),
    // never `0x17`.
    assert_eq!(
        vid_hdr.body[0], 0x17,
        "video seq header FrameType/CodecID byte"
    );
    assert_ne!(vid_hdr.body[0], 0x47, "must not be a TS sync byte");
    assert_eq!(
        vid_hdr.body[1], 0,
        "video seq header AVCPacketType = sequence header"
    );
    // avcC bytes appear verbatim after the 5-byte VideoTagHeader+AVCPacketType+CompositionTime.
    let CodecConfig::Avc { config, .. } = media.tracks[0].config() else {
        panic!("track 0 must be AVC");
    };
    let mut avcc = alloc_body(config.config.serialized_len());
    let n = config.config.serialize_into(&mut avcc).unwrap();
    avcc.truncate(n);
    assert_eq!(
        &vid_hdr.body[5..],
        &avcc[..],
        "avcC bytes verbatim in the sequence-header payload"
    );

    // AudioTagHeader (AAC/44.1kHz/16-bit/mono for this fixture) = 0xAE;
    // AACPacketType=SEQUENCE_HEADER(0).
    assert_eq!(
        aud_hdr.body[0], 0xAE,
        "audio seq header AudioTagHeader byte (mono AAC)"
    );
    assert_eq!(
        aud_hdr.body[1], 0,
        "audio seq header AACPacketType = sequence header"
    );

    // --- Per-frame payloads: sent continuously ---
    let frames = transmux::flv_frame_payloads(&media).expect("build frame payloads");
    assert_eq!(frames.len(), 75 + 131, "one payload per demuxed sample");
    let video_frames: Vec<_> = frames
        .iter()
        .filter(|p| p.kind == transmux::FlvPayloadKind::Video)
        .collect();
    let audio_frames: Vec<_> = frames
        .iter()
        .filter(|p| p.kind == transmux::FlvPayloadKind::Audio)
        .collect();
    assert_eq!(video_frames.len(), 75, "video frame payload count");
    assert_eq!(audio_frames.len(), 131, "audio frame payload count");

    for p in &video_frames {
        assert_ne!(
            p.body[0], 0x47,
            "video frame payload must not look like a TS packet"
        );
        assert_eq!(p.body[0] & 0x0F, 0x07, "video frame CodecID nibble = AVC");
        assert!(
            matches!(p.body[0] >> 4, 1 | 2),
            "video frame FrameType nibble = keyframe or inter"
        );
        assert_eq!(p.body[1], 1, "video frame AVCPacketType = NALU");
    }
    for p in &audio_frames {
        assert_ne!(
            p.body[0], 0x47,
            "audio frame payload must not look like a TS packet"
        );
        assert_eq!(p.body[0] >> 4, 0x0A, "audio frame SoundFormat nibble = AAC");
        assert_eq!(p.body[1], 1, "audio frame AACPacketType = raw AU");
    }

    // Timestamps: FLV-demuxed tracks are already at timescale 1000 (ms), so
    // `flv_frame_payloads`'s dts-based ms rescale is the identity — the
    // first video payload's `timestamp_ms` must equal that sample's own
    // absolute `dts` exactly (bites on any accidental batch-relative reset).
    let first_sample_dts = media.tracks[0].samples[0].dts.expect("video dts");
    assert_eq!(
        video_frames[0].timestamp_ms as i64, first_sample_dts,
        "first video payload ms timestamp == sample absolute dts"
    );
}

/// Allocate a zeroed serialization buffer.
fn alloc_body(len: usize) -> Vec<u8> {
    vec![0u8; len]
}

// ---------------------------------------------------------------------------
// Test 7 — FlvMux rescales track ticks to FLV's millisecond clock (r04-W12)
// ---------------------------------------------------------------------------

/// `FlvMux::package` writes each track's `Sample::duration` running sum into
/// the FLV tag `Timestamp` field, which is a **millisecond** clock (§E.4.1),
/// and `Sample::composition_offset()` into the `CompositionTime` SI24 field,
/// also milliseconds (§E.4.3.2). Both are in the track's own
/// `TrackSpec::timescale` ticks, so a non-1000 timescale must be rescaled.
///
/// Before the fix the raw tick count went straight out, so this 90 kHz track
/// (the timescale `TsDemux` produces) came back as timestamps 90× too large
/// (30 000 ms instead of 333 ms — 30 seconds of A/V drift per second) and its
/// composition offset was truncated into SI24.
#[test]
fn package_rescales_non_ms_timescales() {
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_TICKS: u32 = 3_000; // 30 fps at 90 kHz

    // One AVC `avc1` from the real fixture, so the muxer has a valid config.
    let mut demux = FlvDemux::new();
    let source = demux.unpackage(FLV).expect("demux av.flv");
    let CodecConfig::Avc { config, .. } = source.tracks[0].config() else {
        panic!("fixture track 0 must be AVC");
    };
    let config = config.clone();

    // Two video samples at 0 and 3000 ticks (= 0 ms and 33 ms), the second
    // carrying a 1-frame (33 ms) composition offset.
    let samples: Vec<transmux::Sample> = (0..2u32)
        .map(|i| {
            let dts = (i * FRAME_TICKS) as i64;
            transmux::Sample::new(
                alloc_body(4),
                Some(dts),
                Some(dts + FRAME_TICKS as i64),
                Some(FRAME_TICKS),
                true,
            )
        })
        .collect();
    let track = transmux::Track::new(
        transmux::TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 320,
                height: 240,
            },
        ),
        samples,
    );
    let media = transmux::Media::new(vec![track], VIDEO_TIMESCALE);

    let mut mux = FlvMux::new();
    let flv = mux.package(&media).expect("package 90 kHz FLV");

    // Parse the tag loop and pick out the two NALU tags' timestamps.
    let (timestamps, comp_times) = flv_nalu_tags(&flv);
    assert_eq!(timestamps.len(), 2, "two NALU tags");
    // Bites: without the rescale these are 0 and 3000 ms (90× too large).
    assert_eq!(timestamps[0], 0, "first tag timestamp in ms");
    assert_eq!(
        timestamps[1], 33,
        "second tag timestamp in ms (3000/90000 s)"
    );
    assert_eq!(comp_times[0], 33, "first CompositionTime in ms");
    assert_eq!(comp_times[1], 33, "second CompositionTime in ms");
}

/// Scan the FLV tag loop for AVC `NALU` tags, returning each tag's
/// `Timestamp` and its `CompositionTime` (both already in FLV's ms clock).
fn flv_nalu_tags(flv: &[u8]) -> (Vec<u32>, Vec<i32>) {
    const TAG_HEADER: usize = 11;
    let data_offset = u32::from_be_bytes([flv[5], flv[6], flv[7], flv[8]]) as usize;
    let mut off = data_offset.max(9) + 4;
    let (mut ts, mut comp) = (Vec::new(), Vec::new());
    while off + TAG_HEADER <= flv.len() {
        let ty = flv[off];
        let size = u32::from_be_bytes([0, flv[off + 1], flv[off + 2], flv[off + 3]]) as usize;
        let lo = u32::from_be_bytes([0, flv[off + 4], flv[off + 5], flv[off + 6]]);
        let hi = flv[off + 7] as u32;
        let body = off + TAG_HEADER;
        if body + size + 4 > flv.len() {
            break;
        }
        if ty == 9 && size >= 5 && flv[body + 1] == 1 {
            ts.push((hi << 24) | lo);
            let raw = ((flv[body + 2] as u32) << 16)
                | ((flv[body + 3] as u32) << 8)
                | flv[body + 4] as u32;
            comp.push(if raw & 0x0080_0000 != 0 {
                (raw | 0xFF00_0000) as i32
            } else {
                raw as i32
            });
        }
        off = body + size + 4;
    }
    (ts, comp)
}

/// A composition offset that cannot fit `CompositionTime`'s signed 24-bit
/// millisecond field (§E.4.3.2) must be an error, not a silent SI24
/// truncation (r04-W12).
#[test]
fn package_rejects_oversized_composition_time() {
    let mut demux = FlvDemux::new();
    let source = demux.unpackage(FLV).expect("demux av.flv");
    let CodecConfig::Avc { config, .. } = source.tracks[0].config() else {
        panic!("fixture track 0 must be AVC");
    };
    let config = config.clone();

    // 9 000 000 ms (2.5 h) at timescale 1000 is far past SI24's 8 388 607 ms.
    let sample = transmux::Sample::new(alloc_body(4), Some(0), Some(9_000_000), Some(1_000), true);
    let track = transmux::Track::new(
        transmux::TrackSpec::new(
            1,
            1000,
            CodecConfig::Avc {
                config,
                width: 320,
                height: 240,
            },
        ),
        vec![sample],
    );
    let media = transmux::Media::new(vec![track], 1000);

    let mut mux = FlvMux::new();
    let err = mux
        .package(&media)
        .expect_err("oversized CompositionTime must be rejected");
    // Bites: before the fix the value was truncated to its low 24 bits and
    // `package` returned `Ok`.
    assert!(
        format!("{err}").contains("CompositionTime"),
        "expected a CompositionTime error, got {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 8 — truncated final tag keeps the complete prefix (r04-W15)
// ---------------------------------------------------------------------------

/// A recorded or captured live FLV routinely ends mid-tag. The demuxer must
/// stop at the truncated tail and return everything before it; before the fix
/// `iter_tags` returned `Err(TagOverrun)` for the *whole* file, so
/// `FlvDemux::unpackage` discarded every good tag that preceded the tail.
#[test]
fn truncated_final_tag_keeps_complete_prefix() {
    let mut demux = FlvDemux::new();
    let full = demux.unpackage(FLV).expect("demux intact av.flv");
    let full_video = full.tracks[0].samples.len();
    let full_audio = full.tracks[1].samples.len();
    assert!(full_video > 5 && full_audio > 5, "fixture has real content");

    // Chop the real fixture part-way through a tag that carries a sample:
    // the audio tag whose body is 368 bytes, cut 100 bytes into it.
    let tag_starts = flv_tag_starts(FLV);
    let (audio_start, _, audio_size) = *tag_starts
        .iter()
        .find(|&&(_, ty, size)| ty == 8 && size == 368)
        .expect("fixture has a 368-byte audio tag");
    let cut = audio_start + 11 + 100;
    assert!(
        cut < audio_start + 11 + audio_size,
        "cut is inside the body"
    );
    let truncated = &FLV[..cut];

    let mut demux = FlvDemux::new();
    let media = demux
        .unpackage(truncated)
        .expect("truncated tail must not fail the whole demux");

    assert_eq!(
        media.tracks.len(),
        2,
        "both tracks survive the truncated tail"
    );
    // Bites: the old code returned `Err` here, so this never ran at all.
    // The truncated tag and the two short tags after it are lost; every tag
    // before the cut survives. Bites: the old code returned `Err` here, so
    // the whole file — all 75 video and all 131 audio samples — was lost.
    assert_eq!(
        (media.tracks[0].samples.len(), media.tracks[1].samples.len()),
        (full_video, full_audio - 2),
        "every sample before the truncated tag survives"
    );
    assert!(
        media.tracks[1].samples.len() > 100,
        "the good prefix is essentially all preserved"
    );
}

/// Every top-level tag's byte offset in an FLV file (header + tag loop).
fn flv_tag_starts(flv: &[u8]) -> Vec<(usize, u8, usize)> {
    let data_offset = u32::from_be_bytes([flv[5], flv[6], flv[7], flv[8]]) as usize;
    let mut off = data_offset.max(9) + 4;
    let mut tags = Vec::new();
    while off + 11 <= flv.len() {
        let size = u32::from_be_bytes([0, flv[off + 1], flv[off + 2], flv[off + 3]]) as usize;
        tags.push((off, flv[off], size));
        off += 11 + size + 4;
    }
    tags
}

/// The tail rule is *only* for the tail: a stream whose very first tag is
/// already incomplete has nothing to return, so it still errors.
#[test]
fn incomplete_first_tag_still_errors() {
    // Real fixture header + PreviousTagSize0, then an 11-byte tag header
    // declaring 1000 body bytes, with none of the body present.
    let mut bytes = FLV[..13].to_vec();
    bytes.push(9); // Video tag
    bytes.extend_from_slice(&[0x00, 0x03, 0xE8]); // DataSize = 1000
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]); // Timestamp + StreamID
    let mut demux = FlvDemux::new();
    let err = demux
        .unpackage(&bytes)
        .expect_err("an incomplete first tag must be reported");
    assert!(
        format!("{err}").contains("tag body"),
        "expected TagOverrun, got {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 9 — AAC channel count comes from Table 1.19 (r04-W16)
// ---------------------------------------------------------------------------

/// The `channelConfiguration` field of an `AudioSpecificConfig` is an *index*
/// into ISO/IEC 14496-3 Table 1.19, not a channel count: configuration `7`
/// means 8 channels (7.1) and configuration `0` means the mapping is carried
/// in-band by a `program_config_element`, so the count is unknown from the
/// ASC. Both FLV demuxers used the raw field as the count, so this real
/// ffmpeg-produced 7.1 FLV was reported as a 7-channel track.
#[test]
fn aac_channel_count_uses_table_1_19() {
    // The fixture is `ffmpeg -f lavfi -i "anullsrc=r=48000:cl=7.1" -c:a aac -f flv`;
    // its ASC is 0x11B856E500 (AAC-LC, sfi 3 = 48000 Hz, channelConfiguration 7).
    const FLV_7_1: &[u8] = include_bytes!("../../fixtures/flv/aac-7_1.flv");
    const ORACLE: &str = include_str!("../../fixtures/flv/aac-7_1.oracle.csv");
    let row = ORACLE
        .lines()
        .find(|l| !l.starts_with('#') && !l.trim().is_empty())
        .expect("oracle row");
    let cols: Vec<&str> = row.split(',').collect();
    let oracle_channels: u16 = cols[0].parse().expect("oracle channels");
    let oracle_rate: u32 = cols[2].parse().expect("oracle sample rate");
    assert_eq!(oracle_channels, 8, "ffprobe oracle says 8 channels (7.1)");

    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV_7_1).expect("demux aac-7_1.flv");
    let CodecConfig::Aac {
        channel_count,
        sample_rate,
        ..
    } = media.tracks[0].config()
    else {
        panic!("track 0 must be AAC");
    };
    // Bites: the raw field (7) was written as the count.
    assert_eq!(*channel_count, oracle_channels, "7.1 = 8 channels");
    assert_eq!(*sample_rate, oracle_rate, "sfi 3 = 48000 Hz");
}

/// The same mapping through the streaming demuxer's `TrackAdded` event.
#[test]
fn streaming_aac_channel_count_uses_table_1_19() {
    use transmux::DemuxEvent;
    const FLV_7_1: &[u8] = include_bytes!("../../fixtures/flv/aac-7_1.flv");

    let mut demux = transmux::StreamingFlvDemux::new();
    demux.feed(FLV_7_1).expect("feed aac-7_1.flv");
    let mut channels = None;
    while let Some(ev) = demux.poll_event() {
        if let DemuxEvent::TrackAdded(spec) = ev
            && let CodecConfig::Aac { channel_count, .. } = &spec.config
        {
            channels = Some(*channel_count);
        }
    }
    // Bites: the raw field (7) was written as the count.
    assert_eq!(
        channels,
        Some(8),
        "7.1 = 8 channels through the streaming path"
    );
}

/// Table 1.19 for every non-reserved configuration, including the in-band
/// case (`channelConfiguration == 0`), which is *not* zero channels.
#[test]
fn channel_configuration_channel_counts_match_table_1_19() {
    use transmux::aac_asc::ChannelConfiguration;
    let cases: [(ChannelConfiguration, Option<u16>); 8] = [
        (ChannelConfiguration::InBand, None), // PCE carries the mapping
        (ChannelConfiguration::Mono, Some(1)),
        (ChannelConfiguration::Stereo, Some(2)),
        (ChannelConfiguration::Ch3, Some(3)),
        (ChannelConfiguration::Ch4, Some(4)),
        (ChannelConfiguration::Ch5, Some(5)),
        (ChannelConfiguration::Ch5_1, Some(6)),
        (ChannelConfiguration::Ch7_1, Some(8)),
    ];
    for (cc, expected) in cases {
        assert_eq!(cc.channel_count(), expected, "{cc:?}");
    }
    assert_eq!(ChannelConfiguration::from(9).channel_count(), None);
}

// ---------------------------------------------------------------------------
// Test 10 — oversized AVC dimensions are rejected, never truncated (r04-W38)
// ---------------------------------------------------------------------------

/// `CodecConfig::Avc`'s dimensions are `u16`, but an SPS's
/// `pic_width_in_mbs_minus1`/`pic_height_in_map_units_minus1` are `ue(v)` and
/// unbounded. The FLV demuxers wrote `info.width as u16`, silently folding a
/// 65 536-wide SPS down to 0; a container that misdescribes its own coded size
/// must be an error instead (the `#997` class, previously fixed only in
/// `ts_demux`).
///
/// The fixture is a hand-built FLV whose AVC sequence header carries the SPS
/// `42 00 1F F4 00 08 00 38 80` — Baseline profile, `frame_mbs_only_flag = 1`,
/// `pic_width_in_mbs_minus1 = 4095` (width 65 536), height 48 — followed by
/// one NALU tag. `ffprobe` rejects the file outright ("Invalid data"), which
/// is exactly the point: no conformant decoder accepts these dimensions.
#[test]
fn oversized_avc_dimensions_are_rejected() {
    const OVERSIZE: &[u8] = include_bytes!("../../fixtures/flv/oversize-dims.flv");

    let mut demux = FlvDemux::new();
    let err = demux
        .unpackage(OVERSIZE)
        .expect_err("a 65 536-wide SPS must be rejected, not truncated");
    // Bites: `as u16` folded 65536 to 0 and `unpackage` returned `Ok`.
    assert!(
        format!("{err}").contains("width does not fit 16 bits"),
        "expected a width-overflow error, got {err}"
    );
}

/// The same rejection through the streaming demuxer, which is the path an
/// RTMP publisher's sequence header takes.
#[test]
fn streaming_oversized_avc_dimensions_are_rejected() {
    const OVERSIZE: &[u8] = include_bytes!("../../fixtures/flv/oversize-dims.flv");

    let mut demux = transmux::StreamingFlvDemux::new();
    let err = demux
        .feed(OVERSIZE)
        .expect_err("a 65 536-wide SPS must be rejected, not truncated");
    assert!(
        format!("{err}").contains("width does not fit 16 bits"),
        "expected a width-overflow error, got {err}"
    );
}

/// 65 535 is the largest dimension the IR can carry, so it still parses —
/// the check rejects what does not fit, not what is merely large.
#[test]
fn maximum_representable_width_still_parses() {
    let mut flv: Vec<u8> = Vec::new();
    flv.extend_from_slice(b"FLV   	    ");
    // The same SPS with pic_width_in_mbs_minus1 = 4094 → 65 520 samples.
    let sps = hex_to_bytes("42001ff4001ffee2");
    let mut nal: Vec<u8> = Vec::new();
    nal.push(0x67);
    nal.extend_from_slice(&sps);
    let mut avcc: Vec<u8> = Vec::new();
    avcc.extend_from_slice(&[0x01, 0x42, 0x00, 0x1F, 0xFF, 0xE1]);
    avcc.extend_from_slice(&(nal.len() as u16).to_be_bytes());
    avcc.extend_from_slice(&nal);
    avcc.push(0x00);
    let mut seq_body: Vec<u8> = Vec::new();
    seq_body.extend_from_slice(&[0x17, 0x00, 0x00, 0x00, 0x00]);
    seq_body.extend_from_slice(&avcc);
    push_flv_tag(&mut flv, 9, 0, &seq_body);
    // One length-prefixed NAL so a video sample (and thus a track) exists.
    push_flv_tag(
        &mut flv,
        9,
        0,
        &[0x17, 0x01, 0x00, 0x00, 0x00, 0, 0, 0, 1, 0x09, 0x10],
    );

    let mut demux = FlvDemux::new();
    let media = demux.unpackage(&flv).expect("65 520-wide SPS parses");
    let CodecConfig::Avc { width, height, .. } = media.tracks[0].config() else {
        panic!("track 0 must be AVC");
    };
    assert_eq!(*width, 65_520, "width decoded from the SPS");
    assert_eq!(*height, 48, "height decoded from the SPS");
}

/// Append one FLV tag (header + body + `PreviousTagSize`) to `out`.
fn push_flv_tag(out: &mut Vec<u8>, tag_type: u8, timestamp: u32, body: &[u8]) {
    let start = out.len();
    out.push(tag_type);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&timestamp.to_be_bytes()[1..4]);
    out.push((timestamp >> 24) as u8);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(body);
    let size = (out.len() - start) as u32;
    out.extend_from_slice(&size.to_be_bytes());
}

/// Decode a hex string to bytes.
fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

// ---------------------------------------------------------------------------
// Item 4 — a truncated tail is reported, not silently indistinguishable
// ---------------------------------------------------------------------------

/// A tag whose body overruns the buffer ends the walk, and the caller can tell:
/// `last_walk_was_truncated()` reports it, so a truncated capture is not
/// indistinguishable from a complete one.
#[test]
fn truncated_tail_is_reported_through_the_public_api() {
    // Real fixture, chopped inside a sample-carrying tag.
    let tag_starts = flv_tag_starts(FLV);
    let (audio_start, _, _) = *tag_starts
        .iter()
        .find(|&&(_, ty, size)| ty == 8 && size == 368)
        .expect("fixture has a 368-byte audio tag");
    let cut = audio_start + 11 + 100;

    let mut demux = FlvDemux::new();
    let media = demux
        .unpackage(&FLV[..cut])
        .expect("truncated tail tolerated");
    assert_eq!(media.tracks.len(), 2, "the good prefix survives");
    assert!(
        demux.last_walk_was_truncated(),
        "the cut in the last tag's body must be reported"
    );

    // Bites the other way too: a complete file reports false, and the flag is
    // reset per call on a reused demuxer.
    let mut demux = FlvDemux::new();
    demux.unpackage(FLV).expect("intact fixture");
    assert!(
        !demux.last_walk_was_truncated(),
        "a complete file is not truncated"
    );
    demux.unpackage(&FLV[..cut]).expect("truncated");
    assert!(demux.last_walk_was_truncated(), "flag tracks the last call");
    demux.unpackage(FLV).expect("intact again");
    assert!(
        !demux.last_walk_was_truncated(),
        "the flag must not be stale across calls"
    );
}

/// A tag whose **body is entirely present** but whose trailing 4-byte
/// `PreviousTagSize` is missing is a complete tag, not a truncation: the field
/// is redundant (§E.4.1), so it is kept and nothing is reported.
#[test]
fn tag_missing_only_its_previous_tag_size_is_accepted() {
    let mut demux = FlvDemux::new();
    let full = demux.unpackage(FLV).expect("intact fixture");
    let full_video = full.tracks[0].samples.len();
    let full_audio = full.tracks[1].samples.len();

    // Append one more real audio tag, then drop just its trailing 4-byte
    // `PreviousTagSize`: every tag body is present, only that redundant field
    // is gone.
    let mut extended = FLV.to_vec();
    push_flv_tag(&mut extended, 8, 61999, &[0xAF, 0x01, 0x00, 0x77, 0x88]);
    let body_only = &extended[..extended.len() - 4];

    let mut demux = FlvDemux::new();
    let media = demux
        .unpackage(body_only)
        .expect("a body-complete final tag is not truncated");
    assert_eq!(
        (media.tracks[0].samples.len(), media.tracks[1].samples.len()),
        (full_video, full_audio + 1),
        "every sample, including the new final tag's, is kept"
    );
    assert!(
        !demux.last_walk_was_truncated(),
        "a missing PreviousTagSize is not a truncated tail"
    );
}

// ---------------------------------------------------------------------------
// Item 5 — a zero timescale is an error, not a file timestamped at 0
// ---------------------------------------------------------------------------

/// `timescale == 0` used to make every tag's timestamp 0 (the `ticks_to_ms`
/// placeholder), silently flattening the whole file onto one instant. It is now
/// an error. `RtpDepacketiser` has produced timescale-0 tracks, so this is
/// reachable, not hypothetical.
#[test]
fn zero_timescale_is_rejected_not_flattened_to_zero() {
    let mut demux = FlvDemux::new();
    let source = demux.unpackage(FLV).expect("demux av.flv");
    let CodecConfig::Avc { config, .. } = source.tracks[0].config() else {
        panic!("fixture track 0 must be AVC");
    };
    let config = config.clone();

    let samples: Vec<transmux::Sample> = (0..3u32)
        .map(|i| {
            transmux::Sample::new(alloc_body(4), Some(i as i64), Some(i as i64), Some(1), true)
        })
        .collect();
    let track = transmux::Track::new(
        transmux::TrackSpec::new(
            1,
            0, // no timescale at all
            CodecConfig::Avc {
                config,
                width: 320,
                height: 240,
            },
        ),
        samples,
    );
    let media = transmux::Media::new(vec![track], 0);

    let mut mux = FlvMux::new();
    let err = mux
        .package(&media)
        .expect_err("a zero timescale must be rejected");
    // Bites: before the fix this returned `Ok` with every tag timestamped 0.
    assert!(
        format!("{err}").contains("non-zero track timescale"),
        "expected a timescale error, got {err}"
    );
}

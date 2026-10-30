#![cfg(feature = "std")]
#![allow(clippy::vec_init_then_push)]
//! I-frame playlist rendering (P2a).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::{Parse, Timestamp, Unpackage};
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use hls_runtime::server::{BlockingQuery, Container, DEFAULT_TRACK_ID, HlsOrigin, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;
use transmux::movie_fragment::MovieFragmentBox;
use transmux::pipeline::{
    CodecConfig, FragmentTrackData, TrackSpec, build_init_segment, build_media_segment,
};
use transmux::ts_demux::TsDemux;
use transmux::{Fmp4Demux, TrackId};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with() -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(64), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    (trunk, writer)
}

fn origin_fmp4() -> (Arc<Trunk>, HlsOrigin, SegmentWriter) {
    let (trunk, writer) = trunk_with();
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin");
    let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
    origin.set_init(Bytes::from(init));
    (trunk, origin, writer)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    test_fixtures::read(&format!("hls/cmaf-fmp4/{name}"))
}

fn patient() -> (Timestamp, AwaitPolicy) {
    (
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    )
}

fn playlist_text(origin: &HlsOrigin) -> String {
    let (now, policy) = patient();
    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected playlist, got {other:?}"),
    }
}

fn iframe_playlist_text(origin: &HlsOrigin) -> EgressResponse<hls_runtime::server::HlsBody> {
    let (now, policy) = patient();
    origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
            media_playlist_name: "media.m3u8".to_string(),
        },
        now,
        policy,
    )
}

fn find_moof_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    while offset + transmux::box_types::BOX_HEADER_MIN_SIZE <= bytes.len() {
        let rest = &bytes[offset..];
        let header = transmux::box_types::BoxHeader::parse(rest).ok()?;
        let hdr_sz = header.header_size();
        if hdr_sz > rest.len() {
            break;
        }
        let total = if header.size == u64::from(transmux::box_types::SIZE_TO_EOF) {
            rest.len()
        } else {
            usize::try_from(header.size).ok().filter(|&t| t >= hdr_sz)?
        };
        if header.box_type.is(b"moof") {
            return Some(offset);
        }
        let step = total.min(rest.len());
        if step == 0 {
            break;
        }
        offset += step;
    }
    None
}

fn find_mdat_payload_start(bytes: &[u8], sample_start: usize) -> Option<usize> {
    let mut offset = 0usize;
    while offset + transmux::box_types::BOX_HEADER_MIN_SIZE <= bytes.len() {
        let rest = &bytes[offset..];
        let header = transmux::box_types::BoxHeader::parse(rest).ok()?;
        let hdr_sz = header.header_size();
        if hdr_sz > rest.len() {
            break;
        }
        let total = if header.size == u64::from(transmux::box_types::SIZE_TO_EOF) {
            rest.len()
        } else {
            usize::try_from(header.size).ok().filter(|&t| t >= hdr_sz)?
        };
        if header.box_type.is(b"mdat") {
            let payload = offset + hdr_sz;
            let end = offset + total;
            if sample_start >= payload && sample_start < end {
                return Some(payload);
            }
        }
        let step = total.min(rest.len());
        if step == 0 {
            break;
        }
        offset += step;
    }
    None
}

#[test]
fn iframe_geometry_oracle() {
    let (trunk, origin, writer) = origin_fmp4();
    let mut seg_bytes_list = Vec::new();
    for i in 0..3u32 {
        let bytes = fixture_bytes(&format!("index{i}.m4s"));
        seg_bytes_list.push(bytes.clone());
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let _ = trunk;

    let text = match iframe_playlist_text(&origin) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    let parsed = MediaPlaylist::parse(&text).expect("parse iframe playlist");

    for (idx, ps) in parsed.segments.iter().enumerate() {
        let len = ps.byte_range.as_ref().expect("byterange").length;
        let uri = ps.uri.clone();
        // resolve resource bytes
        let (now, policy) = patient();
        let res = origin.resolve(HlsRequest::Resource { name: uri.clone() }, now, policy);
        let body_bytes = match res {
            EgressResponse::Ready {
                body: hls_runtime::server::HlsBody::Resource(b),
                ..
            } => b,
            other => panic!("expected resource, got {other:?}"),
        };
        let seg_bytes = &seg_bytes_list[idx];
        assert_eq!(&body_bytes[..len as usize], &seg_bytes[..len as usize]);

        // sample_ranges over FULL segment
        let moof_off = find_moof_offset(seg_bytes).expect("moof");
        let rest = &seg_bytes[moof_off..];
        let header = transmux::box_types::BoxHeader::parse(rest).expect("box header");
        let hdr_sz = header.header_size();
        let total = usize::try_from(header.size).expect("size");
        let body = &seg_bytes[moof_off + hdr_sz..moof_off + total];
        let moof = MovieFragmentBox::parse_body(body).expect("moof parse");
        let mdats = transmux::frag_offsets::mdat_ranges(seg_bytes);
        let samples = transmux::frag_offsets::sample_ranges_in(
            seg_bytes,
            &mdats,
            moof_off,
            &moof,
            TrackId::new(1),
        )
        .expect("sample ranges");
        assert!(!samples.is_empty());
        let s0 = samples[0];
        assert_eq!(s0.flags & (1 << 16), 0, "sample 0 must be sync");
        let mdat_start = find_mdat_payload_start(seg_bytes, s0.start).expect("mdat");
        assert_eq!(
            s0.start, mdat_start,
            "sample 0 must start at mdat payload start"
        );
        assert_eq!(
            len as usize, s0.end,
            "iframe_len must equal end of first video sample"
        );

        // Fmp4Demux first video sample equals those bytes
        let full = [fixture_bytes("init.mp4"), seg_bytes.clone()].concat();
        let media = Fmp4Demux::new().unpackage(&full).expect("demux");
        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("video track");
        assert_eq!(video.samples[0].data.as_ref(), &seg_bytes[s0.start..s0.end]);
    }
}

#[test]
fn iframe_playlist_text_shape() {
    let (_trunk, origin, writer) = origin_fmp4();
    for i in 0..6u32 {
        let bytes = fixture_bytes("index0.m4s");
        let discontinuous = i == 0;
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta { discontinuous },
            ))
            .expect("publish");
    }
    let media_text = playlist_text(&origin);
    let iframe_text = match iframe_playlist_text(&origin) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    let pm = MediaPlaylist::parse(&media_text).expect("parse media");
    let pi = MediaPlaylist::parse(&iframe_text).expect("parse iframe");
    assert!(pi.iframes_only);
    assert_eq!(pi.target_duration, pm.target_duration);
    assert_eq!(pi.media_sequence, pm.media_sequence);
    assert_eq!(pi.discontinuity_sequence, pm.discontinuity_sequence);
    for seg in &pi.segments {
        let br = seg.byte_range.as_ref().expect("byterange");
        assert_eq!(br.offset, Some(0));
        assert!(seg.map.is_some());
        assert!(seg.parts.is_empty());
    }
    // version >= 5
    assert!(pi.version >= 5, "version {} < 5", pi.version);
    // durations equal
    for (a, b) in pi.segments.iter().zip(
        pm.segments
            .iter()
            .skip(pm.segments.len().saturating_sub(pi.segments.len())),
    ) {
        assert_eq!(a.duration, b.duration);
    }
}

#[test]
fn iframe_audio_first_leading_skipped() {
    // Build an audio-first segment from real samples.
    let Some(media) = (|| {
        let data = test_fixtures::require_bytes("ts/h264_aac.ts", "iframe")?;
        TsDemux::new().demux(&data).ok()
    })() else {
        return;
    };
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let init = build_init_segment(&specs, media.movie_timescale).expect("init");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video");
    let samples_n = 5usize.min(video.samples.len()).min(audio.samples.len());
    let mut frags = Vec::new();
    frags.push(FragmentTrackData::new(
        audio.track_id(),
        audio.start_decode_time,
        &audio.samples[..samples_n],
    ));
    frags.push(FragmentTrackData::new(
        video.track_id(),
        video.start_decode_time,
        &video.samples[..samples_n],
    ));
    let seg_bytes = build_media_segment(1, &frags).expect("segment");

    let (trunk, origin, writer) = origin_fmp4();
    origin.set_init(Bytes::from(init.clone()));
    // leading audio-first segment
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(seg_bytes.clone()),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    // followed by a normal fixture segment
    let normal = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(normal),
            SegmentSeq::new(2),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let _ = trunk;
    // playlist available (leading None skipped)
    let text = match iframe_playlist_text(&origin) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    let parsed = MediaPlaylist::parse(&text).expect("parse");
    assert_eq!(parsed.segments.len(), 1);
    // EXT-X-MEDIA-SEQUENCE is the public MSN of the first ENTRY (sequence 2), not of the
    // first window segment (sequence 1, skipped): the media playlist lists both, so the
    // I-frame playlist's sequence is the media playlist's plus one.
    let media = MediaPlaylist::parse(&playlist_text(&origin)).expect("media parse");
    assert_eq!(media.segments.len(), 2);
    assert_eq!(parsed.media_sequence, media.media_sequence + 1);
}

#[test]
fn iframe_audio_first_after_entry_not_found() {
    let Some(media) = (|| {
        let data = test_fixtures::require_bytes("ts/h264_aac.ts", "iframe")?;
        TsDemux::new().demux(&data).ok()
    })() else {
        return;
    };
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let init = build_init_segment(&specs, media.movie_timescale).expect("init");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video");
    let samples_n = 5usize.min(video.samples.len()).min(audio.samples.len());
    let mut frags = Vec::new();
    frags.push(FragmentTrackData::new(
        audio.track_id(),
        audio.start_decode_time,
        &audio.samples[..samples_n],
    ));
    frags.push(FragmentTrackData::new(
        video.track_id(),
        video.start_decode_time,
        &video.samples[..samples_n],
    ));
    let seg_bytes = build_media_segment(2, &frags).expect("segment");

    let (_trunk, origin, writer) = origin_fmp4();
    origin.set_init(Bytes::from(init));
    // entry first
    let normal = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(normal),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    // audio-first after entry
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(seg_bytes),
            SegmentSeq::new(2),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    assert_eq!(iframe_playlist_text(&origin), EgressResponse::NotFound);
}

#[test]
fn iframe_nonsync_first_video_leading_skipped() {
    let Some(media) = (|| {
        let data = test_fixtures::require_bytes("ts/h264_aac.ts", "iframe")?;
        TsDemux::new().demux(&data).ok()
    })() else {
        return;
    };
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let init = build_init_segment(&specs, media.movie_timescale).expect("init");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio");
    // start at a non-key sample
    let start = video
        .samples
        .iter()
        .position(|s| !s.flags.is_sync)
        .unwrap_or(1);
    let samples_n = 5usize;
    let end = (start + samples_n).min(video.samples.len());
    let mut frags = Vec::new();
    // video first
    frags.push(FragmentTrackData::new(
        video.track_id(),
        video.start_decode_time,
        &video.samples[start..end],
    ));
    frags.push(FragmentTrackData::new(
        audio.track_id(),
        audio.start_decode_time,
        &audio.samples[..samples_n.min(audio.samples.len())],
    ));
    let seg_bytes = build_media_segment(3, &frags).expect("segment");

    let (_trunk, origin, writer) = origin_fmp4();
    origin.set_init(Bytes::from(init));
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(seg_bytes),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let normal = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(normal),
            SegmentSeq::new(2),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let text = match iframe_playlist_text(&origin) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    let parsed = MediaPlaylist::parse(&text).expect("parse");
    assert_eq!(parsed.segments.len(), 1);
}

#[test]
fn iframe_nonsync_first_video_after_entry_not_found() {
    let Some(media) = (|| {
        let data = test_fixtures::require_bytes("ts/h264_aac.ts", "iframe")?;
        TsDemux::new().demux(&data).ok()
    })() else {
        return;
    };
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let init = build_init_segment(&specs, media.movie_timescale).expect("init");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio");
    let start = video
        .samples
        .iter()
        .position(|s| !s.flags.is_sync)
        .unwrap_or(1);
    let samples_n = 5usize;
    let end = (start + samples_n).min(video.samples.len());
    let mut frags = Vec::new();
    frags.push(FragmentTrackData::new(
        video.track_id(),
        video.start_decode_time,
        &video.samples[start..end],
    ));
    frags.push(FragmentTrackData::new(
        audio.track_id(),
        audio.start_decode_time,
        &audio.samples[..samples_n.min(audio.samples.len())],
    ));
    let seg_bytes = build_media_segment(4, &frags).expect("segment");

    let (_trunk, origin, writer) = origin_fmp4();
    origin.set_init(Bytes::from(init));
    let normal = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(normal),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(seg_bytes),
            SegmentSeq::new(2),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    assert_eq!(iframe_playlist_text(&origin), EgressResponse::NotFound);
}

#[test]
fn iframe_part_is_bad_request() {
    let (_trunk, origin, writer) = origin_fmp4();
    let bytes = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(bytes),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let (now, policy) = patient();
    let resp = origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(1),
                hls_part: Some(0),
                hls_skip: None,
                hls_primary: None,
            },
            media_playlist_name: "media.m3u8".to_string(),
        },
        now,
        policy,
    );
    assert!(matches!(resp, EgressResponse::BadRequest { .. }));
}

#[test]
fn iframe_ts_origin_not_found() {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(64), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("writer");
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .container(Container::MpegTs)
        .instance(7)
        .build()
        .expect("origin");
    let bytes = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(bytes),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    assert_eq!(iframe_playlist_text(&origin), EgressResponse::NotFound);
}

#[test]
fn iframe_audio_only_not_found() {
    let (trunk, writer) = trunk_with();
    let origin = HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin");
    // no track specs -> no video track
    origin.set_track_specs(&[]).unwrap();
    let bytes = vec![0u8; 8];
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(bytes),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    assert_eq!(iframe_playlist_text(&origin), EgressResponse::NotFound);
}

#[test]
fn iframe_rendition_report_uses_configured_media_playlist_name() {
    let (trunk, origin, writer) = origin_fmp4();
    for i in 0..3u32 {
        let bytes = fixture_bytes(&format!("index{i}.m4s"));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let _ = trunk;

    let text = match origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
            media_playlist_name: "live/main.m3u8".to_string(),
        },
        patient().0,
        patient().1,
    ) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    assert!(
        text.contains("#EXT-X-RENDITION-REPORT:URI=\"live/main.m3u8\""),
        "playlist: {text}"
    );
}

#[test]
fn iframe_rendition_report_defaults_to_media_m3u8() {
    let (trunk, origin, writer) = origin_fmp4();
    for i in 0..3u32 {
        let bytes = fixture_bytes(&format!("index{i}.m4s"));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let _ = trunk;

    let text = match origin.resolve(
        HlsRequest::IFramePlaylist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
            media_playlist_name: "media.m3u8".to_string(),
        },
        patient().0,
        patient().1,
    ) {
        EgressResponse::Ready {
            body: hls_runtime::server::HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected iframe playlist, got {other:?}"),
    };
    assert!(
        text.contains("#EXT-X-RENDITION-REPORT:URI=\"media.m3u8\""),
        "playlist: {text}"
    );
}

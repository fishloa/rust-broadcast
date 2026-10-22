//! Streaming (chunked) segment ingest on `DashifIngestSession`
//! (`begin_segment` / `push_segment_bytes` / `finish_segment`, DASH-IF §6.2.4):
//! samples must appear as soon as a CMAF chunk (`moof`+`mdat`) is complete,
//! and the result must not depend on where the byte stream is split.
//! Real data: the committed `h264_aac.ts` packaged by transmux's own writers.

use broadcast_common::{Package, Stage, Timestamp, Unpackage};
use media_plane::ingress::SessionEvent;
use multimux::dashif_ingest::{
    DashifIngestSession, IngestError, IngestObject, ManifestFormat, ObjectKind, SessionError,
};
use transmux::FragmentTrackData;
use transmux::Sample;
use transmux::dash::DashPackager;
use transmux::pipeline::{build_init_segment, build_media_segment};
use transmux::ts_demux::TsDemux;

struct Fixture {
    mpd: String,
    /// `(rep id, init bytes)` in MPD order.
    inits: Vec<(String, Vec<u8>)>,
    /// Video (rep "1") segment 1 as three chunks, each `styp+moof+mdat`.
    video_chunks: Vec<Vec<u8>>,
    /// Audio (rep "2") segment 1, whole.
    audio_seg: Vec<u8>,
}

fn fixture() -> Fixture {
    let ts = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts"),
    )
    .expect("fixture");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
    let mpd = DashPackager::default().package(&media).expect("mpd");
    let mut inits = Vec::new();
    for (i, t) in media.tracks.iter().enumerate() {
        inits.push((
            (i + 1).to_string(),
            build_init_segment(std::slice::from_ref(&t.spec), media.movie_timescale).expect("init"),
        ));
    }
    let v = &media.tracks[0];
    let cuts = [0usize, 9, 17, 25];
    let mut video_chunks = Vec::new();
    for w in cuts.windows(2) {
        let offset: u64 = v.samples[..w[0]]
            .iter()
            .map(|s| u64::from(s.duration.unwrap_or(0)))
            .sum();
        let ftd = FragmentTrackData::new(
            v.spec.track_id,
            v.start_decode_time + offset,
            &v.samples[w[0]..w[1]],
        );
        video_chunks.push(build_media_segment(1, &[ftd]).expect("chunk"));
    }
    let a = &media.tracks[1];
    let ftd = FragmentTrackData::new(a.spec.track_id, a.start_decode_time, &a.samples[..40]);
    let audio_seg = build_media_segment(1, &[ftd]).expect("audio seg");
    Fixture {
        mpd,
        inits,
        video_chunks,
        audio_seg,
    }
}

fn feed(s: &mut DashifIngestSession, kind: ObjectKind, bytes: &[u8]) -> Result<(), SessionError> {
    s.feed(IngestObject { kind, bytes }, Timestamp::from_nanos(0))
}

fn drain(s: &mut DashifIngestSession) -> Vec<SessionEvent> {
    std::iter::from_fn(|| s.poll()).collect()
}

/// A live session (manifest + both headers) with the AUDIO segment already
/// ingested whole, so only the video representation is exercised.
fn live(f: &Fixture) -> DashifIngestSession {
    let mut s = DashifIngestSession::new();
    feed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        f.mpd.as_bytes(),
    )
    .unwrap();
    for (id, init) in &f.inits {
        feed(
            &mut s,
            ObjectKind::Header {
                representation_id: id.clone(),
            },
            init,
        )
        .unwrap();
    }
    feed(
        &mut s,
        ObjectKind::Segment {
            representation_id: "2".into(),
            number: Some(1),
            time: None,
        },
        &f.audio_seg,
    )
    .unwrap();
    drain(&mut s);
    s
}

fn video_samples(events: &[SessionEvent]) -> Vec<Sample> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Sample {
                track_id, sample, ..
            } if track_id.value() == 1 => Some(sample.clone()),
            _ => None,
        })
        .collect()
}

fn same(a: &[Sample], b: &[Sample], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: sample count");
    for (k, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(x.pts, y.pts, "{what}: sample {k} pts");
        assert_eq!(x.dts, y.dts, "{what}: sample {k} dts");
        assert_eq!(x.duration, y.duration, "{what}: sample {k} duration");
        assert_eq!(x.flags.is_sync, y.flags.is_sync, "{what}: sample {k} sync");
        assert_eq!(x.data, y.data, "{what}: sample {k} bytes");
    }
}

/// The reference: the whole logical segment fed as one object.
fn whole_segment_samples(f: &Fixture) -> Vec<Sample> {
    let mut s = live(f);
    let all: Vec<u8> = f.video_chunks.concat();
    feed(
        &mut s,
        ObjectKind::Segment {
            representation_id: "1".into(),
            number: Some(1),
            time: None,
        },
        &all,
    )
    .unwrap();
    video_samples(&drain(&mut s))
}

#[test]
fn chunks_are_emitted_as_soon_as_their_mdat_is_complete() {
    let f = fixture();
    let mut s = live(&f);
    let mut h = s.begin_segment("1", Some(1), None).unwrap();
    let mut got = Vec::new();
    for (i, chunk) in f.video_chunks.iter().enumerate() {
        s.push_segment_bytes(&mut h, chunk).unwrap();
        got.extend(video_samples(&drain(&mut s)));
        // After chunk i has been pushed (and NOT finished), exactly the
        // samples of chunks 0..=i are out.
        let expected = [9usize, 17, 25][i];
        assert_eq!(got.len(), expected, "after chunk {i}");
    }
    s.finish_segment(h).unwrap();
    assert!(
        drain(&mut s).is_empty(),
        "finish must not emit anything new"
    );
    same(&got, &whole_segment_samples(&f), "chunked vs whole");
}

/// The biting test for the box-boundary logic: split the byte stream at many
/// offsets — every offset within 12 bytes of each box boundary (where a header
/// can be cut mid-field) plus a stride across the bodies — and the samples
/// must equal the whole-segment reference no matter where the cut falls.
#[test]
fn result_is_independent_of_where_the_stream_is_split() {
    let f = fixture();
    let want = whole_segment_samples(&f);
    let all: Vec<u8> = f.video_chunks.concat();

    // Box boundaries of the concatenated stream.
    let mut boundaries = vec![0usize];
    let mut off = 0usize;
    while off + 8 <= all.len() {
        let size =
            u32::from_be_bytes([all[off], all[off + 1], all[off + 2], all[off + 3]]) as usize;
        assert!(
            size >= 8 && off + size <= all.len(),
            "fixture is box-aligned"
        );
        off += size;
        boundaries.push(off);
    }
    assert_eq!(off, all.len());

    let mut splits: std::collections::BTreeSet<usize> = (0..=all.len()).step_by(499).collect();
    for b in &boundaries {
        for d in 0..=12usize {
            splits.insert(b.saturating_sub(d));
            splits.insert((b + d).min(all.len()));
        }
    }
    assert!(splits.len() > 50);
    for k in splits {
        let mut s = live(&f);
        let mut h = s.begin_segment("1", Some(1), None).unwrap();
        s.push_segment_bytes(&mut h, &all[..k]).unwrap();
        s.push_segment_bytes(&mut h, &all[k..]).unwrap();
        s.finish_segment(h).unwrap();
        same(
            &video_samples(&drain(&mut s)),
            &want,
            &format!("split at {k}"),
        );
    }
}

#[test]
fn a_partial_trailing_box_is_a_bad_request() {
    let f = fixture();
    let mut s = live(&f);
    let mut h = s.begin_segment("1", Some(1), None).unwrap();
    s.push_segment_bytes(&mut h, &f.video_chunks[0]).unwrap();
    let half = &f.video_chunks[1][..f.video_chunks[1].len() / 2];
    s.push_segment_bytes(&mut h, half).unwrap();
    let e = s.finish_segment(h).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::BadRequest)),
        "{e:?}"
    );
    // The complete first chunk had already been published; the session lives.
    assert_eq!(video_samples(&drain(&mut s)).len(), 9);
    let mut h = s.begin_segment("1", Some(2), None).unwrap();
    s.push_segment_bytes(&mut h, &f.video_chunks[0]).unwrap();
    s.finish_segment(h).unwrap();
}

#[test]
fn a_duplicate_segment_emits_nothing_even_when_streamed() {
    let f = fixture();
    let mut s = live(&f);
    let all: Vec<u8> = f.video_chunks.concat();
    feed(
        &mut s,
        ObjectKind::Segment {
            representation_id: "1".into(),
            number: Some(1),
            time: None,
        },
        &all,
    )
    .unwrap();
    assert_eq!(video_samples(&drain(&mut s)).len(), 25);

    let mut h = s.begin_segment("1", Some(1), None).unwrap();
    for chunk in &f.video_chunks {
        s.push_segment_bytes(&mut h, chunk).unwrap();
    }
    s.finish_segment(h).unwrap();
    assert!(
        drain(&mut s).is_empty(),
        "a re-POSTed segment emits nothing"
    );
}

#[test]
fn begin_runs_the_same_checks_as_the_whole_object_path() {
    let f = fixture();
    let mut s = DashifIngestSession::new();
    let e = s.begin_segment("1", Some(1), None).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::NoManifest)),
        "{e:?}"
    );
    feed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        f.mpd.as_bytes(),
    )
    .unwrap();
    let e = s.begin_segment("1", Some(1), None).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::SegmentWithoutHeader)),
        "{e:?}"
    );
    let e = s.begin_segment("99", Some(1), None).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::BadRequest)),
        "{e:?}"
    );
}

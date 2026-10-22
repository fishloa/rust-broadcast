//! Strict behaviour tests for `DashifIngestSession` (DASH-IF ingest P2a):
//! exact event contents against an independent demux of the same bytes, exact
//! error variants, duplicate/regression handling. Real data: the committed
//! `h264_aac.ts` fixture packaged by transmux's own DASH writer.

use broadcast_common::{Package, Stage, Timestamp, Unpackage};
use media_plane::ingress::SessionEvent;
use multimux::dashif_ingest::{
    DashifIngestSession, IngestError, IngestObject, ManifestFormat, ObjectKind, SessionError,
};
use transmux::FragmentTrackData;
use transmux::Sample;
use transmux::dash::DashPackager;
use transmux::media::Fmp4Demux;
use transmux::pipeline::{build_init_segment, build_media_segment};
use transmux::ts_demux::TsDemux;

const SAMPLES_PER_SEGMENT: usize = 6;

struct Presentation {
    mpd: String,
    /// `(representation id, init bytes, [segment bytes; 2])`, MPD order.
    reps: Vec<(String, Vec<u8>, Vec<Vec<u8>>)>,
}

fn presentation() -> Presentation {
    let ts = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts"),
    )
    .expect("fixture");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
    let mpd = DashPackager::default().package(&media).expect("mpd");
    let mut reps = Vec::new();
    for (i, t) in media.tracks.iter().enumerate() {
        let init =
            build_init_segment(std::slice::from_ref(&t.spec), media.movie_timescale).expect("init");
        assert!(
            t.samples.len() >= 2 * SAMPLES_PER_SEGMENT,
            "fixture track {i} too short"
        );
        let mut segs = Vec::new();
        for s in 0..2 {
            let chunk = &t.samples[s * SAMPLES_PER_SEGMENT..(s + 1) * SAMPLES_PER_SEGMENT];
            let base = t.start_decode_time + chunk_offset(&t.samples[..s * SAMPLES_PER_SEGMENT]);
            let ftd = FragmentTrackData::new(t.spec.track_id, base, chunk);
            segs.push(build_media_segment(u32::try_from(s + 1).unwrap(), &[ftd]).expect("seg"));
        }
        reps.push(((i + 1).to_string(), init, segs));
    }
    Presentation { mpd, reps }
}

fn chunk_offset(prefix: &[Sample]) -> u64 {
    prefix
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .sum()
}

fn feed(s: &mut DashifIngestSession, kind: ObjectKind, bytes: &[u8]) -> Result<(), SessionError> {
    s.feed(IngestObject { kind, bytes }, Timestamp::from_nanos(0))
}

fn drain(s: &mut DashifIngestSession) -> Vec<SessionEvent> {
    std::iter::from_fn(|| s.poll()).collect()
}

fn header(id: &str) -> ObjectKind {
    ObjectKind::Header {
        representation_id: id.into(),
    }
}

fn segment(id: &str, number: u64) -> ObjectKind {
    ObjectKind::Segment {
        representation_id: id.into(),
        number: Some(number),
        time: None,
    }
}

/// Independent oracle: demux init++segment directly, in order.
fn oracle(init: &[u8], seg: &[u8]) -> Vec<Sample> {
    let mut combined = init.to_vec();
    combined.extend_from_slice(seg);
    Fmp4Demux::new()
        .unpackage(combined.as_slice())
        .expect("oracle demux")
        .tracks
        .into_iter()
        .flat_map(|t| t.samples)
        .collect()
}

fn live_session(p: &Presentation) -> DashifIngestSession {
    let mut s = DashifIngestSession::new();
    feed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        p.mpd.as_bytes(),
    )
    .unwrap();
    for (id, init, _) in &p.reps {
        feed(&mut s, header(id), init).unwrap();
    }
    let evs = drain(&mut s);
    assert!(matches!(evs[0], SessionEvent::Established), "{evs:?}");
    assert!(matches!(evs[1], SessionEvent::NewProgram { .. }), "{evs:?}");
    assert_eq!(evs.len(), 2);
    s
}

#[test]
fn new_program_carries_remapped_ids_and_specs_in_mpd_order() {
    let p = presentation();
    let mut s = DashifIngestSession::new();
    feed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        p.mpd.as_bytes(),
    )
    .unwrap();
    // Headers arrive in REVERSE of MPD order: ids must still follow the MPD.
    for (id, init, _) in p.reps.iter().rev() {
        feed(&mut s, header(id), init).unwrap();
    }
    let evs = drain(&mut s);
    let Some(SessionEvent::NewProgram { tracks, .. }) = evs.get(1) else {
        panic!("expected NewProgram second, got {evs:?}");
    };
    assert_eq!(tracks.len(), p.reps.len());
    for (i, ((_, init, _), spec)) in p.reps.iter().zip(tracks).enumerate() {
        assert_eq!(spec.track_id.value(), u32::try_from(i + 1).unwrap());
        let local = Fmp4Demux::new().unpackage(init.as_slice()).unwrap();
        assert_eq!(spec.timescale, local.tracks[0].spec.timescale);
    }
}

#[test]
fn samples_equal_an_independent_demux_sample_for_sample() {
    let p = presentation();
    let mut s = live_session(&p);
    for (id, _, segs) in &p.reps {
        feed(&mut s, segment(id, 1), &segs[0]).unwrap();
    }
    let evs = drain(&mut s);
    for (i, (_, init, segs)) in p.reps.iter().enumerate() {
        let want = oracle(init, &segs[0]);
        let got: Vec<&Sample> = evs
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Sample {
                    track_id, sample, ..
                } if track_id.value() == u32::try_from(i + 1).unwrap() => Some(sample),
                _ => None,
            })
            .collect();
        assert_eq!(got.len(), want.len(), "rep {i}");
        assert_eq!(want.len(), SAMPLES_PER_SEGMENT);
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g.pts, w.pts, "rep {i} sample {k} pts");
            assert_eq!(g.dts, w.dts, "rep {i} sample {k} dts");
            assert_eq!(g.duration, w.duration, "rep {i} sample {k} duration");
            assert_eq!(g.flags.is_sync, w.flags.is_sync, "rep {i} sample {k} sync");
            assert_eq!(g.data, w.data, "rep {i} sample {k} bytes");
        }
    }
}

#[test]
fn errors_are_the_exact_section_5_3_variants() {
    let p = presentation();

    // Segment before any manifest: no grouping to place it.
    let mut s = DashifIngestSession::new();
    let e = feed(&mut s, segment("1", 1), &p.reps[0].2[0]).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::NoManifest)),
        "{e:?}"
    );

    // Segment for a rep with no header: 412.
    let mut s = DashifIngestSession::new();
    feed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        p.mpd.as_bytes(),
    )
    .unwrap();
    let e = feed(&mut s, segment("1", 1), &p.reps[0].2[0]).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::SegmentWithoutHeader)),
        "{e:?}"
    );

    // Header for a representation the MPD does not declare: 400-class.
    let e = feed(&mut s, header("99"), &p.reps[0].1).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::BadRequest)),
        "{e:?}"
    );

    // A changed header after the rep is live: HeaderChanged (400).
    let mut s = live_session(&p);
    let e = feed(&mut s, header("1"), &p.reps[1].1).unwrap_err();
    assert!(
        matches!(e, SessionError::Ingest(IngestError::HeaderChanged)),
        "{e:?}"
    );
    // ...while a byte-identical repeat is an accepted no-op.
    feed(&mut s, header("1"), &p.reps[0].1).unwrap();
    assert!(drain(&mut s).is_empty());
}

#[test]
fn duplicate_emits_nothing_and_regression_still_emits() {
    let p = presentation();
    let mut s = live_session(&p);
    let (id, init, segs) = &p.reps[0];

    feed(&mut s, segment(id, 2), &segs[1]).unwrap();
    let first = drain(&mut s).len();
    assert_eq!(first, SAMPLES_PER_SEGMENT);

    // Same number again = idempotent re-POST (§5.3.2/§6.2.5): no events.
    feed(&mut s, segment(id, 2), &segs[1]).unwrap();
    assert!(drain(&mut s).is_empty(), "duplicate must emit no samples");

    // A LOWER number is a regression (§6.2.10/11 SHOULD, not MUST): the
    // samples are still delivered, in order.
    feed(&mut s, segment(id, 1), &segs[0]).unwrap();
    let evs = drain(&mut s);
    let want = oracle(init, &segs[0]);
    assert_eq!(evs.len(), want.len());
    for (e, w) in evs.iter().zip(&want) {
        let SessionEvent::Sample { sample, .. } = e else {
            panic!("expected Sample, got {e:?}");
        };
        assert_eq!(sample.dts, w.dts);
        assert_eq!(sample.data, w.data);
    }
}

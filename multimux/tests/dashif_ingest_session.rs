use broadcast_common::{Package, Serialize, Stage, Timestamp, Unpackage};
use media_plane::ingress::SessionEvent;
use multimux::dashif_ingest::{DashifIngestSession, IngestObject, ObjectKind};
use transmux::FragmentTrackData;
use transmux::dash::DashPackager;
use transmux::pipeline::{build_init_segment, build_media_segment};
use transmux::ts_demux::TsDemux;

fn demux_media() -> transmux::media::Media {
    let ts = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts"),
    )
    .expect("fixture");
    let mut demux = TsDemux::new();
    demux.unpackage(&ts[..]).expect("demux")
}

#[test]
fn session_basic_flow() {
    let media = demux_media();
    let mut pkg = DashPackager::default();
    let mpd = pkg.package(&media).expect("package");
    let grouping_check =
        multimux::dashif_ingest::ManifestGrouping::from_mpd(mpd.as_bytes()).unwrap();
    let rep_order: Vec<String> = grouping_check
        .switching_sets
        .iter()
        .flatten()
        .cloned()
        .collect();
    assert_eq!(rep_order, vec!["1".to_string(), "2".to_string()]);

    // per-track init + segment
    let mut inits: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();
    let mut segs: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();
    let mut sample_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, t) in media.tracks.iter().enumerate() {
        let rid = (i + 1).to_string();
        let init =
            build_init_segment(std::slice::from_ref(&t.spec), media.movie_timescale).expect("init");
        let n = 5.min(t.samples.len());
        let ftd = FragmentTrackData::new(t.spec.track_id, t.start_decode_time, &t.samples[..n]);
        let seg = build_media_segment(1, &[ftd]).expect("seg");
        inits.insert(rid.clone(), init);
        segs.insert(rid.clone(), seg);
        sample_counts.insert(rid.clone(), 5.min(t.samples.len()));
    }

    let mut session = DashifIngestSession::new();
    let ts = Timestamp::from_nanos(0);

    // probe no events
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Probe,
                bytes: &[],
            },
            ts,
        )
        .unwrap();
    assert!(session.poll().is_none());

    // manifest
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Manifest(multimux::dashif_ingest::ManifestFormat::Dash),
                bytes: mpd.as_bytes(),
            },
            ts,
        )
        .unwrap();
    assert!(matches!(session.poll(), Some(SessionEvent::Established)));
    assert!(session.poll().is_none());

    // re-sent manifest ignored
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Manifest(multimux::dashif_ingest::ManifestFormat::Dash),
                bytes: mpd.as_bytes(),
            },
            ts,
        )
        .unwrap();
    assert!(session.poll().is_none());

    // HLS manifest ignored
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Manifest(multimux::dashif_ingest::ManifestFormat::Hls),
                bytes: b"#EXTM3U",
            },
            ts,
        )
        .unwrap();

    // unknown rep
    let r = session.feed(
        IngestObject {
            kind: ObjectKind::Header {
                representation_id: "99".into(),
            },
            bytes: inits.get("1").unwrap(),
        },
        ts,
    );
    assert!(r.is_err());

    // header rep 1 only
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Header {
                    representation_id: "1".into(),
                },
                bytes: inits.get("1").unwrap(),
            },
            ts,
        )
        .unwrap();
    // segment rep1 buffered (no samples yet)
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Segment {
                    representation_id: "1".into(),
                    number: Some(1),
                    time: None,
                },
                bytes: segs.get("1").unwrap(),
            },
            ts,
        )
        .unwrap();
    // no Sample events yet
    let mut evs = Vec::new();
    while let Some(e) = session.poll() {
        evs.push(e);
    }
    assert!(evs.is_empty(), "samples should be buffered, got {:?}", evs);

    // header rep 2 -> NewProgram + flushed samples
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Header {
                    representation_id: "2".into(),
                },
                bytes: inits.get("2").unwrap(),
            },
            ts,
        )
        .unwrap();
    // feed rep2 segment now that it's live
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Segment {
                    representation_id: "2".into(),
                    number: Some(1),
                    time: None,
                },
                bytes: segs.get("2").unwrap(),
            },
            ts,
        )
        .unwrap();
    let mut evs = Vec::new();
    while let Some(e) = session.poll() {
        evs.push(e);
    }
    // first NewProgram
    assert!(matches!(evs.first(), Some(SessionEvent::NewProgram { .. })));
    // count samples
    let mut counts = std::collections::HashMap::new();
    for e in &evs {
        if let SessionEvent::Sample { track_id, .. } = e {
            *counts.entry(track_id.value()).or_insert(0usize) += 1;
        }
    }
    for (rid, &c) in &sample_counts {
        let tid = rid.parse::<u32>().unwrap();
        assert_eq!(
            counts.get(&tid).copied().unwrap_or(0),
            c,
            "track {} count",
            rid
        );
    }

    // segment-first => 412 for rep with no header? already have. Test new session segment before header
    let mut s2 = DashifIngestSession::new();
    s2.feed(
        IngestObject {
            kind: ObjectKind::Manifest(multimux::dashif_ingest::ManifestFormat::Dash),
            bytes: mpd.as_bytes(),
        },
        ts,
    )
    .unwrap();
    let r = s2.feed(
        IngestObject {
            kind: ObjectKind::Segment {
                representation_id: "1".into(),
                number: Some(1),
                time: None,
            },
            bytes: segs.get("1").unwrap(),
        },
        ts,
    );
    assert!(r.is_err());

    // duplicate segment => no extra events
    // feed same segment again on original session (now live)
    while session.poll().is_some() {}
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Segment {
                    representation_id: "1".into(),
                    number: Some(1),
                    time: None,
                },
                bytes: segs.get("1").unwrap(),
            },
            ts,
        )
        .unwrap();
    let mut after = Vec::new();
    while let Some(e) = session.poll() {
        after.push(e);
    }
    // duplicate => no Sample events (on_segment returns Ok before demux)
    let sample_events = after
        .iter()
        .filter(|e| matches!(e, SessionEvent::Sample { .. }))
        .count();
    assert_eq!(sample_events, 0, "duplicate should emit no samples");

    // different header after live => HeaderChanged
    let mut diff_init = inits.get("1").unwrap().clone();
    diff_init[0] ^= 0xFF;
    let r = session.feed(
        IngestObject {
            kind: ObjectKind::Header {
                representation_id: "1".into(),
            },
            bytes: &diff_init,
        },
        ts,
    );
    assert!(r.is_err());

    // identical header repeat ignored
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Header {
                    representation_id: "1".into(),
                },
                bytes: inits.get("1").unwrap(),
            },
            ts,
        )
        .unwrap();

    session.finish().unwrap();
}

#[test]
fn session_lmsg() {
    let media = demux_media();
    let mut pkg = DashPackager::default();
    let mpd = pkg.package(&media).expect("package");

    let t = &media.tracks[0];
    let init =
        build_init_segment(std::slice::from_ref(&t.spec), media.movie_timescale).expect("init");
    let ftd = FragmentTrackData::new(
        t.spec.track_id,
        t.start_decode_time,
        &t.samples[..5.min(t.samples.len())],
    );
    let seg = build_media_segment(1, &[ftd]).expect("seg");

    // build styp with lmsg compatible brand
    let consumed = transmux::parse_box(&seg).unwrap().1;
    let rest = &seg[consumed..];
    let styp_box = transmux::segments::SegmentTypeBox {
        major_brand: *b"msdh",
        minor_version: 0,
        compatible_brands: vec![*b"msdh", *b"msix", *b"lmsg"],
    };
    let mut styp_bytes = vec![0u8; styp_box.serialized_len()];
    styp_box.serialize_into(&mut styp_bytes).unwrap();
    let mut lmsg_seg = Vec::new();
    lmsg_seg.extend_from_slice(&styp_bytes);
    lmsg_seg.extend_from_slice(rest);

    let mut session = DashifIngestSession::new();
    let ts = Timestamp::from_nanos(0);
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Manifest(multimux::dashif_ingest::ManifestFormat::Dash),
                bytes: mpd.as_bytes(),
            },
            ts,
        )
        .unwrap();
    // only feed rep1 header to keep NewProgram not emitted? Actually need both for NewProgram. Feed both headers but no segments before lmsg? We'll feed rep1 header only then segment rep1 should buffer. But lmsg mark_last via ingest_state; check following header restart accepted.
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Header {
                    representation_id: "1".into(),
                },
                bytes: &init,
            },
            ts,
        )
        .unwrap();
    // feed lmsg segment rep1 (header exists, others missing) -> buffered, mark_last
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Segment {
                    representation_id: "1".into(),
                    number: Some(1),
                    time: None,
                },
                bytes: &lmsg_seg,
            },
            ts,
        )
        .unwrap();
    // header restart accepted: feed identical header again
    session
        .feed(
            IngestObject {
                kind: ObjectKind::Header {
                    representation_id: "1".into(),
                },
                bytes: &init,
            },
            ts,
        )
        .unwrap();
}

//! DASH-IF ingest decisions the existing suites leave open: the size-budget
//! defaults and setters, the manifest grouping the session exposes, a header
//! replaced before the program is live, the per-track state machine's clock,
//! and the `BaseURL` / template rules of `ManifestGrouping::from_mpd`.

use broadcast_common::{Stage, Timestamp};
use dash_runtime::ingest::{
    Addressing, DEFAULT_BUFFERED_BUDGET_BYTES, DEFAULT_MAX_BUFFERED_OBJECT_BYTES,
    DEFAULT_MAX_OBJECT_BYTES, DashifIngestConfig, DashifIngestSession, IngestError, IngestObject,
    IngestState, ManifestFormat, ManifestGrouping, ObjectKind, SegmentDisposition,
};
use transmux::dash::DashPackager;
use transmux::pipeline::build_init_segment;

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn mpd_xml(extra_mpd: &str, period_extra: &str, set_extra: &str, rep_extra: &str) -> String {
    format!(
        r#"<MPD xmlns="{NS}" type="static" mediaPresentationDuration="PT10S" minimumUpdatePeriod="PT2S">{extra_mpd}
           <Period id="p" start="PT0S">{period_extra}<AdaptationSet mimeType="video/mp4">{set_extra}
           <SegmentTemplate timescale="1" duration="2" initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/$Number$.m4s"/>
           <Representation id="V" bandwidth="1">{rep_extra}</Representation></AdaptationSet></Period></MPD>"#
    )
}

fn media_and_mpd() -> (String, transmux::media::Media) {
    let ts = test_fixtures::read("ts/h264-aac.ts");

    let media = test_harness::collect::media_from_ts(&ts[..]).expect("demux");
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mpd = DashPackager::default()
        .render(&specs, &test_harness::collect::segment_log(&media))
        .expect("package");
    (mpd, media)
}

fn init_of(media: &transmux::media::Media, track: usize) -> Vec<u8> {
    build_init_segment(
        std::slice::from_ref(&media.tracks[track].spec),
        media.movie_timescale,
    )
    .expect("init")
}

fn fed(session: &mut DashifIngestSession, kind: ObjectKind, bytes: &[u8]) -> Result<(), String> {
    session
        .feed(IngestObject { kind, bytes }, Timestamp::from_nanos(0))
        .map_err(|e| format!("{e}"))
}

#[test]
fn the_default_size_budgets_are_sixty_four_and_eight_mibibytes() {
    assert_eq!(DEFAULT_MAX_OBJECT_BYTES, 67_108_864);
    assert_eq!(DEFAULT_MAX_BUFFERED_OBJECT_BYTES, 8_388_608);
    assert_eq!(DEFAULT_BUFFERED_BUDGET_BYTES, 67_108_864);
    let c = DashifIngestConfig::default();
    assert_eq!(c.max_object_bytes, 67_108_864);
    assert_eq!(c.max_buffered_object_bytes, 8_388_608);
    assert_eq!(c.buffered_budget_bytes, 67_108_864);
}

#[test]
fn each_setter_replaces_only_its_own_field() {
    let c = DashifIngestConfig::default()
        .with_max_object_bytes(1_000)
        .with_max_buffered_object_bytes(2_000)
        .with_buffered_budget_bytes(3_000);
    assert_eq!(c.max_object_bytes, 1_000);
    assert_eq!(c.max_buffered_object_bytes, 2_000);
    assert_eq!(c.buffered_budget_bytes, 3_000);
    assert_eq!(
        DashifIngestConfig::default()
            .with_max_object_bytes(7)
            .max_object_bytes,
        7
    );
    assert_eq!(
        DashifIngestConfig::default()
            .with_max_buffered_object_bytes(7)
            .max_buffered_object_bytes,
        7
    );
    assert_eq!(
        DashifIngestConfig::default()
            .with_buffered_budget_bytes(7)
            .buffered_budget_bytes,
        7
    );
}

#[test]
fn the_session_reports_its_config_and_the_grouping_it_learned() {
    let (mpd, _) = media_and_mpd();
    let cfg = DashifIngestConfig::default().with_max_object_bytes(1234);
    let mut s = DashifIngestSession::new(cfg);
    assert!(s.grouping().is_none());
    assert_eq!(s.config().max_object_bytes, 1234);
    assert_eq!(s.config().max_buffered_object_bytes, 8_388_608);
    fed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        mpd.as_bytes(),
    )
    .unwrap();
    let g = s.grouping().expect("the grouping of the accepted manifest");
    assert_eq!(
        g.switching_sets,
        vec![vec!["1".to_owned()], vec!["2".to_owned()]]
    );
    assert_eq!(g.addressing, Addressing::Number);
}

#[test]
fn a_second_header_before_the_program_is_live_is_accepted() {
    let (mpd, media) = media_and_mpd();
    let mut s = DashifIngestSession::new(DashifIngestConfig::default());
    fed(
        &mut s,
        ObjectKind::Manifest(ManifestFormat::Dash),
        mpd.as_bytes(),
    )
    .unwrap();
    let first = init_of(&media, 0);
    let second = init_of(&media, 1);
    fed(
        &mut s,
        ObjectKind::Header {
            representation_id: "1".into(),
        },
        &first,
    )
    .unwrap();
    assert!(
        fed(
            &mut s,
            ObjectKind::Header {
                representation_id: "1".into(),
            },
            &second,
        )
        .is_ok(),
        "the program is not live yet: a different init replaces the first"
    );
}

#[test]
fn the_track_state_machine_counts_time_as_well_as_number() {
    let mut st = IngestState::new();
    assert!(!st.has_header("v"));
    assert!(matches!(
        st.on_segment("v", None, Some(10)),
        Err(IngestError::SegmentWithoutHeader)
    ));
    st.on_header("v");
    assert!(st.has_header("v"));
    assert_eq!(
        st.on_segment("v", None, Some(10)).unwrap(),
        SegmentDisposition::Accepted
    );
    assert_eq!(
        st.on_segment("v", None, Some(10)).unwrap(),
        SegmentDisposition::Duplicate
    );
    assert_eq!(
        st.on_segment("v", None, Some(20)).unwrap(),
        SegmentDisposition::Accepted
    );
    assert_eq!(
        st.on_segment("v", None, Some(15)).unwrap(),
        SegmentDisposition::Regression
    );
    assert!(!st.is_last("v"));
    st.mark_last("v");
    assert!(st.is_last("v"));
    st.on_header("v");
    assert!(!st.is_last("v"), "a restart clears the end marker");
}

#[test]
fn a_base_url_at_any_level_is_refused() {
    for (mpd_extra, period_extra, set_extra, rep_extra) in [
        ("<BaseURL>http://cdn.example/</BaseURL>", "", "", ""),
        ("", "<BaseURL>http://cdn.example/</BaseURL>", "", ""),
        ("", "", "<BaseURL>http://cdn.example/</BaseURL>", ""),
        ("", "", "", "<BaseURL>http://cdn.example/</BaseURL>"),
    ] {
        let xml = mpd_xml(mpd_extra, period_extra, set_extra, rep_extra);
        assert!(
            matches!(
                ManifestGrouping::from_mpd(xml.as_bytes()),
                Err(IngestError::MpdBaseUrlPresent)
            ),
            "{xml}"
        );
    }
    let clean = mpd_xml("", "", "", "");
    assert!(ManifestGrouping::from_mpd(clean.as_bytes()).is_ok());
}

#[test]
fn the_addressing_mode_comes_from_the_media_template() {
    let number = mpd_xml("", "", "", "");
    assert_eq!(
        ManifestGrouping::from_mpd(number.as_bytes())
            .unwrap()
            .addressing,
        Addressing::Number
    );
    let time = number.replace(
        "$RepresentationID$/$Number$.m4s",
        "$RepresentationID$/$Time$.m4s",
    );
    assert_eq!(
        ManifestGrouping::from_mpd(time.as_bytes())
            .unwrap()
            .addressing,
        Addressing::Time
    );
    let formatted = number.replace("$Number$", "$Number%03d$");
    assert_eq!(
        ManifestGrouping::from_mpd(formatted.as_bytes())
            .unwrap()
            .addressing,
        Addressing::Number,
        "a formatted $Number%03d$ still counts as one Number"
    );
    let twice = number.replace("$Number$", "$Number$-$Number$");
    assert!(matches!(
        ManifestGrouping::from_mpd(twice.as_bytes()),
        Err(IngestError::MpdTemplateMismatch { attr: "media" })
    ));
}

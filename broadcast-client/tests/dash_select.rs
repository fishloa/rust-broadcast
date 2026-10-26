//! Variant and audio-rendition selection of a DASH `Session`: the Variants/AudioRendition mapping,
//! `select_variant`, `select_audio_rendition`, automatic audio-language selection, and the choices
//! surviving a Period change. The MPDs are the real committed ones with extra Representations and
//! AdaptationSets added; the extra ones are served from the real media of the original, under
//! other URLs, so a switch shows on the wire while the samples stay checkable against the oracle.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;

use broadcast_client::{Config, Error, Event, Session, Status, TrackId};
use dash_common::loopback::{Origin, oracle};
use dash_common::world::World;
use dash_sim::{Backend, DashSim, Logged, SEC, StaticBackend};
use hls_runtime::client::MasterSelection;

const HOST: &str = "http://dash.test";
const MANIFEST: &str = "manifest.mpd";
const RESTART: &str = "multiperiod-restart/multiperiod.mpd";

struct Alias {
    inner: StaticBackend,
    map: fn(&str) -> String,
}

impl Backend for Alias {
    fn serve(&mut self, at_utc_us: i64, req: &Logged) -> dash_sim::Answer {
        let mut mapped = req.clone();
        mapped.url = (self.map)(&req.url);
        self.inner.serve(at_utc_us, &mapped)
    }
}

fn alias_ffmpeg(url: &str) -> String {
    let Some(file) = url.strip_prefix("http://dash.test/alt-") else {
        return url.to_owned();
    };
    let (stream, rest) = file.split_once('-').unwrap();
    let stream = if stream == "v" { 0 } else { 1 };
    match rest.strip_suffix(".m4s") {
        Some("init") => format!("{HOST}/init-stream{stream}.m4s"),
        Some(n) => format!("{HOST}/chunk-stream{stream}-{n}.m4s"),
        None => url.to_owned(),
    }
}

fn alias_restart(url: &str) -> String {
    let Some(file) = url.strip_prefix("http://dash.test/multiperiod-restart/alt-") else {
        return url.to_owned();
    };
    format!("{HOST}/multiperiod-restart/{file}")
}

const ALT_VIDEO: &str = r#"<Representation id="9" mimeType="video/mp4" codecs="avc1.4d400d" bandwidth="20000" width="320" height="240" sar="1:1">
				<SegmentTemplate timescale="90000" initialization="alt-v-init.m4s" media="alt-v-$Number%05d$.m4s" startNumber="1">
					<SegmentTimeline>
						<S t="2070" d="90000" r="2" />
					</SegmentTimeline>
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
		<AdaptationSet id="1" contentType="audio""#;

fn alt_audio(id: u32, lang: &str, label: &str, role: &str) -> String {
    format!(
        r#"<AdaptationSet id="{id}" contentType="audio" startWithSAP="1" segmentAlignment="true" lang="{lang}">
			<Role schemeIdUri="urn:mpeg:dash:role:2011" value="{role}"/>
			<Label>{label}</Label>
			<Representation id="{id}0" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="96013" audioSamplingRate="44100">
				<AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="1" />
				<SegmentTemplate timescale="44100" initialization="alt-a-init.m4s" media="alt-a-$Number%05d$.m4s" startNumber="1">
					<SegmentTimeline>
						<S t="0" d="41984" />
						<S d="44032" />
						<S d="45056" />
						<S d="3072" />
					</SegmentTimeline>
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
	"#
    )
}

fn base_manifest() -> String {
    test_fixtures::read_to_string(&format!("dash/{MANIFEST}"))
}

fn with_two_video() -> String {
    let t = base_manifest();
    let marker = "</AdaptationSet>\n\t\t<AdaptationSet id=\"1\" contentType=\"audio\"";
    assert!(t.contains(marker));
    t.replacen(marker, ALT_VIDEO, 1)
}

fn with_two_audio(
    first_lang: &str,
    first_role: &str,
    second_lang: &str,
    second_role: &str,
) -> String {
    let t = base_manifest()
        .replacen(
            "<AdaptationSet id=\"1\" contentType=\"audio\" startWithSAP=\"1\" segmentAlignment=\"true\" bitstreamSwitching=\"true\" lang=\"und\">",
            &format!(
                "<AdaptationSet id=\"1\" contentType=\"audio\" startWithSAP=\"1\" segmentAlignment=\"true\" bitstreamSwitching=\"true\" lang=\"{first_lang}\"><Role schemeIdUri=\"urn:mpeg:dash:role:2011\" value=\"{first_role}\"/><Label>First</Label>"
            ),
            1,
        );
    t.replacen(
        "\t</Period>",
        &format!(
            "\t{}</Period>",
            alt_audio(2, second_lang, "Second", second_role)
        ),
        1,
    )
}

fn sim_for(rel: &str, text: String, config: Config, map: fn(&str) -> String) -> DashSim {
    let url = format!("{HOST}/{rel}");
    let origin = Origin::new().doc(&url, text.into_bytes());
    DashSim::new(
        &url,
        config,
        World::new(1_700_000_000 * 1_000_000),
        Box::new(Alias {
            inner: StaticBackend(origin),
            map,
        }),
    )
}

fn manifests(sim: &DashSim) -> Vec<&MasterSelection> {
    sim.events
        .iter()
        .filter_map(|e| match e {
            Event::Variants(m) => Some(m),
            _ => None,
        })
        .collect()
}

fn audio_events(sim: &DashSim) -> Vec<(Option<usize>, Option<String>, Option<String>)> {
    sim.events
        .iter()
        .filter_map(|e| match e {
            Event::AudioRendition {
                index,
                name,
                language,
            } => Some((*index, name.clone(), language.clone())),
            _ => None,
        })
        .collect()
}

fn run_until(sim: &mut DashSim, pred: impl Fn(&DashSim) -> bool) {
    for _ in 0..200_000 {
        sim.run_for(10_000);
        if pred(sim) || sim.terminal() {
            return;
        }
    }
    panic!("condition not reached");
}

fn samples_of(sim: &DashSim, track: u32) -> usize {
    sim.events
        .iter()
        .filter(|e| matches!(e, Event::Sample { track: t, .. } if t.value() == track))
        .count()
}

fn one_at_a_time() -> Config {
    let mut config = Config::default();
    config.max_inflight_segments = 1;
    config
}

#[test]
fn the_variants_event_maps_the_manifest_and_names_the_active_audio() {
    let mut sim = DashSim::static_origin(&format!("{HOST}/{MANIFEST}"), Config::default());
    sim.run_to_terminal(60 * SEC);
    let m = manifests(&sim);
    assert_eq!(m.len(), 1);
    let m = m[0];
    assert_eq!(m.chosen, 0);
    assert_eq!(m.variants.len(), 1);
    let v = &m.variants[0];
    assert_eq!(v.url, "http://dash.test/init-stream0.m4s");
    assert_eq!(v.bandwidth, 58_141);
    assert_eq!(v.average_bandwidth, None);
    assert_eq!(v.resolution, Some((320, 240)));
    assert_eq!(v.codecs.as_deref(), Some("avc1.4d400d"));
    assert_eq!(v.subtitles_group, None);
    assert_eq!(m.renditions.len(), 1);
    let r = &m.renditions[0];
    assert_eq!(r.media_type, broadcast_hls::MediaType::Audio);
    assert_eq!(r.group_id, "1");
    assert_eq!(r.name, "und");
    assert_eq!(r.language.as_deref(), Some("und"));
    assert!(!r.default, "no Role main: no DEFAULT");
    assert!(r.autoselect);
    assert_eq!(r.url.as_deref(), Some("http://dash.test/init-stream1.m4s"));
    assert_eq!(v.audio_group.as_deref(), Some("1"));
    assert_eq!(
        audio_events(&sim),
        vec![(Some(0), Some("und".to_owned()), Some("und".to_owned()))]
    );
    let order: Vec<&str> = sim
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Status(Status::Establishing) => Some("establishing"),
            Event::Variants(_) => Some("variants"),
            Event::AudioRendition { .. } => Some("audio"),
            Event::Tracks(_) => Some("tracks"),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["establishing", "variants", "audio", "tracks"]);
}

#[test]
fn two_video_representations_list_in_mpd_order_and_the_best_is_chosen() {
    let mut sim = sim_for(MANIFEST, with_two_video(), Config::default(), alias_ffmpeg);
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let m = manifests(&sim)[0];
    let bw: Vec<u32> = m.variants.iter().map(|v| v.bandwidth).collect();
    assert_eq!(bw, [58_141, 20_000]);
    assert_eq!(m.chosen, 0, "the highest bandwidth");
    assert_eq!(m.variants[1].url, "http://dash.test/alt-v-init.m4s");
}

#[test]
fn select_variant_switches_at_the_next_segment_and_keeps_the_samples_intact() {
    let mut sim = sim_for(MANIFEST, with_two_video(), one_at_a_time(), alias_ffmpeg);
    run_until(&mut sim, |s| samples_of(s, 1) > 0);
    sim.session.select_variant(1).unwrap();
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    assert!(!sim.requests_to("chunk-stream0-00001").is_empty());
    assert!(
        !sim.requests_to("alt-v-00003").is_empty(),
        "the later segments come from the new Representation"
    );
    assert!(
        sim.requests_to("alt-v-00001").is_empty(),
        "the first was already requested"
    );
    let want = oracle(&[
        "init-stream0.m4s",
        "chunk-stream0-00001.m4s",
        "chunk-stream0-00002.m4s",
        "chunk-stream0-00003.m4s",
    ]);
    let got = sim.samples();
    assert_eq!(dash_sim::sigs_of(&got[&1]), want[0]);
}

#[test]
fn an_unknown_variant_is_an_error_and_nothing_is_known_before_the_manifest() {
    let mut early = Session::open(&format!("{HOST}/{MANIFEST}"), Config::default(), 0).unwrap();
    assert!(matches!(
        early.select_variant(0),
        Err(Error::NoSuchVariant(0))
    ));
    assert!(matches!(
        early.select_audio_rendition(Some(0)),
        Err(Error::NoSuchRendition)
    ));
    let mut sim = sim_for(MANIFEST, with_two_video(), Config::default(), alias_ffmpeg);
    sim.run_to_terminal(60 * SEC);
    assert!(matches!(
        sim.session.select_variant(2),
        Err(Error::NoSuchVariant(2))
    ));
    assert!(
        sim.session.select_variant(1).is_ok(),
        "a known index after Ended is accepted"
    );
    let after = sim.events.len();
    sim.run_for(5 * SEC);
    assert_eq!(
        sim.events.len(),
        after,
        "and ignored: nothing starts, nothing is emitted"
    );
}

#[test]
fn a_manual_variant_choice_survives_a_period_change() {
    let alt = |period: u32, track: u32| {
        format!(
            r#"<Representation id="9{period}" codecs="avc1.4D400D" width="320" height="240" frameRate="25" sar="1:1" bandwidth="20000">
    <SegmentTemplate media="alt-p{period}_dash_track{track}_$Number$.m4s" initialization="alt-p{period}_dash_track{track}_init.mp4" timescale="12800" startNumber="1" duration="25600"/>
   </Representation>
   "#
        )
    };
    let mut text = test_fixtures::read_to_string(&format!("dash/{RESTART}"));
    text = text.replacen(
        "</Representation>\n  </AdaptationSet>\n  <AdaptationSet segmentAlignment=\"true\" mimeType=\"audio/mp4\" startWithSAP=\"1\">\n   <SegmentTemplate media=\"p1_",
        &format!("</Representation>\n   {}</AdaptationSet>\n  <AdaptationSet segmentAlignment=\"true\" mimeType=\"audio/mp4\" startWithSAP=\"1\">\n   <SegmentTemplate media=\"p1_", alt(1, 1)),
        1,
    );
    text = text.replacen(
        "</Representation>\n  </AdaptationSet>\n  <AdaptationSet segmentAlignment=\"true\" mimeType=\"audio/mp4\" startWithSAP=\"1\">\n   <SegmentTemplate media=\"p2_",
        &format!("</Representation>\n   {}</AdaptationSet>\n  <AdaptationSet segmentAlignment=\"true\" mimeType=\"audio/mp4\" startWithSAP=\"1\">\n   <SegmentTemplate media=\"p2_", alt(2, 1)),
        1,
    );
    assert_eq!(text.matches("alt-p").count(), 4);
    let mut sim = sim_for(RESTART, text, one_at_a_time(), alias_restart);
    run_until(&mut sim, |s| !manifests(s).is_empty());
    let first = manifests(&sim)[0].clone();
    assert_eq!(first.variants.len(), 2);
    let low = first
        .variants
        .iter()
        .position(|v| v.bandwidth == 20_000)
        .unwrap();
    assert_ne!(first.chosen, low);
    sim.session.select_variant(low).unwrap();
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let all = manifests(&sim);
    assert_eq!(all.len(), 2);
    assert_ne!(
        all[1].chosen, low,
        "the second Period starts on its own best Representation"
    );
    let urls: Vec<String> = sim
        .log
        .iter()
        .map(|l| l.url.rsplit('/').next().unwrap().to_owned())
        .collect();
    let asked = |name: &str| urls.iter().any(|u| u == name);
    assert!(
        asked("alt-p1_dash_track1_2.m4s") && asked("alt-p1_dash_track1_3.m4s"),
        "the choice takes effect at the next segment of the first Period: {urls:?}"
    );
    assert!(
        asked("alt-p2_dash_track1_2.m4s") && asked("alt-p2_dash_track1_3.m4s"),
        "and is re-applied in the second, from the next segment not yet requested: {urls:?}"
    );
    assert!(
        !asked("alt-p1_dash_track1_1.m4s"),
        "the first segment was already requested"
    );
}

#[test]
fn the_preferred_audio_language_picks_the_matching_set_and_the_main_role_is_default() {
    let text = || with_two_audio("en", "main", "fr-CA", "alternate");
    let mut sim = sim_for(MANIFEST, text(), Config::default(), alias_ffmpeg);
    sim.run_to_terminal(60 * SEC);
    let m = manifests(&sim)[0];
    assert_eq!(m.renditions.len(), 2);
    assert_eq!(m.renditions[0].name, "First");
    assert_eq!(m.renditions[1].name, "Second");
    assert_eq!(m.renditions[1].language.as_deref(), Some("fr-CA"));
    assert!(m.renditions[0].default && !m.renditions[1].default);
    assert_eq!(
        audio_events(&sim),
        vec![(Some(0), Some("First".to_owned()), Some("en".to_owned()))]
    );

    let mut config = Config::default();
    config.preferred_audio_language = Some("FR".to_owned());
    let mut sim = sim_for(MANIFEST, text(), config, alias_ffmpeg);
    sim.run_to_terminal(60 * SEC);
    assert_eq!(
        audio_events(&sim),
        vec![(Some(1), Some("Second".to_owned()), Some("fr-CA".to_owned()))]
    );
    assert!(!sim.requests_to("alt-a-").is_empty());
    assert!(sim.requests_to("chunk-stream1").is_empty());
    let want = oracle(&[
        "init-stream1.m4s",
        "chunk-stream1-00001.m4s",
        "chunk-stream1-00002.m4s",
        "chunk-stream1-00003.m4s",
        "chunk-stream1-00004.m4s",
    ]);
    assert_eq!(dash_sim::sigs_of(&sim.samples()[&2]), want[0]);
}

#[test]
fn select_audio_rendition_switches_the_audio_set_mid_stream() {
    let mut sim = sim_for(
        MANIFEST,
        with_two_audio("en", "main", "fr", "alternate"),
        one_at_a_time(),
        alias_ffmpeg,
    );
    run_until(&mut sim, |s| samples_of(s, 2) > 0);
    sim.session.select_audio_rendition(Some(1)).unwrap();
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    assert_eq!(
        audio_events(&sim),
        vec![
            (Some(0), Some("First".to_owned()), Some("en".to_owned())),
            (Some(1), Some("Second".to_owned()), Some("fr".to_owned())),
        ]
    );
    assert!(!sim.requests_to("alt-a-00004").is_empty());
    let want = oracle(&[
        "init-stream1.m4s",
        "chunk-stream1-00001.m4s",
        "chunk-stream1-00002.m4s",
        "chunk-stream1-00003.m4s",
        "chunk-stream1-00004.m4s",
    ]);
    assert_eq!(
        dash_sim::sigs_of(&sim.samples()[&2]),
        want[0],
        "no audio sample lost or repeated"
    );
    assert!(matches!(
        sim.session.select_audio_rendition(Some(2)),
        Err(Error::NoSuchRendition)
    ));
}

#[test]
fn audio_can_be_stopped_and_started_again() {
    let mut sim = sim_for(
        MANIFEST,
        with_two_audio("en", "main", "fr", "alternate"),
        one_at_a_time(),
        alias_ffmpeg,
    );
    run_until(&mut sim, |s| samples_of(s, 2) > 0);
    sim.session.select_audio_rendition(None).unwrap();
    let stop_at = sim.events.len();
    let audio_so_far = samples_of(&sim, 2);
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    let tail = &sim.events[stop_at..];
    let tracks: Vec<&Vec<broadcast_client::TrackSpec>> = tail
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(
        tracks.len(),
        1,
        "the track set is re-announced once, without audio"
    );
    assert_eq!(
        tracks[0].iter().map(|t| t.track_id).collect::<Vec<_>>(),
        vec![TrackId::new(1)]
    );
    assert!(
        samples_of(&sim, 2) <= audio_so_far + 1,
        "at most the segment already in flight is still delivered"
    );
    assert!(samples_of(&sim, 1) > 0);
    assert!(audio_events(&sim).last().is_some_and(|a| a.0.is_none()));
    assert!(
        sim.requests_to("alt-a-").is_empty() && sim.requests_to("chunk-stream1-00004").is_empty()
    );
}

#[test]
fn auto_select_audio_off_plays_video_only_until_a_rendition_is_chosen() {
    let mut config = one_at_a_time();
    config.auto_select_audio = false;
    let mut sim = sim_for(
        MANIFEST,
        with_two_audio("en", "main", "fr", "alternate"),
        config,
        alias_ffmpeg,
    );
    run_until(&mut sim, |s| samples_of(s, 1) > 0);
    assert!(audio_events(&sim).iter().all(|a| a.0.is_none()));
    sim.session.select_audio_rendition(Some(1)).unwrap();
    sim.run_to_terminal(60 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert_eq!(audio_events(&sim).last().map(|a| a.0), Some(Some(1)));
    assert!(samples_of(&sim, 2) > 0, "audio plays once chosen");
}

#[test]
fn a_ts_over_http_session_has_no_variants_or_renditions() {
    let mut s = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    assert!(matches!(s.select_variant(0), Err(Error::NotHls)));
    assert!(matches!(s.select_audio_rendition(None), Err(Error::NotHls)));
}

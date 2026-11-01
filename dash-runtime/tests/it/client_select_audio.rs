//! `DashClient::select_audio`: switching the audio set mid-stream, stopping and restarting audio,
//! and the errors. The MPD is the real ffmpeg manifest with a second audio AdaptationSet added; the
//! second set's URLs are served from the real audio files of the first.

use crate::common;

use std::cell::RefCell;
use std::rc::Rc;

use common::loopback::{Drive, Origin, Run, drive, mpd_text, oracle, sigs};
use dash_runtime::client::{DashClient, Error, Output, PullLimits};

const URL: &str = "http://dash.test/manifest.mpd";
const SECOND_AUDIO: &str = r#"<AdaptationSet id="2" contentType="audio" startWithSAP="1" segmentAlignment="true" lang="fr">
			<Representation id="7" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="96013" audioSamplingRate="44100">
				<SegmentTemplate timescale="44100" initialization="init-stream1.m4s" media="chunk-stream1-$Number%05d$.m4s" startNumber="1">
					<SegmentTimeline>
						<S t="0" d="41984" />
						<S d="44032" />
						<S d="45056" />
						<S d="3072" />
					</SegmentTimeline>
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
	</Period>"#;

fn origin() -> Origin {
    let text = mpd_text("manifest.mpd").replacen("</Period>", SECOND_AUDIO, 1);
    Origin::new().doc(URL, text.into_bytes())
}

fn one_at_a_time() -> PullLimits {
    let mut cfg = PullLimits::default();
    cfg.max_inflight_segments = 1;
    cfg
}

type Attempts = Rc<RefCell<Vec<Result<(), Error>>>>;

fn run_with(choice: Vec<(usize, Option<usize>)>) -> (Run, Attempts) {
    let attempts: Attempts = Rc::new(RefCell::new(Vec::new()));
    let log = attempts.clone();
    let mut pending = choice;
    let hook = move |c: &mut DashClient, step: usize| {
        if let Some(&(at, pick)) = pending.first()
            && step >= at
        {
            let r = c.select_audio(pick);
            let retry = matches!(r, Err(Error::NotSwitchable));
            log.borrow_mut().push(r);
            if !retry {
                pending.remove(0);
            }
        }
    };
    let run = drive(
        URL,
        one_at_a_time(),
        &mut origin(),
        &mut Drive {
            hook: Some(Box::new(hook)),
            ..Drive::default()
        },
    );
    (run, attempts)
}

fn audio_samples(run: &Run) -> Vec<common::loopback::Sig> {
    sigs(run.samples().get(&2).map(Vec::as_slice).unwrap_or_default())
}

fn full_audio() -> Vec<common::loopback::Sig> {
    oracle(&[
        "init-stream1.m4s",
        "chunk-stream1-00001.m4s",
        "chunk-stream1-00002.m4s",
        "chunk-stream1-00003.m4s",
        "chunk-stream1-00004.m4s",
    ])
    .remove(0)
}

#[test]
fn the_manifest_lists_both_audio_sets_and_the_policy_picks_the_first() {
    let run = drive(URL, one_at_a_time(), &mut origin(), &mut Drive::default());
    let manifest = run
        .outputs
        .iter()
        .find_map(|o| match o {
            Output::Manifest(m) => Some(m),
            _ => None,
        })
        .unwrap();
    assert_eq!(manifest.audio.len(), 2);
    assert_eq!(manifest.chosen_audio, Some(0));
}

#[test]
fn switching_to_the_other_audio_set_keeps_every_sample_once() {
    let (run, attempts) = run_with(vec![(8, Some(1))]);
    assert!(
        attempts.borrow().iter().any(Result::is_ok),
        "{:?}",
        attempts.borrow()
    );
    assert!(run.ended && run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(audio_samples(&run), full_audio());
}

#[test]
fn an_index_outside_the_audio_list_is_refused_and_changes_nothing() {
    let (run, attempts) = run_with(vec![(8, Some(5))]);
    assert_eq!(attempts.borrow().last(), Some(&Err(Error::NoSuchAudio)));
    assert!(run.ended);
    assert_eq!(audio_samples(&run), full_audio());
}

#[test]
fn nothing_can_be_selected_before_the_period_plays() {
    let mut c = DashClient::new(
        URL,
        dash_runtime::client::DashConfig::default(),
        PullLimits::default(),
    );
    assert_eq!(c.select_audio(Some(0)), Err(Error::NotSwitchable));
    assert_eq!(c.select_audio(None), Err(Error::NotSwitchable));
}

#[test]
fn stopping_audio_ends_its_requests_and_starting_it_again_resumes_from_the_next_segment() {
    let (stopped, attempts) = run_with(vec![(6, None)]);
    assert!(
        attempts.borrow().iter().any(Result::is_ok),
        "{:?}",
        attempts.borrow()
    );
    assert!(
        stopped.ended && stopped.errors.is_empty(),
        "{:?}",
        stopped.errors
    );
    let heard = audio_samples(&stopped);
    assert!(!heard.is_empty() && heard.len() < full_audio().len());
    assert_eq!(
        heard,
        full_audio()[..heard.len()],
        "a prefix, nothing repeated"
    );
    assert!(stopped.requested("chunk-stream1-00004") == 0);

    let (resumed, attempts) = run_with(vec![(6, None), (7, Some(0))]);
    assert!(
        attempts.borrow().iter().filter(|r| r.is_ok()).count() == 2,
        "{:?}",
        attempts.borrow()
    );
    assert!(
        resumed.ended && resumed.errors.is_empty(),
        "{:?}",
        resumed.errors
    );
    let after = audio_samples(&resumed);
    assert!(after.len() > heard.len(), "audio resumes");
    let full = full_audio();
    assert_eq!(after.last(), full.last(), "and reaches the end");
}

#[test]
fn a_set_with_several_representations_plays_its_highest_bandwidth_one() {
    let rep = |id: u32, bandwidth: u32| {
        format!(
            r#"<Representation id="{id}" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="{bandwidth}" audioSamplingRate="44100">
				<SegmentTemplate timescale="44100" initialization="r$RepresentationID$-init.m4s" media="r$RepresentationID$-$Number%05d$.m4s" startNumber="1">
					<SegmentTimeline>
						<S t="0" d="41984" />
						<S d="44032" />
						<S d="45056" />
						<S d="3072" />
					</SegmentTimeline>
				</SegmentTemplate>
			</Representation>"#
        )
    };
    let second = format!(
        r#"<AdaptationSet id="2" contentType="audio" startWithSAP="1" segmentAlignment="true" lang="fr">{}{}</AdaptationSet></Period>"#,
        rep(7, 48_000),
        rep(8, 96_013)
    );
    let text = mpd_text("manifest.mpd").replacen("</Period>", &second, 1);
    let mut origin = Origin::new().doc(URL, text.into_bytes());
    for id in [7, 8] {
        origin = origin.doc(
            &format!("http://dash.test/r{id}-init.m4s"),
            test_fixtures::read("dash/init-stream1.m4s"),
        );
        for n in 1..=4 {
            origin = origin.doc(
                &format!("http://dash.test/r{id}-{n:05}.m4s"),
                test_fixtures::read(&format!("dash/chunk-stream1-{n:05}.m4s")),
            );
        }
    }
    let attempts: Attempts = Rc::new(RefCell::new(Vec::new()));
    let log = attempts.clone();
    let mut done = false;
    let hook = move |c: &mut DashClient, step: usize| {
        if !done && step >= 6 {
            let r = c.select_audio(Some(1));
            done = r.is_ok();
            log.borrow_mut().push(r);
        }
    };
    let run = drive(
        URL,
        one_at_a_time(),
        &mut origin,
        &mut Drive {
            hook: Some(Box::new(hook)),
            ..Drive::default()
        },
    );
    assert!(run.ended && run.errors.is_empty(), "{:?}", run.errors);
    assert!(attempts.borrow().iter().any(Result::is_ok));
    assert!(
        run.requested("r8-") > 0,
        "the 96 kbit/s Representation is used"
    );
    assert_eq!(run.requested("r7-"), 0, "never the lower one");
}

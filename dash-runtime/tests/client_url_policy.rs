//! URL policy of `DashClient`: the origin pin and the http/https-only rule on
//! `Location`, redirect final URLs, BaseURL, templates and BaseURL failover.

mod common;

use common::loopback::{Drive, Fault, FaultKind, Origin, drive, mpd_text};
use dash_runtime::client::{
    Action, DashClient, DashConfig, DropCause, FatalKind, Output, PullLimits, ResponseMeta,
};
use dash_runtime::error::Unsupported;

const DOC: &str = "http://dash.test/live/manifest.mpd";
const AST_UNIX_S: u64 = 1_767_225_600;
const SECOND: u64 = 1_000_000;
const MALICIOUS: &str = "http://169.254.169.254/latest/";

fn pinned() -> DashConfig {
    let mut d = DashConfig::default();
    d.cross_origin_urls = false;
    d
}

fn live_mpd(children: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\" \
         profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" availabilityStartTime=\"2026-01-01T00:00:00Z\" \
         publishTime=\"2026-01-01T00:01:00Z\" minimumUpdatePeriod=\"PT2S\" minBufferTime=\"PT2S\" \
         timeShiftBufferDepth=\"PT30S\">{children}<Period id=\"0\" start=\"PT0S\">\
         <AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\
         <SegmentTemplate timescale=\"1000\" duration=\"2000\" startNumber=\"1\" \
         initialization=\"init.mp4\" media=\"seg$Number$.m4s\"/>\
         <Representation id=\"V\" bandwidth=\"1000\" codecs=\"avc1.4d400d\" width=\"320\" height=\"240\"/>\
         </AdaptationSet></Period></MPD>"
    )
}

fn static_mpd(base: &str, media: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
         profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" mediaPresentationDuration=\"PT6S\" \
         minBufferTime=\"PT2S\"><Period id=\"0\">{base}\
         <AdaptationSet mimeType=\"video/mp4\" contentType=\"video\">\
         <SegmentTemplate timescale=\"1000\" duration=\"2000\" startNumber=\"1\" \
         initialization=\"init.mp4\" media=\"{media}\"/>\
         <Representation id=\"V\" bandwidth=\"1000\" codecs=\"avc1.4d400d\" width=\"320\" height=\"240\"/>\
         </AdaptationSet></Period></MPD>"
    )
}

struct Live {
    client: DashClient,
    now: u64,
}

impl Live {
    fn start(dash: DashConfig, mpd: &str, meta: &ResponseMeta<'_>) -> Self {
        let mut client = DashClient::new(DOC, dash, PullLimits::default());
        let now = 0;
        client.set_wall_clock(now, (AST_UNIX_S + 60) * SECOND);
        let Some(Action::FetchMpd { url, .. }) = client.poll(now) else {
            panic!("first action is the MPD")
        };
        assert_eq!(url, DOC);
        client.on_mpd_with(now, mpd.as_bytes(), meta).unwrap();
        Self { client, now }
    }

    fn actions_until_next_mpd(&mut self) -> (Vec<Action>, Option<String>) {
        let mut seen = Vec::new();
        for _ in 0..20 {
            while let Some(a) = self.client.poll(self.now) {
                if let Action::FetchMpd { url, .. } = &a {
                    let url = url.clone();
                    seen.push(a);
                    return (seen, Some(url));
                }
                seen.push(a);
            }
            self.now += 3 * SECOND;
        }
        (seen, None)
    }

    fn refresh_with(&mut self, mpd: &str, meta: &ResponseMeta<'_>) {
        self.client
            .on_mpd_with(self.now, mpd.as_bytes(), meta)
            .unwrap();
    }
}

fn plain() -> ResponseMeta<'static> {
    ResponseMeta::at(0)
}

#[test]
fn a_foreign_location_is_not_followed_when_pinned() {
    let mpd = live_mpd(&format!("<Location>{MALICIOUS}</Location>"));
    let mut live = Live::start(pinned(), &mpd, &plain());
    let (seen, next) = live.actions_until_next_mpd();
    assert_eq!(next.as_deref(), Some(DOC), "{seen:?}");
}

#[test]
fn a_foreign_location_is_followed_when_cross_origin_urls_are_on() {
    let mpd = live_mpd("<Location>http://cdn.other.test/live/updates.mpd</Location>");
    let mut live = Live::start(DashConfig::default(), &mpd, &plain());
    let (_, next) = live.actions_until_next_mpd();
    assert_eq!(
        next.as_deref(),
        Some("http://cdn.other.test/live/updates.mpd")
    );
}

#[test]
fn a_same_origin_and_a_relative_location_pass_the_pin() {
    for (loc, want) in [
        ("http://dash.test/live/u.mpd", "http://dash.test/live/u.mpd"),
        (
            "http://DASH.test:80/live/u.mpd",
            "http://dash.test/live/u.mpd",
        ),
        ("u2.mpd", "http://dash.test/live/u2.mpd"),
        ("/root/u3.mpd", "http://dash.test/root/u3.mpd"),
    ] {
        let mpd = live_mpd(&format!("<Location>{loc}</Location>"));
        let mut live = Live::start(pinned(), &mpd, &plain());
        let (_, next) = live.actions_until_next_mpd();
        assert_eq!(next.as_deref(), Some(want), "{loc}");
    }
}

#[test]
fn disguised_foreign_locations_are_refused_when_pinned() {
    for loc in [
        "//169.254.169.254/latest/",
        "HTTP://169.254.169.254/latest/",
        "http://dash.test@169.254.169.254/latest/",
        "http://user:pw@169.254.169.254/",
        "https://dash.test/live/u.mpd",
        "http://dash.test:8080/live/u.mpd",
        "http://dash.test./live/u.mpd",
        "http://d\u{e4}sh.test/live/u.mpd",
    ] {
        let mpd = live_mpd(&format!("<Location>{loc}</Location>"));
        let mut live = Live::start(pinned(), &mpd, &plain());
        let (seen, next) = live.actions_until_next_mpd();
        assert_eq!(next.as_deref(), Some(DOC), "{loc}: {seen:?}");
    }
}

#[test]
fn a_non_http_location_never_becomes_an_action() {
    for loc in [
        "file:///etc/passwd",
        "ftp://dash.test/live/u.mpd",
        "data:text/plain;base64,AAAA",
    ] {
        for dash in [pinned(), DashConfig::default()] {
            let mpd = live_mpd(&format!("<Location>{loc}</Location>"));
            let mut live = Live::start(dash, &mpd, &plain());
            let (seen, next) = live.actions_until_next_mpd();
            assert_eq!(next.as_deref(), Some(DOC), "{loc}");
            assert!(
                seen.iter().all(|a| !a.url().starts_with("file:")
                    && !a.url().starts_with("ftp:")
                    && !a.url().starts_with("data:")),
                "{loc}: {seen:?}"
            );
        }
    }
}

#[test]
fn a_foreign_final_url_is_not_adopted_when_pinned() {
    let mpd = live_mpd("");
    let meta = ResponseMeta::at(0).with_final_url("http://evil.test/live/manifest.mpd");
    let mut live = Live::start(pinned(), &mpd, &meta);
    let (seen, next) = live.actions_until_next_mpd();
    assert_eq!(next.as_deref(), Some(DOC));
    assert!(
        seen.iter()
            .filter(|a| !matches!(a, Action::FetchMpd { .. }))
            .all(|a| a.url().starts_with("http://dash.test/")),
        "the pin origin moved: {seen:?}"
    );
}

#[test]
fn a_foreign_final_url_on_a_refresh_is_not_adopted_when_pinned() {
    let mpd = live_mpd("");
    let mut live = Live::start(pinned(), &mpd, &plain());
    let (_, next) = live.actions_until_next_mpd();
    assert_eq!(next.as_deref(), Some(DOC));
    let meta = ResponseMeta::at(live.now).with_final_url(MALICIOUS);
    live.refresh_with(&mpd, &meta);
    let (_, next) = live.actions_until_next_mpd();
    assert_eq!(next.as_deref(), Some(DOC));
}

#[test]
fn a_foreign_final_url_is_adopted_when_cross_origin_urls_are_on() {
    let mpd = live_mpd("");
    let meta = ResponseMeta::at(0).with_final_url("http://cdn.other.test/live/moved.mpd");
    let mut live = Live::start(DashConfig::default(), &mpd, &meta);
    let (_, next) = live.actions_until_next_mpd();
    assert_eq!(
        next.as_deref(),
        Some("http://cdn.other.test/live/moved.mpd")
    );
}

#[test]
fn a_same_origin_final_url_is_adopted_when_pinned() {
    let mpd = live_mpd("");
    let meta = ResponseMeta::at(0).with_final_url("http://dash.test/live/moved.mpd");
    let mut live = Live::start(pinned(), &mpd, &meta);
    let (_, next) = live.actions_until_next_mpd();
    assert_eq!(next.as_deref(), Some("http://dash.test/live/moved.mpd"));
}

#[test]
fn a_non_http_final_url_is_never_adopted() {
    for dash in [pinned(), DashConfig::default()] {
        let mpd = live_mpd("");
        let meta = ResponseMeta::at(0).with_final_url("file:///etc/passwd");
        let mut live = Live::start(dash, &mpd, &meta);
        let (_, next) = live.actions_until_next_mpd();
        assert_eq!(next.as_deref(), Some(DOC));
    }
}

#[test]
fn a_foreign_final_url_of_a_static_mpd_is_not_adopted_when_pinned() {
    let mpd = static_mpd("", "seg$Number$.m4s");
    let mut client = DashClient::new(DOC, pinned(), PullLimits::default());
    let Some(Action::FetchMpd { .. }) = client.poll(0) else {
        panic!("mpd")
    };
    let meta = ResponseMeta::at(0).with_final_url("http://evil.test/live/manifest.mpd");
    client.on_mpd_with(0, mpd.as_bytes(), &meta).unwrap();
    let urls: Vec<String> = std::iter::from_fn(|| client.poll(0))
        .map(|a| a.url().to_owned())
        .collect();
    assert!(!urls.is_empty());
    assert!(
        urls.iter().all(|u| u.starts_with("http://dash.test/")),
        "{urls:?}"
    );
}

fn static_urls_and_outcome(base: &str, media: &str) -> (Vec<String>, Vec<DropCause>) {
    let mpd = static_mpd(base, media);
    let mut client = DashClient::new(DOC, DashConfig::default(), PullLimits::default());
    let Some(Action::FetchMpd { .. }) = client.poll(0) else {
        panic!("mpd")
    };
    client.on_mpd(0, mpd.as_bytes()).unwrap();
    let urls: Vec<String> = std::iter::from_fn(|| client.poll(0))
        .map(|a| a.url().to_owned())
        .collect();
    let mut drops = Vec::new();
    let mut fatal = None;
    while let Some(o) = client.next_output() {
        match o {
            Output::TrackDropped { cause, .. } => drops.push(cause),
            Output::Fatal(f) => fatal = Some(f),
            _ => {}
        }
    }
    assert_eq!(
        fatal,
        Some(FatalKind::MpdInvalid {
            reason: "no_playable_representation"
        })
    );
    (urls, drops)
}

#[test]
fn a_file_base_url_is_never_requested() {
    let (urls, drops) =
        static_urls_and_outcome("<BaseURL>file:///var/</BaseURL>", "seg$Number$.m4s");
    assert!(urls.iter().all(|u| !u.starts_with("file:")), "{urls:?}");
    assert!(drops.contains(&DropCause::UnusableUrl), "{drops:?}");
}

#[test]
fn a_data_url_template_is_never_requested_and_reports_data_url() {
    let (urls, drops) = static_urls_and_outcome("", "data:video/mp4;base64,AAAA$Number$");
    assert!(urls.iter().all(|u| !u.starts_with("data:")), "{urls:?}");
    assert!(
        drops.contains(&DropCause::Unsupported(Unsupported::DataUrl)),
        "{drops:?}"
    );
}

#[test]
fn a_data_url_init_is_never_requested() {
    let mpd = static_mpd("", "seg$Number$.m4s").replace("init.mp4", "data:video/mp4;base64,AAAA");
    let mut client = DashClient::new(DOC, DashConfig::default(), PullLimits::default());
    let Some(Action::FetchMpd { .. }) = client.poll(0) else {
        panic!("mpd")
    };
    client.on_mpd(0, mpd.as_bytes()).unwrap();
    let urls: Vec<String> = std::iter::from_fn(|| client.poll(0))
        .map(|a| a.url().to_owned())
        .collect();
    assert!(urls.iter().all(|u| !u.starts_with("data:")), "{urls:?}");
}

const DIR: &str = "timeline-ffmpeg-quirk";
const URL: &str = "http://dash.test/timeline-ffmpeg-quirk/manifest.mpd";

fn failover_text() -> String {
    let t = mpd_text("timeline-ffmpeg-quirk/manifest.mpd").replace(
        "<MPD ",
        "<MPD xmlns:dvb=\"urn:dvb:dash:dash-extensions:2014-1\" ",
    );
    t.replacen(
        "<Period id=\"0\"",
        "<BaseURL dvb:priority=\"1\" serviceLocation=\"a\">http://dash.test/timeline-ffmpeg-quirk/</BaseURL>\
         <BaseURL dvb:priority=\"2\" serviceLocation=\"b\">http://b.test/</BaseURL><Period id=\"0\"",
        1,
    )
}

fn failover_origin() -> Origin {
    Origin::new()
        .doc(URL, failover_text().into_bytes())
        .mount("b.test", DIR)
        .fault(Fault::times(
            "dash.test/timeline-ffmpeg-quirk/chunk-stream0-00002",
            FaultKind::Status(503),
            2,
        ))
}

#[test]
fn pinned_failover_never_rotates_onto_a_foreign_origin() {
    let mut origin = failover_origin();
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive {
            dash: pinned(),
            ..Drive::default()
        },
    );
    assert!(run.fatal.is_none(), "{:?}", run.fatal);
    assert!(run.ended);
    assert_eq!(run.requested("b.test"), 0, "{:?}", run.requests);
    assert_eq!(
        run.requested("dash.test/timeline-ffmpeg-quirk/chunk-stream0-00002"),
        3
    );
}

#[test]
fn unpinned_failover_still_rotates_to_the_secondary() {
    let mut origin = failover_origin();
    let run = drive(
        URL,
        PullLimits::default(),
        &mut origin,
        &mut Drive::default(),
    );
    assert!(run.ended, "{:?}", run.fatal);
    assert_eq!(
        run.requested("dash.test/timeline-ffmpeg-quirk/chunk-stream0-00002"),
        2
    );
    assert_eq!(run.requested("b.test/chunk-stream0-00002"), 1);
}

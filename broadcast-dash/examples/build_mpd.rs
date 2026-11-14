//! Build a small static MPD in code — one video and one audio Representation,
//! each with a `SegmentTemplate` — and render it with `Mpd::to_xml`.
//!
//! Run with `cargo run -p broadcast-dash --example build_mpd`.

use broadcast_common::timescale::Timescale;
use broadcast_dash::{
    AdaptationSet, CommonAttributes, Mpd, Period, PresentationType, Representation, SegmentTemplate,
};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn main() {
    let mut video = Representation::default();
    video.id = "v0".to_string();
    video.bandwidth = 2_000_000;
    let mut vc = CommonAttributes::default();
    vc.codecs = Some("avc1.42E01E".to_string());
    vc.mime_type = Some("video/mp4".to_string());
    vc.width = Some(1280);
    vc.height = Some(720);
    vc.frame_rate = Some("30".to_string());
    video.common = vc;
    let mut tmpl = SegmentTemplate::default();
    tmpl.media = Some("chunk-$Number$.m4s".to_string());
    tmpl.initialization = Some("init.mp4".to_string());
    tmpl.multi.base.timescale = Some(timescale(90000));
    tmpl.multi.duration = Some(90000);
    tmpl.multi.start_number = Some(1);
    video.segment.template = Some(tmpl);

    let mut audio = Representation::default();
    audio.id = "a0".to_string();
    audio.bandwidth = 128_000;
    let mut ac = CommonAttributes::default();
    ac.codecs = Some("mp4a.40.2".to_string());
    ac.mime_type = Some("audio/mp4".to_string());
    ac.audio_sampling_rate = Some("48000".to_string());
    audio.common = ac;
    let mut atmpl = SegmentTemplate::default();
    atmpl.media = Some("audio-chunk-$Number$.m4s".to_string());
    atmpl.initialization = Some("audio-init.mp4".to_string());
    atmpl.multi.base.timescale = Some(timescale(48000));
    atmpl.multi.duration = Some(48000);
    atmpl.multi.start_number = Some(1);
    audio.segment.template = Some(atmpl);

    let mut video_set = AdaptationSet::default();
    video_set.id = Some(0);
    video_set.content_type = Some("video".to_string());
    video_set.representations.push(video);

    let mut audio_set = AdaptationSet::default();
    audio_set.id = Some(1);
    audio_set.content_type = Some("audio".to_string());
    audio_set.representations.push(audio);

    let mut period = Period::default();
    period.id = Some("p0".to_string());
    period.adaptation_sets.push(video_set);
    period.adaptation_sets.push(audio_set);

    let mut mpd = Mpd::default();
    mpd.id = Some("demo".to_string());
    mpd.profiles = vec!["urn:mpeg:dash:profile:isoff-on-demand:2011".to_string()];
    mpd.kind = PresentationType::Static;
    mpd.media_presentation_duration = Some(std::time::Duration::from_secs(60));
    mpd.min_buffer_time = Some(std::time::Duration::from_secs(2));
    mpd.periods.push(period);

    let xml = mpd.to_xml().expect("render");
    print!("{xml}");
    assert_eq!(Mpd::parse(xml.as_bytes()).expect("re-parse"), mpd);
}

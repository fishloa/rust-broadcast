use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;

const OVERLAPPING_NEGATIVE_REPEAT: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="1984-05-18T22:26:08Z" mediaPresentationDuration="PT5278190S">
  <Period start="PT280064S"><AdaptationSet><Representation id="r" bandwidth="1">
    <SegmentTemplate timescale="95489" startNumber="0" endNumber="206175207424" media="$Time$.m4s">
      <SegmentTimeline><S d="473472053551" r="-1"/><S t="1646" d="465709505827" r="896"/></SegmentTimeline>
    </SegmentTemplate>
  </Representation></AdaptationSet></Period></MPD>"#;

#[test]
fn negative_repeat_cut_by_next_t_ends_where_the_next_run_starts() {
    let mpd = Mpd::parse(OVERLAPPING_NEGATIVE_REPEAT.as_bytes()).unwrap();
    let planned = plan_representation(&mpd, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    let plan = &planned.plan;
    let first = plan.get(0).unwrap().unwrap();
    let second = plan.get(1).unwrap().unwrap();
    assert_eq!(first.time_ticks + first.duration_ticks, second.time_ticks);
    assert!(second.available_from_us >= first.available_from_us);
}

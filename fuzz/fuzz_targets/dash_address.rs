#![no_main]

use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;
use dash_runtime::timing::live_edge::{join_index, newest_complete, oldest_in_window, window};
use libfuzzer_sys::fuzz_target;

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";
const MAX_ENTRIES: usize = 12;
const WALK: u64 = 256;

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u8(&mut self) -> u8 {
        let (first, rest) = self.0.split_first().map_or((0, self.0), |(b, r)| (*b, r));
        self.0 = rest;
        first
    }

    fn u64(&mut self) -> u64 {
        (0..8).fold(0u64, |acc, _| (acc << 8) | u64::from(self.u8()))
    }

    fn small(&mut self, modulus: u64) -> u64 {
        self.u64() % modulus.max(1)
    }
}

// Fuzz Segment addressing with arbitrary numbers: build an MPD around them,
// plan the Representation, and check the plan's own invariants (dense indices,
// strictly increasing numbers, non-decreasing time and availability, window
// and join inside their bounds) on top of "no panic, bounded work".
fuzz_target!(|data: &[u8]| {
    let mut b = Bytes(data);
    let dynamic = b.u8() & 1 == 1;
    let kind = b.u8() % 3;
    let timescale = 1 + b.small(200_000);
    let duration = 1 + b.small(u64::from(u32::MAX));
    let start_number = b.u64() % 2 * b.small(1 << 40);
    let end_number = (b.u8() & 1 == 1).then(|| b.small(1 << 41));
    let pto = b.u64() % 2 * b.small(u64::MAX / 2);
    let period_start = b.small(1_000_000);
    let period_duration = (b.u8() & 1 == 1).then(|| b.small(10_000_000));
    let tsb = (b.u8() & 1 == 1).then(|| b.small(100_000));
    let mpd_duration = b.small(10_000_000);
    let ast_s = b.small(4_000_000_000);
    let now_s = b.small(5_000_000_000);
    let delay_s = b.small(1_000);

    let mut timeline = String::new();
    for _ in 0..(b.u8() as usize % MAX_ENTRIES) {
        let t = (b.u8() & 1 == 1).then(|| b.u64());
        let n = (b.u8() & 7 == 0).then(|| b.small(1 << 41));
        let d = 1 + b.small(1 << 40);
        let r = match b.u8() % 4 {
            0 => -1,
            1 => i64::MAX,
            2 => b.small(1_000) as i64,
            _ => 0,
        };
        let t_attr = t.map_or(String::new(), |t| format!(r#" t="{t}""#));
        let n_attr = n.map_or(String::new(), |n| format!(r#" n="{n}""#));
        timeline.push_str(&format!(r#"<S{t_attr}{n_attr} d="{d}" r="{r}"/>"#));
    }
    let end_attr = end_number.map_or(String::new(), |e| format!(r#" endNumber="{e}""#));
    let common = format!(
        r#" timescale="{timescale}" startNumber="{start_number}" presentationTimeOffset="{pto}"{end_attr}"#
    );
    let segment = match kind {
        0 => format!(r#"<SegmentTemplate{common} duration="{duration}" media="$Number$.m4s"/>"#),
        1 => format!(
            r#"<SegmentTemplate{common} media="$Time$.m4s"><SegmentTimeline>{timeline}</SegmentTimeline></SegmentTemplate>"#
        ),
        _ => format!(
            r#"<SegmentList{common} duration="{duration}"><SegmentURL media="a"/><SegmentURL media="b"/></SegmentList>"#
        ),
    };
    let type_attr = if dynamic { "dynamic" } else { "static" };
    let tsb_attr = tsb.map_or(String::new(), |t| format!(r#" timeShiftBufferDepth="PT{t}S""#));
    let pd_attr = period_duration.map_or(String::new(), |d| format!(r#" duration="PT{d}S""#));
    let ast = jiff::Timestamp::from_second(i64::try_from(ast_s).unwrap_or(0)).unwrap_or_default();
    let xml = format!(
        r#"<MPD xmlns="{NS}" type="{type_attr}" availabilityStartTime="{ast}" mediaPresentationDuration="PT{mpd_duration}S"{tsb_attr}>
           <Period start="PT{period_start}S"{pd_attr}><AdaptationSet><Representation id="r" bandwidth="1">{segment}</Representation></AdaptationSet></Period></MPD>"#
    );
    let Ok(mpd) = Mpd::parse(xml.as_bytes()) else { return };
    let Ok(planned) = plan_representation(&mpd, RepRef::at(0, 0, 0), &PlanOptions::default()) else {
        return;
    };
    let plan = &planned.plan;

    let mut previous = None;
    for i in 0..WALK.min(plan.len().unwrap_or(WALK)) {
        let seg = plan.get(i).expect("index below len() exists");
        assert_eq!(seg.index, i);
        if let Some((n, t, from)) = previous {
            assert!(seg.number > n || kind == 2 || end_number.is_some(), "numbers increase");
            if kind != 2 {
                assert!(seg.time_ticks >= t, "time never decreases");
            }
            if let (Some(a), Some(b)) = (from, seg.available_from_us) {
                assert!(b >= a, "availability never decreases");
            }
        }
        previous = Some((seg.number, seg.time_ticks, seg.available_from_us));
    }
    if let Some(n) = plan.len() {
        assert!(plan.get(n).is_none());
    }

    if plan.is_dynamic() {
        let now = i128::from(now_s) * 1_000_000;
        let delay = i128::from(delay_s) * 1_000_000;
        let newest = newest_complete(plan, now);
        if let Some(k) = newest {
            assert!(plan.get(k).unwrap().available_from_us.unwrap() <= now);
            if let Some(next) = plan.get(k + 1) {
                assert!(next.available_from_us.unwrap() > now, "newest is the last complete one");
            }
        }
        let _ = oldest_in_window(plan, now);
        if let Some(w) = window(plan, now) {
            assert!(w.oldest <= w.newest);
            let j = join_index(plan, now, delay).unwrap();
            assert!(j >= w.oldest && j <= w.newest);
        }
    }
});

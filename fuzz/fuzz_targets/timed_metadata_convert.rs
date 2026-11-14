#![no_main]

use broadcast_common::timescale::Timescale;
use libfuzzer_sys::fuzz_target;
use mp4_emsg::EmsgBox;
use timed_metadata::DateRange;
use timed_metadata::convert::emsg_to_scte35;
use timed_metadata::convert::{SegmentTiming, emsg_to_v0, emsg_to_v1};

const TIMING_LEN: usize = 16;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = core::str::from_utf8(data)
        && let Ok(range) = DateRange::parse_tag_line(text)
        && let Ok(line) = range.to_tag_line()
    {
        let _ = DateRange::parse_tag_line(&line);
    }

    let _ = emsg_to_scte35(data);

    if data.len() < TIMING_LEN {
        return;
    }
    let (head, body) = data.split_at(TIMING_LEN);
    let Some(timescale) = Timescale::new(u32::from_le_bytes(head[0..4].try_into().unwrap())) else {
        return;
    };
    let timing = SegmentTiming {
        earliest_presentation_time: u64::from_le_bytes(head[0..8].try_into().unwrap()),
        presentation_time_offset: u64::from_le_bytes(head[8..16].try_into().unwrap()),
        timescale,
    };
    let Ok(emsg) = EmsgBox::parse(body) else {
        return;
    };
    let _ = emsg_to_v1(&emsg, &timing);
    let _ = emsg_to_v0(&emsg, &timing);
});

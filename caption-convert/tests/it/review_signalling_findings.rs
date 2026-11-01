//! Adversarial-review findings (review/signalling-captions): FAILING
//! regressions for confirmed defects; production code is untouched.

use caption_convert::parse_srt;

/// #1210 moved the SRT timestamp parser onto
/// `timed_metadata::webvtt::parse_timestamp(s, ',')`. The deleted
/// `caption-convert::time` path normalised `,` -> `.` and parsed on `.`, so an
/// SRT using the (common in the wild) `.` millisecond separator was accepted
/// by 0.2.0. It is now rejected, a behaviour change the CHANGELOG lists only
/// as "Internal". Either restore the tolerance or document the break.
#[test]
fn srt_with_dot_millisecond_separator_still_parses() {
    let srt = "1\n00:00:01.000 --> 00:00:02.500\nhello\n\n";
    let cues = parse_srt(srt).expect("0.2.0 accepted '.' in SRT timestamps");
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].start.0, 90_000);
}

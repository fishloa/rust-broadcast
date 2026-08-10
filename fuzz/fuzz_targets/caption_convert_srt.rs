#![no_main]

use caption_convert::{parse_srt, srt_to_webvtt, write_srt};
use libfuzzer_sys::fuzz_target;

// Fuzz `caption-convert`'s SRT parser (`src/srt.rs`, `src/time.rs`) on
// arbitrary UTF-8 text. Must not panic on any input, however malformed.
//
// Beyond no-panic, this asserts the same text-format round-trip invariant as
// the `caption_convert_webvtt` target: SRT has no formal spec, but this
// crate's own de facto grammar is deterministic, and `parse_srt` splits
// blocks on a literal `"\n\n"`, so a `Cue`'s `text` can never itself contain
// a blank-line pair. `write_srt(&cues)` re-parsed must therefore reproduce
// the exact same `Cue` list (mirroring `srt.rs`'s own
// `write_then_parse_round_trips` unit test, generalised to arbitrary input).
fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Ok(input) = core::str::from_utf8(data) else {
        return;
    };

    let Ok(cues) = parse_srt(input) else {
        return;
    };

    let rewritten = write_srt(&cues);
    let reparsed = match parse_srt(&rewritten) {
        Ok(r) => r,
        Err(e) => panic!(
            "write_srt produced a document parse_srt could not re-parse: \
             {e:?}\ninput: {input:?}\nrewritten: {rewritten:?}"
        ),
    };
    assert_eq!(
        reparsed, cues,
        "srt round-trip: cues differ after write_srt -> parse_srt\n\
         input: {input:?}\nrewritten: {rewritten:?}"
    );

    // SRT -> WebVTT is documented lossless (issue #931: SRT has no
    // construct WebVTT cannot represent) — round-tripping through WebVTT and
    // back to SRT must therefore reproduce the identical Cue list too.
    let vtt = srt_to_webvtt(input).expect("srt_to_webvtt must accept anything parse_srt accepted");
    let (back, lossy) =
        caption_convert::webvtt_to_srt(&vtt).expect("webvtt_to_srt must accept its own writer's output");
    assert!(
        !lossy,
        "srt -> webvtt round-trip introduced a construct webvtt_to_srt calls lossy\n\
         input: {input:?}\nvtt: {vtt:?}"
    );
    let back_cues = parse_srt(&back).expect("webvtt_to_srt always produces valid SRT");
    assert_eq!(
        back_cues, cues,
        "srt -> webvtt -> srt lost cues\ninput: {input:?}\nvtt: {vtt:?}"
    );
});

#![no_main]

use caption_convert::{parse_webvtt, webvtt_to_srt, write_document};
use libfuzzer_sys::fuzz_target;

// Fuzz `caption-convert`'s WebVTT parser (`src/webvtt.rs`, `src/time.rs`) on
// arbitrary UTF-8 text. Must not panic on any input, however malformed.
//
// Beyond no-panic, this asserts the text-format round-trip invariant
// CRATE-ACCEPTANCE.md requires for markup formats: "parse -> serialize ->
// re-parse must yield an equal document". `write_document` never emits a
// construct (`NOTE`/`STYLE`/`REGION`, cue identifiers, cue settings) that
// `parse_webvtt` doesn't accept back, and a `Cue`'s `text` field — built by
// `parse_webvtt` splitting on blank lines — can never itself contain a blank
// or whitespace-only line, so `write_document(&cues)` re-parsed must
// reproduce the exact same `Cue` list. This is the generalisation of
// `lib.rs`'s own `multiline_payload_round_trips_through_write_and_parse` unit
// test to arbitrary input.
fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Ok(input) = core::str::from_utf8(data) else {
        return;
    };

    let Ok(parsed) = parse_webvtt(input) else {
        return;
    };

    let rewritten = write_document(&parsed.cues);
    let reparsed = match parse_webvtt(&rewritten) {
        Ok(r) => r,
        Err(e) => panic!(
            "write_document produced a document parse_webvtt could not \
             re-parse: {e:?}\ninput: {input:?}\nrewritten: {rewritten:?}"
        ),
    };
    assert_eq!(
        reparsed.cues, parsed.cues,
        "webvtt round-trip: cues differ after write_document -> parse_webvtt\n\
         input: {input:?}\nrewritten: {rewritten:?}"
    );

    // Lossless WebVTT -> SRT -> WebVTT must preserve every cue (issue #931:
    // `webvtt_to_srt`'s `lossy` flag is the caller's promise that nothing
    // SRT-incompatible was dropped).
    if let Ok((srt, lossy)) = webvtt_to_srt(input) {
        if !lossy {
            let back = match caption_convert::srt_to_webvtt(&srt) {
                Ok(b) => b,
                Err(e) => panic!(
                    "srt_to_webvtt failed on webvtt_to_srt's own (lossless) \
                     output: {e:?}\nsrt: {srt:?}"
                ),
            };
            let round_tripped = parse_webvtt(&back)
                .expect("srt_to_webvtt always produces a valid WEBVTT document");
            assert_eq!(
                round_tripped.cues, parsed.cues,
                "webvtt -> srt -> webvtt lost cues despite lossy=false\n\
                 input: {input:?}\nsrt: {srt:?}"
            );
        }
    }
});

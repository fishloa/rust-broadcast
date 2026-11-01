//! No-panic hostility sweep (audit S3) of the text parsers: seed WebVTT / SRT
//! documents (header metadata, STYLE/REGION/NOTE blocks, cue identifiers, cue
//! settings, inline tags, `&amp;` escapes, CRLF/CR line endings, hour and
//! no-hour timestamps) are truncated at every char boundary and byte-mutated
//! with a fixed LCG (so a failure is reproducible), then pushed through every
//! parse and conversion entry point. The contract is "`Ok` or `Err`, never a
//! panic" - timestamp arithmetic on absurd values (`99:59:59.999`, 20-digit
//! numbers) and string slicing on multi-byte characters are where a text
//! parser panics.

use caption_convert::{parse_srt, parse_webvtt, srt_to_webvtt, webvtt_to_srt};

const SEEDS: &[&str] = &[
    "WEBVTT - title\nKind: captions\nLanguage: en\n\nNOTE a comment\n\nSTYLE\n::cue { color: red }\n\nREGION\nid:r1\nwidth:40%\n\n1\n00:00:01.000 --> 00:00:03.500 line:80% align:center\nHello &amp; <b>world</b>\n\n00:03.500 --> 00:05.000\nSecond cue\nline two\n\n",
    "WEBVTT\r\n\r\n00:00:00.000 --> 00:00:01.000\r\nCRLF cue\r\n\r\n",
    "WEBVTT\r\r00:00:00.000 --> 00:00:01.000\rCR only\r\r",
    "1\n00:00:01,000 --> 00:00:03,500\nHello <i>world</i>\n\n2\n00:00:04,000 --> 00:00:05,250\nLine one\nLine two\n\n3\n99:59:59,999 --> 100:00:00,000\nbig\n\n",
    "1\r\n00:00:01,000 --> 00:00:02,000\r\n\u{00e9}\u{4e2d}\u{1f600} multi-byte\r\n\r\n",
];

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn exercise(s: &str) {
    let _ = parse_srt(s);
    let _ = parse_webvtt(s);
    let _ = srt_to_webvtt(s);
    let _ = webvtt_to_srt(s);
}

#[test]
fn seeds_parse_at_least_one_way() {
    // The sweep is only meaningful if the seeds reach the parsers' deep paths.
    assert!(parse_webvtt(SEEDS[0]).is_ok());
    assert!(parse_srt(SEEDS[3]).is_ok());
}

#[test]
fn truncation_at_every_char_boundary_never_panics() {
    for seed in SEEDS {
        for (i, _) in seed.char_indices() {
            exercise(&seed[..i]);
        }
        exercise(seed);
    }
}

#[test]
fn random_byte_mutation_never_panics() {
    let mut rng = Lcg(0xFACE_FEED);
    for seed in SEEDS {
        for _ in 0..4_000 {
            let mut b = seed.as_bytes().to_vec();
            for _ in 0..1 + rng.next() % 4 {
                let pos = (rng.next() as usize) % b.len();
                b[pos] = match rng.next() % 6 {
                    0 => b'0',
                    1 => b'9',
                    2 => b':',
                    3 => b'-',
                    4 => 0xFF,
                    _ => rng.next() as u8,
                };
            }
            exercise(&String::from_utf8_lossy(&b));
        }
    }
}

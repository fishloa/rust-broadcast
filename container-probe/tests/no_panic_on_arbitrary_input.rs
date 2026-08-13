//! Robustness: the probe must never panic and must always return a well-formed
//! `Probe` (one of `Identified`/`Ambiguous`/`Insufficient`/`Unknown`) on
//! arbitrary input. Audit finding 7 — there was previously no
//! `no_panic_on_arbitrary_input` test anywhere in the crate.
//!
//! What is fed, per audit finding 7:
//! - every `1..=64`-byte prefix of each real fixture (a truncated prefix is the
//!   most common hostile input — a caller read a few bytes at a time),
//! - 4 KiB of zeros and 4 KiB of `0xFF`,
//! - a deterministic pseudo-random buffer (seeded here; no `rand` dependency).
//!
//! "Well-formed" is the one property that holds for *every* non-panicking
//! `Probe`: it is one of the four variants. We match all four (plus the
//! `#[non_exhaustive]` wildcard) so a structure change that made an input fall
//! through would be a compile-time failure, not a silent pass.

use container_probe::Probe;
use std::fs;

/// Join a workspace-relative fixture path to an absolute path from this crate.
fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// A deterministic pseudo-random bytes generator (xorshift64), seeded here so
/// the test is reproducible without a `rand` dependency. Never zero, never 0xFF
/// everywhere — a plausible high-entropy corpus.
struct Prng(u64);

impl Prng {
    fn next_u8(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 & 0xFF) as u8
    }
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.next_u8();
        }
    }
}

/// Feed `data` to `probe` and assert it returns a well-formed `Probe`.
fn assert_well_formed(data: &[u8]) {
    let p = container_probe::probe(data);
    match &p {
        Probe::Identified { .. } | Probe::Ambiguous { .. } => {
            // Identified is fine (possibly Ambiguous). Nothing more to check.
        }
        Probe::Insufficient { need_at_least, .. } => {
            // A request for more bytes must exceed what was already supplied —
            // a `need_at_least <= data.len()` would tell the caller to fetch
            // bytes it already holds, and is the one property of this variant
            // with value to assert.
            assert!(
                *need_at_least > data.len(),
                "Insufficient must demand more than the {} supplied bytes, got {p:?}",
                data.len()
            );
        }
        Probe::Unknown => {}
        // `#[non_exhaustive]` requires a wildcard arm; reaching it means the
        // enum grew and this match must be revisited.
        _ => panic!("unexpected new Probe variant: {p:?}"),
    }
}

/// Every 1..=64-byte prefix of each real fixture is fed, plus the whole fixture.
#[test]
fn no_panic_on_prefixes_of_every_real_fixture() {
    let fixtures = [
        "fixtures/ts/h264_aac.ts",
        "fixtures/mp4/h264_high.mp4",
        "fixtures/mkv/h264_aac.mkv",
        "fixtures/webm/vorbis.webm",
        "fixtures/mxf/op1a_mpeg2_pcm.mxf",
        "fixtures/ps/h264_ac3.ps",
        "fixtures/flv/av.flv",
        "fixtures/container-probe/pcm_s16le.wav",
        "fixtures/container-probe/opus.ogg",
        "fixtures/container-probe/video.asf",
        "fixtures/container-probe/aac.adts",
        "fixtures/container-probe/audio.mp3",
        "fixtures/container-probe/h264.annexb",
    ];
    for rel in fixtures {
        let full = fixture(rel);
        for len in 1..=64usize.min(full.len()) {
            let prefix = &full[..len];
            assert_well_formed(prefix);
        }
        // The whole file too.
        assert_well_formed(&full);
    }
}

/// 4 KiB of zeros and 4 KiB of `0xFF` must not panic and must return a
/// well-formed `Probe` (in fact both are `Unknown`/`Insufficient`).
#[test]
fn no_panic_on_zero_and_fill() {
    assert_well_formed(&[0u8; 4096]);
    assert_well_formed(&[0xFFu8; 4096]);
}

/// A deterministic pseudo-random buffer must not panic and must return a
/// well-formed `Probe`.
#[test]
fn no_panic_on_deterministic_random() {
    let mut rng = Prng(0x9E3779B97F4A7C15);
    let mut buf = vec![0u8; 8192];
    rng.fill(&mut buf);
    assert_well_formed(&buf);
}

/// Structured length-field mutations: valid magic followed by adversarial
/// length fields, for every prober that decodes one.
///
/// The truncation + random + fill inputs above deliberately cannot reach these
/// code paths — a width-8 EBML VINT, an ISOBMFF `largesize`, an MXF BER length,
/// or an ADTS/MP3 `frame_length` never appears in a 64-byte prefix of the
/// fixtures, in uniform bytes, or at random. The one real input that did reach
/// them — `1A 45 DF A3 01 …` (EBML magic + a width-8 size VINT) — panicked the
/// pre-fix arithmetic (`1 << (7 - width + 1)` underflow). These inputs drive
/// each length decoder directly so a regression that the random corpus cannot
/// see still fails this test.
fn structured_length_mutations() -> Vec<Vec<u8>> {
    let mut out = Vec::new();

    // EBML: magic + an element-size VINT of every width 1..=8. A width-`w`
    // VINT opens with a first byte whose low `8 - w` bit is the length marker
    // (`1 << (8 - w)`), so `vint_width` reports `w`. Width 8 is exactly the
    // underflow case.
    for width in 1..=8u32 {
        let mut buf = vec![0x1A, 0x45, 0xDF, 0xA3]; // EBML magic
        buf.push(1u8 << (8 - width));
        buf.resize(4 + width as usize, 0);
        // Trailing bytes so the header walk has data to scan for a DocType.
        buf.extend_from_slice(b"\x42\x82\x80\x00\x00\x00\x00\x00\x00\x00");
        out.push(buf);
    }

    // ISOBMFF: `size32 == 1` (64-bit `largesize` follows), with adversarial
    // largesize values — including one that cannot fit a `usize` on a 32-bit
    // target, which the decoder must reject rather than truncate.
    let mut large = vec![0u8; 16];
    large[3] = 1; // SIZE_INDICATES_LARGESIZE == 1
    large[4..8].copy_from_slice(b"free");
    large[8..].copy_from_slice(&u64::MAX.to_be_bytes());
    out.push(large);
    // `size32 == 0` (runs to end of file).
    let mut eof = vec![0u8; 8];
    eof[4..8].copy_from_slice(b"free");
    out.push(eof);
    // A size32 below the 8-byte header minimum.
    let mut tiny = vec![0u8; 8];
    tiny[3] = 7;
    tiny[4..8].copy_from_slice(b"free");
    out.push(tiny);

    // MXF: 16-byte partition-pack key, then a BER long-form length whose
    // octet-count field takes every value 1..=9 (9 is out of the legal 1..=8
    // range and must be rejected).
    let mxf_key = [
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    for octets in 1..=9u8 {
        let mut m = mxf_key.to_vec();
        m.push(0x80 | octets); // long form, count = octets
        m.extend(std::iter::repeat_n(0u8, octets as usize + 2));
        out.push(m);
    }

    // ADTS: valid sync + adversarial 13-bit `frame_length` (including the
    // maximum 0x1FFF and a sub-minimum value).
    for fl in [7u16, 0x01FF, 0x1FFF, 0x0800] {
        let mut a = vec![0xFF, 0xF1, 0x00, 0, 0, 0];
        a[3] = ((fl >> 11) & 0x03) as u8;
        a[4] = ((fl >> 3) & 0xFF) as u8;
        a[5] = ((fl & 0x07) as u8) << 5;
        a.extend(std::iter::repeat_n(0x00, 32));
        out.push(a);
    }

    // MP3: valid MPEG-1 Layer III sync + adversarial bitrate/sample-rate/padding
    // bytes (reserved bitrate index, max bitrate, reserved sample rate).
    for (b2, b3) in [(0x54u8, 0x00u8), (0xF4, 0x00), (0x20, 0x02), (0x54, 0x02)] {
        let mut m = vec![0xFF, 0xFB, b2, b3];
        m.extend(std::iter::repeat_n(0x00, 32));
        out.push(m);
    }

    out
}

/// Every structured length-field mutation must be fed and return a well-formed
/// `Probe` — this is the path a random corpus cannot reach.
#[test]
fn no_panic_on_structured_length_mutations() {
    for input in structured_length_mutations() {
        assert_well_formed(&input);
    }
}

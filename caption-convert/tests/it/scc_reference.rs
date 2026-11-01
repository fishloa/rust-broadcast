//! Real Scenarist SCC files as an independent oracle for the CEA-608 -> cue
//! path: libcaption's `tos.scc` (MIT) and pycaption's `example.scc`
//! (Apache-2.0), see `fixtures/captions/PROVENANCE.md`.
//!
//! An SCC file is a list of `timecode<TAB>hex words`, each word being one
//! CEA-608 byte pair (odd parity intact). Feeding those pairs through
//! `Cea608ToWebVtt` must display exactly the text a trivial independent reader
//! of the pairs extracts (CTA-608-E: control pairs start with 0x10-0x1F, text
//! pairs are two printable bytes, an extended-character pair `0x12/0x13 xx`
//! backspaces over the standard-character stand-in that precedes it). The
//! comparison is over ASCII letters and digits only, so the handful of
//! national characters (which map to different glyphs than ASCII) cannot cause
//! a false mismatch, and cue timing is pinned separately.
#![cfg(feature = "cc-data")]

use caption_convert::Cea608ToWebVtt;
use cc_data::decode::Cea608Channel;

const PTS_PER_FRAME: u64 = 3003; // 90 kHz / (30000/1001) fps
const FPS_NOMINAL: u64 = 30;
const PARITY_MASK: u8 = 0x7F;
const CC_DATA_PREFIX: [u8; 3] = [0xC1, 0xFF, 0xFC]; // em, process, count=1; marker; valid field-1
const CC_DATA_MARKER: u8 = 0xFF;

struct SccPair {
    pts: u64,
    b1: u8,
    b2: u8,
}

fn parse_scc(text: &str) -> Vec<SccPair> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let Some((tc, words)) = line.split_once('\t') else {
            continue;
        };
        let f: Vec<u64> = tc
            .split([':', ';'])
            .map(|p| p.parse().expect("timecode field"))
            .collect();
        let frames = ((f[0] * 60 + f[1]) * 60 + f[2]) * FPS_NOMINAL + f[3];
        for (i, w) in words.split_whitespace().enumerate() {
            let v = u16::from_str_radix(w, 16).expect("hex word");
            out.push(SccPair {
                pts: (frames + i as u64) * PTS_PER_FRAME,
                b1: (v >> 8) as u8,
                b2: (v & 0xFF) as u8,
            });
        }
    }
    out
}

const SCREEN_COLS: usize = 32;
const EOC: (u8, u8) = (0x14, 0x2F);

/// One pop-on caption as an independent reader sees it: its ASCII
/// alphanumerics, and whether every row fits the 32-column screen (a row that
/// overflows is clamped by a conforming decoder, which the plain reader does
/// not model, so such captions are excluded from the text comparison).
struct OracleCaption {
    alnum: String,
    fits: bool,
}

/// Split `pairs` into the captions that each EOC (0x14 0x2F) pops on.
fn oracle_captions(pairs: &[SccPair]) -> Vec<OracleCaption> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut col = 0usize;
    let mut fits = true;
    for p in pairs {
        let (a, b) = (p.b1 & PARITY_MASK, p.b2 & PARITY_MASK);
        if (a, b) == EOC {
            let alnum: String = text.chars().filter(char::is_ascii_alphanumeric).collect();
            if !alnum.is_empty() {
                out.push(OracleCaption { alnum, fits });
            }
            text.clear();
            fits = true;
            col = 0;
            continue;
        }
        match a {
            0x10..=0x1F => {
                if (0x40..=0x7F).contains(&b) {
                    // PAC: starts a row; bit 4 selects an indent of 4 * bits 3:1.
                    col = if b & 0x10 != 0 {
                        usize::from((b & 0x0E) >> 1) * 4
                    } else {
                        0
                    };
                } else if (a == 0x17 || a == 0x1F) && (0x21..=0x23).contains(&b) {
                    col += usize::from(b - 0x20); // tab offset
                } else if (a == 0x12 || a == 0x13) && (0x20..=0x3F).contains(&b) {
                    // Extended character: replaces the preceding stand-in.
                    text.pop();
                } else if (a == 0x11 || a == 0x19) && (0x20..=0x3F).contains(&b) {
                    col += 1; // mid-row code or special character
                }
            }
            0x20..=0x7F => {
                text.push(a as char);
                col += 1;
                if b >= 0x20 {
                    text.push(b as char);
                    col += 1;
                }
            }
            _ => {}
        }
        if col > SCREEN_COLS {
            fits = false;
        }
    }
    out
}

fn convert(pairs: &[SccPair]) -> Cea608ToWebVtt {
    let mut conv = Cea608ToWebVtt::new(Cea608Channel::Cc1);
    for p in pairs {
        let mut cc = CC_DATA_PREFIX.to_vec();
        cc.extend([p.b1, p.b2, CC_DATA_MARKER]);
        conv.push_cc_data(p.pts, &cc).expect("valid cc_data()");
    }
    conv.finalize(pairs.last().map_or(0, |p| p.pts) + 90_000);
    conv
}

fn assert_text_matches(path: &str, min_compared: usize) {
    let pairs = parse_scc(&test_fixtures::read_to_string(path));
    let captions = oracle_captions(&pairs);
    let conv = convert(&pairs);
    let cues = conv.cues();
    assert_eq!(
        cues.len(),
        captions.len(),
        "{path}: one cue per non-blank caption"
    );
    let mut compared = 0;
    for (i, (cue, cap)) in cues.iter().zip(&captions).enumerate() {
        if !cap.fits {
            continue;
        }
        let got: String = cue
            .text
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        assert_eq!(got, cap.alnum, "{path}: caption {i}");
        compared += 1;
    }
    assert!(
        compared >= min_compared,
        "{path}: only {compared} captions comparable"
    );
}

#[test]
fn tos_scc_text_matches_the_independent_reader() {
    assert_text_matches("captions/libcaption/tos.scc", 40);
}

#[test]
fn pycaption_example_scc_text_matches_the_independent_reader() {
    assert_text_matches("captions/pycaption/example.scc", 2);
}

#[test]
fn tos_scc_cues_start_at_their_eoc_timecode_and_never_overlap() {
    let pairs = parse_scc(&test_fixtures::read_to_string(
        "captions/libcaption/tos.scc",
    ));
    let conv = convert(&pairs);
    let cues = conv.cues();
    assert!(cues.len() > 40, "{} cues", cues.len());
    for w in cues.windows(2) {
        assert!(w[0].start.0 < w[0].end.0, "zero/negative cue {:?}", w[0]);
        assert!(
            w[0].end.0 <= w[1].start.0,
            "overlap {:?} / {:?}",
            w[0],
            w[1]
        );
    }
    // Each pop-on cue becomes visible exactly when its EOC (0x942F) pair is
    // received - never earlier.
    let eoc_pts: Vec<u64> = pairs
        .iter()
        .filter(|p| p.b1 & PARITY_MASK == 0x14 && p.b2 & PARITY_MASK == 0x2F)
        .map(|p| p.pts)
        .collect();
    for c in cues {
        assert!(
            eoc_pts.contains(&c.start.0),
            "cue {:?} does not start at an EOC pair",
            c
        );
    }
}

//! HLS fixture-corpus conformance test (issue #869).
//!
//! Parses **every** committed `.m3u8` fixture under `fixtures/hls/` with
//! [`broadcast_hls::MediaPlaylist::parse`] / [`broadcast_hls::MasterPlaylist::parse`]
//! and asserts the outcome — either a clean parse, or (for the two spec
//! fixtures that are genuinely unparsable) the exact documented error.
//!
//! Provenance for every fixture lives in `fixtures/hls/MANIFEST.md`; read
//! that file, not this one, for *why* each fixture exists.
//!
//! Fixtures are loaded with `concat!(env!("CARGO_MANIFEST_DIR"), ..)` +
//! `std::fs` at **runtime** — never `include_bytes!`, never a bare relative
//! path, and never a skip-on-missing branch. A missing fixture is a hard
//! `panic!`, not a silently-vacuous pass.
//!
//! # Findings (issue #869)
//!
//! Of the 10 committed §9 spec `.m3u8` fixtures, **8 parse cleanly** and
//! **2 fail**:
//!
//! - `9.10-daterange-scte35.m3u8` — fails at line 2, "media segment URI with
//!   no preceding #EXTINF". Root cause: the spec's own `...` elision marker
//!   (standing in for "Media Segment declarations for 60s worth of media"
//!   that the spec elides for brevity) is not a comment — it doesn't start
//!   with `#` — so the parser reads it as a bare segment URI line. This is
//!   NOT a missing-tag gap; §9.10 is an intentionally-incomplete excerpt, and
//!   no real encoder would ever emit a literal `...` segment URI.
//! - `9.11-low-latency-playlist.m3u8` — fails at line 3, same reason and same
//!   root cause (its leading `...` stands in for "EXT-X-PART tags have been
//!   removed from earlier Parent Segments").
//!
//! Every other spec fixture — including ones exercising tags this crate
//! doesn't structurally model (`EXT-X-MEDIA`, `EXT-X-KEY`,
//! `EXT-X-CONTENT-STEERING`, `EXT-X-DATERANGE`/SCTE-35) — parses without
//! error: unrecognized tags are preserved verbatim into
//! [`broadcast_hls::MediaPlaylist::extra_tags`] (or silently skipped by
//! [`broadcast_hls::MasterPlaylist`], which has no such escape hatch) rather
//! than rejected. So this run surfaces **no** missing-tag parse failures for
//! issue #872 to pick up — the two failures above are a structural property
//! of the spec excerpts, not an implementation gap.

use std::fs;
use std::path::PathBuf;

use broadcast_hls::{MasterPlaylist, MediaPlaylist};

/// `fixtures/hls/`, resolved from this crate's own manifest dir — never a
/// bare relative path (that depends on the caller's cwd).
fn fixtures_hls_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/hls"))
}

/// Read one fixture file by its path relative to `fixtures/hls/`. Panics
/// loudly (never skips) if the file is missing — a missing fixture is a
/// regression, not something to route around.
fn read_fixture(rel_path: &str) -> String {
    let path = fixtures_hls_root().join(rel_path);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read committed fixture {}: {e}", path.display()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Media,
    Master,
}

/// Outcome of parsing a fixture.
enum Expect {
    /// Must parse without error.
    Ok,
    /// Must fail with an error whose Display contains this substring.
    Err(&'static str),
}

struct Case {
    /// Path relative to `fixtures/hls/`.
    path: &'static str,
    kind: Kind,
    expect: Expect,
}

fn assert_case(case: &Case) {
    let text = read_fixture(case.path);
    match (case.kind, &case.expect) {
        (Kind::Media, Expect::Ok) => {
            MediaPlaylist::parse(&text)
                .unwrap_or_else(|e| panic!("{} should parse as a Media Playlist: {e}", case.path));
        }
        (Kind::Master, Expect::Ok) => {
            MasterPlaylist::parse(&text).unwrap_or_else(|e| {
                panic!("{} should parse as a Multivariant Playlist: {e}", case.path)
            });
        }
        (Kind::Media, Expect::Err(reason_substr)) => {
            let err = MediaPlaylist::parse(&text).expect_err(&format!(
                "{} is a known-unparsable spec excerpt (see module docs) and must still fail",
                case.path
            ));
            let msg = err.to_string();
            assert!(
                msg.contains(reason_substr),
                "{}: expected error containing {reason_substr:?}, got {msg:?}",
                case.path
            );
        }
        (Kind::Master, Expect::Err(reason_substr)) => {
            let err = MasterPlaylist::parse(&text).expect_err(&format!(
                "{} is a known-unparsable spec excerpt and must still fail",
                case.path
            ));
            let msg = err.to_string();
            assert!(
                msg.contains(reason_substr),
                "{}: expected error containing {reason_substr:?}, got {msg:?}",
                case.path
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 1 — draft-pantos-hls-rfc8216bis-22 §9 examples (fixtures/hls/spec/)
// ---------------------------------------------------------------------------
//
// §9.8 (Session Data fragment) and §9.9 (bare CHARACTERISTICS attribute
// value) are excluded per MANIFEST.md — neither begins with #EXTM3U, so
// neither is a playlist a parser could ever be expected to accept. §9.12's
// and §9.13's JSON Steering Manifests are committed as `.json`, not parsed
// here (this test is playlist-only); §9.6/§9.7/§9.10/§9.12 additionally carry
// a `.rfc-verbatim.txt` sibling (backslash line-continuations exactly as
// printed in the spec) that is deliberately NOT a `.m3u8` and NOT parsed —
// it exists purely for byte-provenance, see MANIFEST.md.

const SPEC_CASES: &[Case] = &[
    Case {
        path: "spec/9.1-simple-media-playlist.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "spec/9.2-live-media-playlist-https.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "spec/9.3-encrypted-media-segments.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "spec/9.4-multivariant-playlist.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    Case {
        path: "spec/9.5-multivariant-with-iframes.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    Case {
        path: "spec/9.6-multivariant-alternative-audio.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    Case {
        path: "spec/9.7-multivariant-alternative-video.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    // Genuinely unparsable: the spec's own `...` elision marker for elided
    // Media Segment declarations is read as a bare (no preceding #EXTINF)
    // segment URI. See the module doc "Findings" section.
    Case {
        path: "spec/9.10-daterange-scte35.m3u8",
        kind: Kind::Media,
        expect: Expect::Err("no preceding #EXTINF"),
    },
    Case {
        path: "spec/9.11-low-latency-playlist.m3u8",
        kind: Kind::Media,
        expect: Expect::Err("no preceding #EXTINF"),
    },
    Case {
        path: "spec/9.12-content-steering.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
];

#[test]
fn spec_examples_parse_as_expected() {
    for case in SPEC_CASES {
        assert_case(case);
    }
}

/// Directory-completeness guard: every `.m3u8` file actually committed under
/// `fixtures/hls/spec/` must be covered by [`SPEC_CASES`] above — so a
/// fixture added later without a matching test case fails loudly instead of
/// silently going unparsed-and-unchecked forever.
#[test]
fn spec_dir_has_no_untested_m3u8_fixtures() {
    let dir = fixtures_hls_root().join("spec");
    let mut on_disk: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".m3u8"))
        .collect();
    on_disk.sort();

    let mut expected: Vec<String> = SPEC_CASES
        .iter()
        .map(|c| c.path.trim_start_matches("spec/").to_string())
        .collect();
    expected.sort();

    assert_eq!(
        on_disk, expected,
        "fixtures/hls/spec/*.m3u8 on disk must exactly match SPEC_CASES in this test"
    );
}

// ---------------------------------------------------------------------------
// Pre-existing hand-made fixtures (fixtures/hls/*.m3u8) — kept for
// regression coverage; provenance in MANIFEST.md. Four are well-formed
// enough to parse (the deliberate defects they carry — EXTINF exceeding
// TARGETDURATION, a DATERANGE missing its required ID — are semantic rule
// violations that `media-doctor::check_playlist` flags, not syntax errors
// this structural parser rejects). `missing-extm3u.m3u8` is the one
// genuinely unparsable case: it is missing the required #EXTM3U header by
// design.
// ---------------------------------------------------------------------------

const HAND_MADE_CASES: &[Case] = &[
    Case {
        path: "valid.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "bad-extinf.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "malformed-daterange.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "reference-vod.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "missing-extm3u.m3u8",
        kind: Kind::Media,
        expect: Expect::Err("missing #EXTM3U header"),
    },
];

#[test]
fn hand_made_fixtures_parse_as_expected() {
    for case in HAND_MADE_CASES {
        assert_case(case);
    }
}

// ---------------------------------------------------------------------------
// Tier 2 — real Apple HLS example streams (fixtures/hls/real/) — playlists
// only, no segment media. Provenance (exact URLs, fetch date) in
// MANIFEST.md. Every one of these is a real encoder's output, so all are
// expected to parse cleanly; a failure here would be a genuine parser bug
// against production content, not a spec-fixture curiosity.
// ---------------------------------------------------------------------------

const REAL_CASES: &[Case] = &[
    // bipbop_16x9 — the classic MPEG-2 TS-segmented (byte-range into a
    // single main.ts) multivariant example, with an alternate (non-default)
    // audio rendition and a subtitle rendition group.
    Case {
        path: "real/bipbop-ts/master.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    Case {
        path: "real/bipbop-ts/gear1-video.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "real/bipbop-ts/alternate-audio.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    // bipbop_adv_example_hevc — the fMP4/CMAF-shaped (byte-range into a
    // single main.mp4, #EXT-X-MAP init segment) multivariant example, with
    // three alternate audio renditions (AAC/AC-3/EC-3), closed captions, and
    // subtitles.
    Case {
        path: "real/bipbop-fmp4-hevc/master.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    Case {
        path: "real/bipbop-fmp4-hevc/v5-video.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    Case {
        path: "real/bipbop-fmp4-hevc/a1-audio.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
];

#[test]
fn real_stream_fixtures_parse_cleanly() {
    for case in REAL_CASES {
        assert_case(case);
    }
}

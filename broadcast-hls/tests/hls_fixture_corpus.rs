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
//! `EXT-X-DATERANGE`/SCTE-35) — parses without error: unrecognized tags are
//! preserved verbatim into [`broadcast_hls::MediaPlaylist::extra_tags`] (or
//! silently skipped by [`broadcast_hls::MasterPlaylist`], which has no such
//! escape hatch) rather than rejected. So this run surfaces **no**
//! missing-tag parse failures for issue #872 to pick up — the two failures
//! above are a structural property of the spec excerpts, not an
//! implementation gap.
//!
//! # Round-trip invariant (issue #872)
//!
//! Every fixture that parses is additionally asserted to **round-trip**:
//! `parse -> to_m3u8() -> parse` must yield an equal document. Per
//! `docs/CRATE-ACCEPTANCE.md` §1 a text format is *not* required to be
//! byte-identical across that cycle, so the assertion is on the parsed
//! document, not the bytes; every way the rendered text may legitimately
//! differ from the input (unmodeled `EXT-X-MEDIA` dropped from a
//! Multivariant Playlist, canonical tag ordering, an always-emitted
//! `EXT-X-VERSION`, …) is enumerated in `broadcast-hls/README.md` under
//! "Round-trip fidelity".
//!
//! Note this makes the real-stream tier (`fixtures/hls/real/`) do double
//! duty: those six Apple playlists carry `EXT-X-MEDIA` groups, byte-ranges
//! and `EXT-X-MAP` init segments that no hand-made fixture exercises, so
//! they are where a carry-forward/dedup regression in `map`/`bitrate`
//! rendering would actually surface.
//!
//! # Tier 1b — hand-built fixtures (`fixtures/hls/handbuilt/`, issue #872)
//!
//! Nine of RFC 8216bis §4.4's 32 tags landed in #872, and the spec's own §9
//! examples do not cover all of them with a complete, parseable playlist
//! (`EXT-X-GAP`, `EXT-X-BITRATE`, `EXT-X-PLAYLIST-TYPE`, `EXT-X-START`,
//! `EXT-X-DEFINE` and `EXT-X-SESSION-KEY` appear in no §9 example at all;
//! `EXT-X-SESSION-DATA` appears only in §9.8, which is an explicit
//! *fragment* with no `#EXTM3U` line and is therefore excluded from the
//! spec tier — see MANIFEST.md). The `handbuilt/` fixtures fill exactly
//! that gap. They are **not** spec vectors and are deliberately pathed and
//! named so nobody mistakes them for one; their `EXT-X-VERSION` lines are
//! derived from the §8 table in `broadcast-hls/docs/version-compatibility.md`
//! rather than invented (see MANIFEST.md for each derivation).

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
            let parsed = MediaPlaylist::parse(&text)
                .unwrap_or_else(|e| panic!("{} should parse as a Media Playlist: {e}", case.path));
            // Round-trip (issue #872): parse -> serialize -> re-parse must
            // yield an equal document. Not byte-identity — see the module
            // doc's "Round-trip invariant" section.
            let rendered = parsed.to_m3u8();
            let reparsed = MediaPlaylist::parse(&rendered).unwrap_or_else(|e| {
                panic!(
                    "{}: re-parsing this crate's own rendered output must succeed: {e}\n\
                     rendered:\n{rendered}",
                    case.path
                )
            });
            assert_eq!(
                reparsed, parsed,
                "{}: parse -> serialize -> re-parse must yield an equal document\n\
                 rendered:\n{rendered}",
                case.path
            );
        }
        (Kind::Master, Expect::Ok) => {
            let parsed = MasterPlaylist::parse(&text).unwrap_or_else(|e| {
                panic!("{} should parse as a Multivariant Playlist: {e}", case.path)
            });
            let rendered = parsed.to_m3u8();
            let reparsed = MasterPlaylist::parse(&rendered).unwrap_or_else(|e| {
                panic!(
                    "{}: re-parsing this crate's own rendered output must succeed: {e}\n\
                     rendered:\n{rendered}",
                    case.path
                )
            });
            assert_eq!(
                reparsed, parsed,
                "{}: parse -> serialize -> re-parse must yield an equal document\n\
                 rendered:\n{rendered}",
                case.path
            );
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
// Tier 1b — hand-built fixtures for the #872 tags (fixtures/hls/handbuilt/).
//
// NOT spec vectors. Each is authored in-repo from the confirmed attribute
// grammar in `broadcast-hls/docs/playlist-tags.md`, covering tags no §9
// example exercises as a complete playlist. Provenance + per-file
// EXT-X-VERSION derivation in MANIFEST.md.
// ---------------------------------------------------------------------------

const HANDBUILT_CASES: &[Case] = &[
    // INDEPENDENT-SEGMENTS + START(PRECISE) + DEFINE(NAME/VALUE, QUERYPARAM)
    // + SESSION-KEY (two METHODs, one with an IV).
    Case {
        path: "handbuilt/multivariant-header-tags.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    // SESSION-DATA in all three shapes (URI form, and VALUE+LANGUAGE twice
    // sharing a DATA-ID) — derived from the §9.8 fragment, made into a real
    // playlist. See MANIFEST.md.
    Case {
        path: "handbuilt/session-data-multivariant.m3u8",
        kind: Kind::Master,
        expect: Expect::Ok,
    },
    // PLAYLIST-TYPE + GAP + BITRATE (carry-forward and change) + START
    // (positive offset, no PRECISE) + DEFINE(IMPORT) + variable substitution.
    Case {
        path: "handbuilt/live-vod-gap-bitrate.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    // Derived from §9.10: EXT-X-DATERANGE with SCTE35-OUT and SCTE35-IN,
    // made parseable by supplying the real Media Segments the spec elides
    // with its `...` markers. DATERANGE tags are unmodeled and preserved
    // via `extra_tags`. See MANIFEST.md for EXT-X-VERSION derivation.
    Case {
        path: "handbuilt/daterange-scte35-media.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
    // Derived from §9.11: EXT-X-PART partial segments, EXT-X-PRELOAD-HINT,
    // EXT-X-RENDITION-REPORT, and an EXT-X-DISCONTINUITY mid-roll ad break,
    // with the leading `...` replaced by real header tags
    // (EXT-X-VERSION, SERVER-CONTROL, PART-INF, MAP). See MANIFEST.md.
    Case {
        path: "handbuilt/low-latency-parts-preload-report.m3u8",
        kind: Kind::Media,
        expect: Expect::Ok,
    },
];

#[test]
fn handbuilt_fixtures_parse_as_expected() {
    for case in HANDBUILT_CASES {
        assert_case(case);
    }
}

/// Same directory-completeness guard as the spec tier: a `handbuilt/`
/// fixture added later without a matching case fails loudly rather than
/// sitting unparsed forever.
#[test]
fn handbuilt_dir_has_no_untested_m3u8_fixtures() {
    let dir = fixtures_hls_root().join("handbuilt");
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

    let mut expected: Vec<String> = HANDBUILT_CASES
        .iter()
        .map(|c| c.path.trim_start_matches("handbuilt/").to_string())
        .collect();
    expected.sort();

    assert_eq!(
        on_disk, expected,
        "fixtures/hls/handbuilt/*.m3u8 on disk must exactly match HANDBUILT_CASES in this test"
    );
}

/// The #872 tags must actually be reachable as *typed* data from these
/// fixtures — not merely swept into `extra_tags` (which would let a
/// parse-only regression pass the round-trip assertions above, since a
/// verbatim-preserved tag round-trips just as cleanly as a modeled one).
#[test]
fn handbuilt_fixtures_expose_the_872_tags_as_typed_data() {
    use broadcast_hls::{Define, PlaylistType, SessionDataContent};

    let mv = MasterPlaylist::parse(&read_fixture("handbuilt/multivariant-header-tags.m3u8"))
        .expect("must parse");
    assert!(mv.independent_segments, "EXT-X-INDEPENDENT-SEGMENTS");
    let start = mv.start.expect("EXT-X-START must be typed");
    assert_eq!(start.time_offset, -10.5);
    assert!(start.precise, "PRECISE=YES must be typed");
    assert_eq!(mv.defines.len(), 2, "both EXT-X-DEFINEs must be typed");
    assert!(matches!(&mv.defines[0], Define::Name { name, .. } if name == "base"));
    assert!(matches!(&mv.defines[1], Define::QueryParam { name } if name == "token"));
    assert_eq!(mv.session_keys.len(), 2, "both EXT-X-SESSION-KEYs");
    assert!(
        mv.session_keys[1].iv.is_some(),
        "the SAMPLE-AES-CTR key's IV must decode to 16 bytes, not stay a string"
    );

    let sd = MasterPlaylist::parse(&read_fixture("handbuilt/session-data-multivariant.m3u8"))
        .expect("must parse");
    assert_eq!(sd.session_data.len(), 3, "all three EXT-X-SESSION-DATA");
    assert!(matches!(
        &sd.session_data[0].content,
        SessionDataContent::Uri { .. }
    ));
    assert!(matches!(
        &sd.session_data[1].content,
        SessionDataContent::Value(v) if v == "This is an example"
    ));
    assert_eq!(sd.session_data[2].language.as_deref(), Some("es"));

    let media = MediaPlaylist::parse(&read_fixture("handbuilt/live-vod-gap-bitrate.m3u8"))
        .expect("must parse");
    assert_eq!(media.playlist_type, Some(PlaylistType::Vod));
    assert!(matches!(&media.defines[..], [Define::Import { name }] if name == "base"));
    assert_eq!(media.segments.len(), 3);
    assert!(!media.segments[0].gap, "seg0 carries no EXT-X-GAP");
    assert!(media.segments[1].gap, "seg1's EXT-X-GAP must be typed");
    assert!(!media.segments[2].gap, "GAP must not leak past its segment");
    // EXT-X-BITRATE carries forward onto seg1, then changes at seg2.
    assert_eq!(media.segments[0].bitrate, Some(2000));
    assert_eq!(media.segments[1].bitrate, Some(2000), "carry-forward");
    assert_eq!(media.segments[2].bitrate, Some(1800));

    // Derived from §9.10: two EXT-X-DATERANGE lines (unmodeled tags that
    // go into `extra_tags`) carrying SCTE35-OUT and SCTE35-IN.
    let dtrng = MediaPlaylist::parse(&read_fixture("handbuilt/daterange-scte35-media.m3u8"))
        .expect("must parse");
    assert_eq!(dtrng.segments.len(), 3, "three segments");
    assert!(
        dtrng.extra_tags.len() >= 2,
        "at least two unmodeled EXT-X-DATERANGE lines must land in extra_tags"
    );
    assert!(
        dtrng.extra_tags.iter().any(|t| t.contains("SCTE35-OUT")),
        "SCTE35-OUT DATERANGE must be in extra_tags"
    );
    assert!(
        dtrng.extra_tags.iter().any(|t| t.contains("SCTE35-IN")),
        "SCTE35-IN DATERANGE must be in extra_tags"
    );

    // Derived from §9.11: EXT-X-PART, EXT-X-PRELOAD-HINT,
    // EXT-X-RENDITION-REPORT, EXT-X-DISCONTINUITY, EXT-X-MAP.
    let ll = MediaPlaylist::parse(&read_fixture(
        "handbuilt/low-latency-parts-preload-report.m3u8",
    ))
    .expect("must parse");
    assert_eq!(ll.segments.len(), 6, "six closed segments");
    assert!(
        ll.segments[5].discontinuous,
        "mid-roll DISCONTINUITY at seg-273"
    );
    let seg271 = &ll.segments[3];
    assert_eq!(seg271.parts.len(), 2, "seg271 has two EXT-X-PARTs");
    assert_eq!(&seg271.parts[0].uri, "filePart271.0.mp4");
    assert!(
        seg271.parts[0].independent,
        "first part of seg271 is INDEPENDENT"
    );
    assert_eq!(&seg271.parts[1].uri, "filePart271.1.mp4");
    let _seg274 = &ll.segments[4];
    let ll_config = ll.low_latency.as_ref().expect("must have LowLatencyConfig");
    assert!(
        ll_config.preload_hint_part.is_some(),
        "PRELOAD-HINT must be typed"
    );
    assert!(
        !ll.rendition_reports.is_empty(),
        "RENDITION-REPORT must be typed"
    );
    assert_eq!(
        ll.rendition_reports[0].uri, "/1M/LL-HLS.m3u8",
        "RENDITION-REPORT URI must match"
    );
    assert_eq!(ll.rendition_reports[0].last_msn, 274);
    assert_eq!(ll.rendition_reports[0].last_part, Some(1));
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

// ---------------------------------------------------------------------------
// Gap A (issue #890) — an unmodeled tag on a MasterPlaylist is preserved
// (NOT dropped), so parse → serialize → re-parse yields an equal document.
// ---------------------------------------------------------------------------

#[test]
fn unmodeled_ext_x_media_survives_master_parse_round_trip() {
    let input = "\
#EXTM3U
#EXT-X-VERSION:4
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",LANGUAGE=\"eng\",NAME=\"English\",URI=\"audio.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=1280000,CODECS=\"avc1.64001e,mp4a.40.2\",AUDIO=\"audio\"
low.m3u8
";
    let parsed = MasterPlaylist::parse(input).expect("master with EXT-X-MEDIA must parse");

    // EXT-X-MEDIA is an unmodeled tag on MasterPlaylist — it must land
    // in extra_tags, not be silently dropped.
    assert_eq!(
        parsed.extra_tags.len(),
        1,
        "EXT-X-MEDIA must be preserved in extra_tags"
    );
    assert!(
        parsed.extra_tags[0].starts_with("#EXT-X-MEDIA:"),
        "extra_tags[0] must be the EXT-X-MEDIA line: {}",
        parsed.extra_tags[0]
    );

    // Round-trip: parse → serialize → re-parse must yield an equal document.
    let rendered = parsed.to_m3u8();
    let reparsed = MasterPlaylist::parse(&rendered).unwrap_or_else(|e| {
        panic!(
            "re-parsing this crate's own rendered output must succeed: {e}\n\
             rendered:\n{rendered}"
        )
    });
    assert_eq!(
        reparsed, parsed,
        "parse → serialize → re-parse must yield an equal document\n\
         rendered:\n{rendered}"
    );

    // EXT-X-MEDIA must appear in the rendered output — it must not vanish.
    assert!(
        rendered.contains("#EXT-X-MEDIA:TYPE=AUDIO"),
        "rendered output must contain the EXT-X-MEDIA line:\n{rendered}"
    );
}

//! Cross-check the `EXT-X-VERSION` derivation against the RFC's own §9
//! example playlists (issue #871).
//!
//! Every other version test in this crate compares the derivation against
//! *our* reading of the §8 transcription in `docs/version-compatibility.md`.
//! This one is the only genuinely **independent** check: the playlists in
//! `fixtures/hls/spec/` were written by the spec authors themselves
//! (draft-pantos-hls-rfc8216bis-22 §9, extracted verbatim by issue #877), so
//! where one of them declares an `EXT-X-VERSION`, that number is the spec
//! authors' own answer to the question our derivation is trying to answer.
//! A disagreement means one of the two is wrong, and it is worth knowing
//! which.
//!
//! # Why this compares `computed_version()`, not the rendered output
//!
//! Comparing `to_m3u8()`'s `#EXT-X-VERSION` line against the fixture's would
//! be **circular and vacuous**: `MediaPlaylist::parse` puts the wire's
//! declared version into the `version` field, `version` acts as a floor over
//! the computed minimum, so a re-render of a parsed `EXT-X-VERSION:3`
//! playlist emits `3` whether or not the derivation works at all. Only
//! [`MediaPlaylist::computed_version`] / [`MasterPlaylist::computed_version`]
//! ignore the parsed field and look purely at the playlist's *content* —
//! which is exactly the quantity §8 specifies and the only one worth
//! comparing to the spec authors' declaration.
//!
//! # The absent-tag cases are assertions too, not skips
//!
//! §8's opening rule is that a Playlist only MUST carry `EXT-X-VERSION` if
//! it contains something not compatible with version 1. So a spec example
//! with **no** version tag is the authors asserting "nothing in here
//! triggers a row" — and our derivation returning `None` for it is a real
//! agreement, not an absence of evidence. If the derivation computed a
//! version for a playlist the authors left untagged, that would be a
//! finding (an over-trigger in our table), which is why those are asserted
//! rather than skipped.

use std::fs;

use broadcast_hls::{MasterPlaylist, MediaPlaylist};

fn read_spec_fixture(name: &str) -> String {
    let path = format!(
        "{}/../fixtures/hls/spec/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read spec fixture {path}: {e}"))
}

/// The `EXT-X-VERSION` the spec authors wrote into the fixture, read
/// straight out of the raw text (not via our own parser, so a parser bug
/// cannot quietly make both sides of the comparison agree).
fn declared_version(text: &str) -> Option<u8> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("#EXT-X-VERSION:"))
        .map(|v| {
            v.trim()
                .parse::<u8>()
                .expect("fixture EXT-X-VERSION must be a valid u8")
        })
}

/// Media Playlist examples from RFC 8216bis §9.
///
/// Deliberately excluded (do **not** "fix" these — the RFC itself abridges
/// them with a literal `...` continuation line standing in for elided
/// content, which is not valid playlist syntax and so cannot parse; the
/// fixtures are verbatim extracts and correctly preserve that):
/// - `9.10-daterange-scte35.m3u8` — `...` on line 2, plus a prose line
///   "... Media Segment declarations for 60s worth of media".
/// - `9.11-low-latency-playlist.m3u8` — `...` on line 3, standing in for the
///   elided `EXT-X-VERSION`/`EXT-X-SERVER-CONTROL`/`EXT-X-PART-INF` header
///   block.
const SPEC_MEDIA_PLAYLISTS: &[&str] = &[
    "9.1-simple-media-playlist.m3u8",
    "9.2-live-media-playlist-https.m3u8",
    "9.3-encrypted-media-segments.m3u8",
];

/// Multivariant Playlist examples from RFC 8216bis §9.
const SPEC_MULTIVARIANT_PLAYLISTS: &[&str] = &[
    "9.4-multivariant-playlist.m3u8",
    "9.5-multivariant-with-iframes.m3u8",
    "9.6-multivariant-alternative-audio.m3u8",
    "9.7-multivariant-alternative-video.m3u8",
    "9.12-content-steering.m3u8",
];

#[test]
fn spec_media_playlist_computed_version_matches_the_authors_declaration() {
    for name in SPEC_MEDIA_PLAYLISTS {
        let text = read_spec_fixture(name);
        let pl = MediaPlaylist::parse(&text)
            .unwrap_or_else(|e| panic!("spec fixture {name} must parse: {e}"));
        assert_eq!(
            pl.computed_version(),
            declared_version(&text),
            "{name}: our RFC 8216bis §8 derivation disagrees with the version \
             the spec authors declared in their own example. One of the two \
             is wrong.\n{text}"
        );
    }
}

#[test]
fn spec_multivariant_playlist_computed_version_matches_the_authors_declaration() {
    for name in SPEC_MULTIVARIANT_PLAYLISTS {
        let text = read_spec_fixture(name);
        let pl = MasterPlaylist::parse(&text)
            .unwrap_or_else(|e| panic!("spec fixture {name} must parse: {e}"));
        assert_eq!(
            pl.computed_version(),
            declared_version(&text),
            "{name}: our RFC 8216bis §8 derivation disagrees with the version \
             the spec authors declared in their own example. One of the two \
             is wrong.\n{text}"
        );
    }
}

/// Guard the guard: if the three Media Playlist fixtures stopped declaring a
/// version (re-extracted wrong, replaced by an untagged example), the test
/// above would silently degrade into `None == None` and prove nothing. Pin
/// the fact that they *do* carry a declaration, and that it is the value
/// §8's floating-point-`EXTINF` rule (row 3) accounts for.
#[test]
fn spec_media_fixtures_actually_declare_a_version_to_compare_against() {
    for name in SPEC_MEDIA_PLAYLISTS {
        let text = read_spec_fixture(name);
        assert_eq!(
            declared_version(&text),
            Some(3),
            "{name} must carry the authors' own #EXT-X-VERSION:3 for the \
             comparison above to mean anything"
        );
    }
}

/// The mirror of the test above for the Multivariant fixtures: pin that they
/// carry **no** declaration, so the `None == None` agreement asserted above
/// is the spec authors genuinely leaving them untagged (§8's opening rule),
/// not a fixture that silently lost its version line.
#[test]
fn spec_multivariant_fixtures_are_genuinely_untagged() {
    for name in SPEC_MULTIVARIANT_PLAYLISTS {
        let text = read_spec_fixture(name);
        assert_eq!(
            declared_version(&text),
            None,
            "{name} is expected to carry no #EXT-X-VERSION at all (§8: a \
             Playlist need not declare one if it is version-1 compatible)"
        );
    }
}

/// The two abridged fixtures are excluded above for a stated reason; pin
/// that reason so nobody quietly adds them to the list and then "fixes" the
/// RFC's own text to make them parse.
#[test]
fn abridged_spec_fixtures_do_not_parse_because_the_rfc_elides_them() {
    for name in [
        "9.10-daterange-scte35.m3u8",
        "9.11-low-latency-playlist.m3u8",
    ] {
        let text = read_spec_fixture(name);
        assert!(
            text.lines().any(|l| l.trim() == "..."),
            "{name} is excluded on the grounds that the RFC abridges it with \
             a literal `...` line — that line must actually be there"
        );
        assert!(
            MediaPlaylist::parse(&text).is_err(),
            "{name} is excluded on the grounds that it cannot parse; if it \
             now parses, re-evaluate whether it belongs in the compared set"
        );
    }
}

// ---------------------------------------------------------------------------
// Hand-built fixtures (issue #872) — `fixtures/hls/handbuilt/`.
//
// These are NOT spec vectors, so unlike the §9 fixtures above they carry no
// independent authority: their declared version is *our* claim, derived by
// hand from the §8 table and written down in `fixtures/hls/MANIFEST.md`'s
// "EXT-X-VERSION derivation" section. Asserting `computed_version()` against
// them therefore proves something narrower but still worth having — that the
// hand-derivation in the manifest and the code's derivation agree. It makes
// the manifest's prose table executable, so a claim there cannot rot.
//
// It is also the integration check between #872 and #880: the tags whose
// typed representations #872 added (notably `EXT-X-DEFINE`) are the substrate
// several §8 rows read, so a row left string-matching `extra_tags` after
// those tags became typed would silently stop firing and show up right here.
// ---------------------------------------------------------------------------

fn read_handbuilt_fixture(name: &str) -> String {
    let path = format!(
        "{}/../fixtures/hls/handbuilt/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read handbuilt fixture {path}: {e}"))
}

/// `(fixture, expected computed_version, the MANIFEST's stated reason)` —
/// values transcribed from `fixtures/hls/MANIFEST.md`'s Tier 1b derivation
/// table. Keep the two in step; that is the whole point of this test.
const HANDBUILT_MULTIVARIANT_VERSIONS: &[(&str, Option<u8>, &str)] = &[
    (
        "multivariant-header-tags.m3u8",
        Some(11),
        "§8 row 11 — EXT-X-DEFINE with a QUERYPARAM attribute",
    ),
    (
        "session-data-multivariant.m3u8",
        None,
        "no §8 row triggered — EXT-X-SESSION-DATA carries no version requirement",
    ),
];

const HANDBUILT_MEDIA_VERSIONS: &[(&str, Option<u8>, &str)] = &[
    (
        "live-vod-gap-bitrate.m3u8",
        Some(8),
        "§8 row 8 — variable substitution ({$base} in the segment URIs)",
    ),
    (
        "daterange-scte35-media.m3u8",
        Some(3),
        "§8 row 3 — floating-point EXTINF durations (all segments are 10.000 or 5.993)",
    ),
    (
        "low-latency-parts-preload-report.m3u8",
        Some(6),
        "§8 row 6 — EXT-X-MAP without EXT-X-I-FRAMES-ONLY",
    ),
];

#[test]
fn handbuilt_multivariant_computed_version_matches_the_manifest_derivation() {
    for (name, expected, why) in HANDBUILT_MULTIVARIANT_VERSIONS {
        let text = read_handbuilt_fixture(name);
        let pl = MasterPlaylist::parse(&text)
            .unwrap_or_else(|e| panic!("handbuilt fixture {name} must parse: {e}"));
        assert_eq!(
            pl.computed_version(),
            *expected,
            "{name}: computed version disagrees with the derivation recorded \
             in fixtures/hls/MANIFEST.md ({why}). Either the manifest's \
             reasoning is wrong or the §8 implementation is.\n{text}"
        );
    }
}

#[test]
fn handbuilt_media_computed_version_matches_the_manifest_derivation() {
    for (name, expected, why) in HANDBUILT_MEDIA_VERSIONS {
        let text = read_handbuilt_fixture(name);
        let pl = MediaPlaylist::parse(&text)
            .unwrap_or_else(|e| panic!("handbuilt fixture {name} must parse: {e}"));
        assert_eq!(
            pl.computed_version(),
            *expected,
            "{name}: computed version disagrees with the derivation recorded \
             in fixtures/hls/MANIFEST.md ({why}). Either the manifest's \
             reasoning is wrong or the §8 implementation is.\n{text}"
        );
    }
}

/// Guard the guard, as above: the fixtures' own declared `EXT-X-VERSION`
/// must equal what we derive, otherwise the fixture is itself mis-declared
/// and would mislead any human reading it. (The untagged one must stay
/// untagged — that is its whole point, and inventing a version line for it
/// is exactly the mistake this fixture exists to *not* make.)
#[test]
fn handbuilt_fixtures_declare_the_version_they_derive() {
    for (name, expected, why) in HANDBUILT_MULTIVARIANT_VERSIONS
        .iter()
        .chain(HANDBUILT_MEDIA_VERSIONS.iter())
    {
        let text = read_handbuilt_fixture(name);
        assert_eq!(
            declared_version(&text),
            *expected,
            "{name}: the #EXT-X-VERSION written into the fixture must match \
             the version §8 requires for its content ({why}) — over-declaring \
             locks out clients that could have played it, under-declaring \
             misrepresents the content"
        );
    }
}

/// Row 11 specifically, asserted through the *typed* field rather than the
/// fixture text, so the #872/#880 integration is pinned independently of any
/// parse path: a `Define::QueryParam` built programmatically (never having
/// been text at all) must still trigger row 11. Before #872 wired this row
/// to `defines`, only a raw `extra_tags` line could reach it — so this test
/// fails against a row-11 check that still string-matches.
#[test]
fn query_param_define_triggers_row_11_when_built_programmatically() {
    use broadcast_hls::Define;

    let pl = MasterPlaylist {
        variants: vec![broadcast_hls::Variant {
            bandwidth: 300_000,
            codecs: "avc1.64001e".into(),
            resolution: None,
            uri: "v300/index.m3u8".into(),
        }],
        defines: vec![Define::QueryParam {
            name: "token".into(),
        }],
        ..Default::default()
    };
    assert_eq!(
        pl.computed_version(),
        Some(11),
        "a programmatically-built EXT-X-DEFINE:QUERYPARAM must trigger §8 \
         row 11 — it never passes through extra_tags, so a string-matching \
         implementation of this row would silently miss it"
    );

    // The NAME/VALUE form must NOT trigger row 11 (it is row 8 territory,
    // and only when a substitution is actually used) — otherwise the check
    // above would pass for the wrong reason.
    let plain = MasterPlaylist {
        defines: vec![Define::Name {
            name: "base".into(),
            value: "https://cdn.example.com".into(),
        }],
        ..Default::default()
    };
    assert_eq!(
        plain.computed_version(),
        None,
        "a NAME/VALUE EXT-X-DEFINE with no substitution used triggers no row"
    );
}

/// Known gap, pinned so it stays visible: §8 row 12 (`REQ-` attribute) is
/// matched only on tags that reach `extra_tags`. On a tag this crate models
/// with typed fields, unknown attributes are discarded at parse time, so a
/// `REQ-` attribute there is invisible to the derivation. Closing this needs
/// unknown-attribute retention on every modeled tag — an API change beyond
/// issue #872's scope. If this test starts failing because the gap was
/// closed, delete it and celebrate.
#[test]
fn req_attribute_on_a_modeled_tag_is_a_known_gap() {
    // On an UNMODELED tag, row 12 fires correctly.
    let unmodeled = MasterPlaylist::parse(
        "#EXTM3U\n#EXT-X-FUTURE-FEATURE:REQ-CODEC=\"av01\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH=300000\nv.m3u8\n",
    )
    .expect("must parse");
    assert_eq!(
        unmodeled.computed_version(),
        Some(12),
        "REQ- on an unmodeled tag reaches extra_tags and must trigger row 12"
    );

    // On a MODELED tag it does not, because the attribute is dropped.
    let modeled = MasterPlaylist::parse(
        "#EXTM3U\n#EXT-X-CONTENT-STEERING:SERVER-URI=\"/s\",REQ-FOO=\"bar\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH=300000\nv.m3u8\n",
    )
    .expect("must parse");
    assert_eq!(
        modeled.computed_version(),
        None,
        "documented gap: REQ- on a modeled tag is dropped at parse time and \
         cannot reach the row-12 check. If this now returns Some(12), the \
         gap was closed — update the docs in `scan_tag_lines_for_version`."
    );
}

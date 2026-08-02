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

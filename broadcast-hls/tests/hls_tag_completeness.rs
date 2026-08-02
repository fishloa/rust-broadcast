//! Completeness drift-guard (issue #872): enumerates all 32 tags RFC
//! 8216bis §4.4 defines (per `docs/playlist-tags.md`'s own "Tag count"
//! table) and fails if any tag name has no reference anywhere in `src/`. A
//! future spec revision adding a 33rd tag — or a regression that deletes
//! handling for one of these 32 — surfaces here as a red test rather than
//! silent omission.
//!
//! This is a *presence* check (the tag name appears as source text this
//! crate's parser/renderer dispatches on or documents), not a behavioral
//! one — the per-tag round-trip tests in `src/lib.rs` and
//! `tests/hls_fixture_corpus.rs` cover behavior. A tag deliberately NOT
//! modeled as structured data (e.g. `#EXT-X-MEDIA`, `#EXT-X-DATERANGE`)
//! still passes this check as long as its documented handling
//! (ignore-gracefully / preserve-verbatim) is present in source, per this
//! crate's module-doc "documented gaps" list.

use std::fs;
use std::path::Path;

/// All 32 tags RFC 8216bis §4.4 defines, per `docs/playlist-tags.md`'s "Tag
/// count" table (§4.4.1 through §4.4.6.6).
const ALL_32_TAGS: &[&str] = &[
    // §4.4.1 Basic Tags (2)
    "EXTM3U",
    "EXT-X-VERSION",
    // §4.4.2 Media or Multivariant Playlist Tags (3) — issue #872
    "EXT-X-INDEPENDENT-SEGMENTS",
    "EXT-X-START",
    "EXT-X-DEFINE",
    // §4.4.3 Media Playlist Tags (8)
    "EXT-X-TARGETDURATION",
    "EXT-X-MEDIA-SEQUENCE",
    "EXT-X-DISCONTINUITY-SEQUENCE",
    "EXT-X-ENDLIST",
    "EXT-X-PLAYLIST-TYPE", // issue #872
    "EXT-X-I-FRAMES-ONLY",
    "EXT-X-PART-INF",
    "EXT-X-SERVER-CONTROL",
    // §4.4.4 Media Segment Tags (9)
    "EXTINF",
    "EXT-X-BYTERANGE",
    "EXT-X-DISCONTINUITY",
    "EXT-X-KEY",
    "EXT-X-MAP",
    "EXT-X-PROGRAM-DATE-TIME",
    "EXT-X-GAP",     // issue #872
    "EXT-X-BITRATE", // issue #872
    "EXT-X-PART",
    // §4.4.5 Media Metadata Tags (4)
    "EXT-X-DATERANGE",
    "EXT-X-SKIP",
    "EXT-X-PRELOAD-HINT",
    "EXT-X-RENDITION-REPORT",
    // §4.4.6 Multivariant Playlist Tags (6)
    "EXT-X-MEDIA",
    "EXT-X-STREAM-INF",
    "EXT-X-I-FRAME-STREAM-INF",
    "EXT-X-SESSION-DATA",     // issue #872
    "EXT-X-SESSION-KEY",      // issue #872
    "EXT-X-CONTENT-STEERING", // issue #872
];

fn read_src(dir: &Path, out: &mut String) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            read_src(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push_str(&fs::read_to_string(&path).expect("read .rs"));
            out.push('\n');
        }
    }
}

/// `true` if `tag` occurs in `src` as itself — not as a strict prefix of a
/// longer, distinct tag name (e.g. `EXT-X-MEDIA` must not be satisfied
/// merely by `EXT-X-MEDIA-SEQUENCE` appearing in source).
fn tag_referenced(src: &str, tag: &str) -> bool {
    let mut start = 0;
    while let Some(idx) = src[start..].find(tag) {
        let abs = start + idx;
        let end = abs + tag.len();
        let next = src[end..].chars().next();
        let is_longer_tag_name = matches!(next, Some(c) if c.is_ascii_alphanumeric() || c == '-');
        if !is_longer_tag_name {
            return true;
        }
        start = end;
    }
    false
}

#[test]
fn all_32_rfc8216bis_section_4_4_tags_are_referenced_in_source() {
    assert_eq!(ALL_32_TAGS.len(), 32, "the tag list itself must total 32");

    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut src = String::new();
    read_src(&src_dir, &mut src);

    let missing: Vec<&str> = ALL_32_TAGS
        .iter()
        .filter(|tag| !tag_referenced(&src, tag))
        .copied()
        .collect();

    assert!(
        missing.is_empty(),
        "tag(s) from RFC 8216bis §4.4 with no reference anywhere in src/: {missing:?}\n\
         Either implement parse/render support (see docs/playlist-tags.md for the \
         attribute grammar) or, if intentionally unmodeled, add a `#EXT...` string \
         literal reference (e.g. in a doc comment or an ignore-arm) so the decision \
         is visible in source."
    );
}

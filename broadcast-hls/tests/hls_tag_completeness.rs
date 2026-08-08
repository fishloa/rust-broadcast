//! Completeness drift-guard (issue #872, made behavioral in #935).
//!
//! Two layers:
//!
//! 1. [`all_32_rfc8216bis_section_4_4_tags_are_referenced_in_source`] — a
//!    coarse *presence* check (the tag name appears as source text
//!    somewhere in `src/`). Cheap, but satisfiable by a doc-comment mention
//!    alone — it proves a tag was thought about, not that it is handled.
//! 2. [`typed_tags_populate_their_struct_field_on_parse`] and
//!    [`opaque_tags_are_preserved_verbatim_not_dropped`] — the *behavioral*
//!    guard issue #935 added. Each parses a real playlist containing every
//!    tag and asserts the concrete outcome: for a tag with a typed parse
//!    handler, the corresponding struct field is actually populated (not
//!    just "parse didn't error"); for a tag this crate deliberately does
//!    not model with a struct field (`#EXT-X-KEY`, `#EXT-X-PROGRAM-DATE-TIME`,
//!    `#EXT-X-DATERANGE`, `#EXT-X-MEDIA` — see the module doc's "Known,
//!    documented gaps" list in `src/lib.rs`), it asserts the tag survives
//!    verbatim in `extra_tags` rather than being silently dropped.
//!
//! Before #935, layer 2 did not exist: the only guard was layer 1, which
//! cannot detect a missing typed handler — see the git history of this file
//! for the presence-only version. `README.md`'s "all 32 tags parse *and*
//! serialize" claim (also corrected by #935) had drifted from the code
//! specifically because nothing here could catch that drift.

use std::fs;
use std::path::Path;

use broadcast_hls::{
    EncryptionMethod, MasterPlaylist, MediaPlaylist, PlaylistType, SessionDataContent,
};

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

/// The 4 of the 32 tags with **no typed struct field**: recognized (they
/// don't error and aren't dropped) but carried opaquely in `extra_tags`.
/// See the module doc's "Known, documented gaps" list in `src/lib.rs` and
/// `README.md`'s "Round-trip fidelity" section.
const OPAQUE_TAGS: &[&str] = &[
    "EXT-X-KEY",
    "EXT-X-PROGRAM-DATE-TIME",
    "EXT-X-DATERANGE",
    "EXT-X-MEDIA",
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

// ---------------------------------------------------------------------------
// Behavioral layer (issue #935).
// ---------------------------------------------------------------------------

/// A Media Playlist exercising every §4.4.1/.2/.3/.4/.5 tag that has a typed
/// struct field (i.e. every tag in [`ALL_32_TAGS`] minus [`OPAQUE_TAGS`],
/// restricted to tags valid in a Media Playlist).
const MEDIA_FIXTURE_TYPED_TAGS: &str = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-INDEPENDENT-SEGMENTS\n\
#EXT-X-DEFINE:NAME=\"HOST\",VALUE=\"example.com\"\n\
#EXT-X-START:TIME-OFFSET=-5.0\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:10\n\
#EXT-X-DISCONTINUITY-SEQUENCE:2\n\
#EXT-X-PLAYLIST-TYPE:VOD\n\
#EXT-X-I-FRAMES-ONLY\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-SKIP:SKIPPED-SEGMENTS=3\n\
#EXT-X-BITRATE:560\n\
#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXT-X-BYTERANGE:1000@0\n\
#EXT-X-DISCONTINUITY\n\
#EXT-X-GAP\n\
#EXT-X-PART:DURATION=0.5,URI=\"part0.m4s\"\n\
#EXTINF:6.000,\n\
seg0.m4s\n\
#EXT-X-RENDITION-REPORT:URI=\"../audio/rendition.m3u8\",LAST-MSN=100\n\
#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part1.m4s\"\n\
#EXT-X-ENDLIST\n";

/// A Multivariant (Master) Playlist exercising every §4.4.2/.6 tag with a
/// typed struct field that is Master-Playlist-only (`EXT-X-STREAM-INF`,
/// `EXT-X-I-FRAME-STREAM-INF`, `EXT-X-SESSION-DATA`, `EXT-X-SESSION-KEY`,
/// `EXT-X-CONTENT-STEERING`).
const MASTER_FIXTURE_TYPED_TAGS: &str = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-SESSION-DATA:DATA-ID=\"com.example.data\",VALUE=\"hello\"\n\
#EXT-X-SESSION-KEY:METHOD=AES-128,URI=\"key.bin\"\n\
#EXT-X-CONTENT-STEERING:SERVER-URI=\"steering.json\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=300000,CODECS=\"avc1.64001e,mp4a.40.2\"\n\
v300/index.m3u8\n\
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=100000,URI=\"iframe.m3u8\"\n";

/// Parses [`MEDIA_FIXTURE_TYPED_TAGS`] and [`MASTER_FIXTURE_TYPED_TAGS`] and
/// asserts, per tag, that the field the tag is supposed to populate is
/// actually populated — not merely that parsing didn't error. This is the
/// guard `README.md`'s "all 32 tags parse *and* serialize" claim needs and
/// the presence-only test above cannot provide: deleting a typed `parse`
/// arm still leaves the tag's *name* in doc comments elsewhere in `src/`,
/// so the presence check keeps passing while this one goes red (verified
/// by hand for issue #935 — see the PR description for the transcript).
#[test]
fn typed_tags_populate_their_struct_field_on_parse() {
    let mp = MediaPlaylist::parse(MEDIA_FIXTURE_TYPED_TAGS)
        .expect("the typed-tag media fixture must parse cleanly");

    // §4.4.2 (shared).
    assert!(mp.independent_segments, "EXT-X-INDEPENDENT-SEGMENTS");
    assert!(mp.start.is_some(), "EXT-X-START");
    assert!(!mp.defines.is_empty(), "EXT-X-DEFINE");

    // §4.4.3 Media Playlist Tags.
    assert_eq!(mp.target_duration, 6, "EXT-X-TARGETDURATION");
    assert_eq!(mp.media_sequence, 10, "EXT-X-MEDIA-SEQUENCE");
    assert_eq!(mp.discontinuity_sequence, 2, "EXT-X-DISCONTINUITY-SEQUENCE");
    assert_eq!(
        mp.playlist_type,
        Some(PlaylistType::Vod),
        "EXT-X-PLAYLIST-TYPE"
    );
    assert!(mp.iframes_only, "EXT-X-I-FRAMES-ONLY");
    let ll = mp
        .low_latency
        .as_ref()
        .expect("EXT-X-PART-INF/SERVER-CONTROL must set low_latency");
    assert_eq!(ll.part_target, 0.5, "EXT-X-PART-INF");
    assert!(ll.can_block_reload, "EXT-X-SERVER-CONTROL");

    // §4.4.4 Media Segment Tags.
    assert_eq!(mp.segments.len(), 1, "exactly one segment expected");
    let seg = &mp.segments[0];
    assert_eq!(seg.duration, 6.0, "EXTINF");
    assert_eq!(
        seg.byte_range.map(|br| (br.length, br.offset)),
        Some((1000, Some(0))),
        "EXT-X-BYTERANGE"
    );
    assert!(seg.discontinuous, "EXT-X-DISCONTINUITY");
    assert_eq!(
        seg.map.as_ref().map(|m| m.uri.as_str()),
        Some("init.mp4"),
        "EXT-X-MAP"
    );
    assert!(seg.gap, "EXT-X-GAP");
    assert_eq!(seg.bitrate, Some(560), "EXT-X-BITRATE");
    assert_eq!(seg.parts.len(), 1, "EXT-X-PART");

    // §4.4.5 Media Metadata Tags.
    let skip = mp.skip.as_ref().expect("EXT-X-SKIP");
    assert_eq!(skip.skipped_segments, 3, "EXT-X-SKIP");
    assert_eq!(
        ll.preload_hint_part.as_deref(),
        Some("part1.m4s"),
        "EXT-X-PRELOAD-HINT"
    );
    assert_eq!(mp.rendition_reports.len(), 1, "EXT-X-RENDITION-REPORT");

    assert!(mp.endlist, "EXT-X-ENDLIST");
    // EXTM3U/EXT-X-VERSION: structural — must round-trip into the rendered
    // header exactly (version is a floor, so this also incidentally
    // exercises `effective_version`).
    let rendered = mp.to_m3u8();
    assert!(rendered.starts_with("#EXTM3U\n#EXT-X-VERSION:9\n"));

    let mst = MasterPlaylist::parse(MASTER_FIXTURE_TYPED_TAGS)
        .expect("the typed-tag master fixture must parse cleanly");

    assert_eq!(mst.session_data.len(), 1, "EXT-X-SESSION-DATA");
    assert_eq!(
        mst.session_data[0].content,
        SessionDataContent::Value("hello".to_string()),
        "EXT-X-SESSION-DATA content"
    );
    assert_eq!(mst.session_keys.len(), 1, "EXT-X-SESSION-KEY");
    assert_eq!(
        mst.session_keys[0].method,
        EncryptionMethod::Aes128,
        "EXT-X-SESSION-KEY method"
    );
    let cs = mst
        .content_steering
        .as_ref()
        .expect("EXT-X-CONTENT-STEERING");
    assert_eq!(cs.server_uri, "steering.json", "EXT-X-CONTENT-STEERING");
    assert_eq!(mst.variants.len(), 1, "EXT-X-STREAM-INF");
    assert_eq!(mst.variants[0].bandwidth, 300_000, "EXT-X-STREAM-INF");
    assert_eq!(mst.iframe_variants.len(), 1, "EXT-X-I-FRAME-STREAM-INF");
    assert_eq!(
        mst.iframe_variants[0].bandwidth, 100_000,
        "EXT-X-I-FRAME-STREAM-INF"
    );
}

/// A Media Playlist carrying the 3 opaque tags valid there
/// (`EXT-X-KEY`/`EXT-X-PROGRAM-DATE-TIME`/`EXT-X-DATERANGE`), and a Master
/// Playlist carrying the 1 opaque tag valid there (`EXT-X-MEDIA`). Asserts
/// each survives **verbatim** in `extra_tags` — the documented behavior for
/// a tag with no typed field (module doc "Known, documented gaps",
/// `README.md` "Round-trip fidelity") — rather than being silently dropped.
#[test]
fn opaque_tags_are_preserved_verbatim_not_dropped() {
    const KEY_LINE: &str = "#EXT-X-KEY:METHOD=AES-128,URI=\"https://example.com/key\"";
    const PDT_LINE: &str = "#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00.000Z";
    const DATERANGE_LINE: &str =
        "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0";

    let media_text = format!(
        "#EXTM3U\n\
#EXT-X-TARGETDURATION:6\n\
{KEY_LINE}\n\
{PDT_LINE}\n\
{DATERANGE_LINE}\n\
#EXTINF:6.000,\n\
seg0.m4s\n\
#EXT-X-ENDLIST\n"
    );
    let mp = MediaPlaylist::parse(&media_text).expect("opaque-tag media fixture must parse");
    for (name, line) in [
        ("EXT-X-KEY", KEY_LINE),
        ("EXT-X-PROGRAM-DATE-TIME", PDT_LINE),
        ("EXT-X-DATERANGE", DATERANGE_LINE),
    ] {
        assert!(
            mp.extra_tags.iter().any(|t| t == line),
            "{name} must be preserved verbatim in extra_tags, got {:?}",
            mp.extra_tags
        );
    }

    const MEDIA_TAG_LINE: &str =
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"English\",URI=\"eng.m3u8\"";
    let master_text = format!(
        "#EXTM3U\n\
{MEDIA_TAG_LINE}\n\
#EXT-X-STREAM-INF:BANDWIDTH=300000,CODECS=\"avc1.64001e,mp4a.40.2\"\n\
v300/index.m3u8\n"
    );
    let mst = MasterPlaylist::parse(&master_text).expect("opaque-tag master fixture must parse");
    assert!(
        mst.extra_tags.iter().any(|t| t == MEDIA_TAG_LINE),
        "EXT-X-MEDIA must be preserved verbatim in extra_tags, got {:?}",
        mst.extra_tags
    );

    // Cross-check against the tag-name lists above so this test and
    // `ALL_32_TAGS`/`OPAQUE_TAGS` cannot silently drift apart.
    for tag in OPAQUE_TAGS {
        assert!(
            ALL_32_TAGS.contains(tag),
            "{tag} must be one of the 32 §4.4 tags"
        );
    }
}

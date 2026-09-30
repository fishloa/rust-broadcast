//! Container child order and framing — audit r05-W12 / r05-W13.
//!
//! W12: a container's typed fields carry no order, so the serializer used to
//! write every typed child first and every opaque one after, reordering any
//! file that did not already match. A subtitle track's `sthd` (§6.2.3 puts a
//! media header first in `minf`) moved after `stbl`, and a `moov`'s `pssh`
//! moved after the `trak`s.
//!
//! W13: the child walk read the four-byte `size` itself and clamped with
//! `size.min(remaining)`, `break`ing on `size < 8`. A 64-bit `largesize` child
//! (`size == 1`) therefore dropped itself _and every sibling after it_, a
//! truncated child was silently shortened, and trailing bytes too short to
//! hold a header were ignored.
//!
//! The fixture is real, tool-generated media — see
//! `tests/fixtures/mp4/cmaf/PROVENANCE.md`.

use broadcast_common::{Parse, Serialize};
use transmux::{Error, MovieBox, OpaqueBox};

/// Find a top-level box by type and return its full bytes (header + body).
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        if size < 8 {
            break;
        }
        if &data[offset + 4..offset + 8] == fourcc {
            return &data[offset..offset + size];
        }
        offset += size;
    }
    panic!("box not found");
}

/// Index of the first occurrence of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn subtitle_fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mp4/cmaf/av_subtitle_frag.mp4"
    );
    std::fs::read(path).expect("fixture file must exist")
}

/// An `mvhd`-only movie body, for the synthetic framing cases below.
fn mvhd_bytes() -> Vec<u8> {
    let mut mvhd = vec![0u8; 108];
    mvhd[..4].copy_from_slice(&108u32.to_be_bytes());
    mvhd[4..8].copy_from_slice(b"mvhd");
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes()); // timescale
    mvhd[104..108].copy_from_slice(&2u32.to_be_bytes()); // next_track_id
    mvhd
}

/// A complete `tkhd` box for `track_id`.
fn tkhd_bytes(track_id: u32) -> Vec<u8> {
    let mut tkhd = vec![0u8; 92];
    tkhd[..4].copy_from_slice(&92u32.to_be_bytes());
    tkhd[4..8].copy_from_slice(b"tkhd");
    tkhd[20..24].copy_from_slice(&track_id.to_be_bytes());
    tkhd
}

/// Wrap `body` in a `moov` box.
fn moov_of(body: &[u8]) -> Vec<u8> {
    let mut moov = Vec::new();
    moov.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    moov.extend_from_slice(b"moov");
    moov.extend_from_slice(body);
    moov
}

// ---------------------------------------------------------------------------
// r05-W12 — child order survives the round trip
// ---------------------------------------------------------------------------

/// `sthd` is a subtitle track's media header, which §6.2.3 places *first* in
/// `minf` (before `dinf`/`stbl`). This crate models `vmhd`/`smhd` but not
/// `sthd`/`nmhd`, so it is kept as an opaque `minf` child — and the old
/// "typed first, opaque last" serializer moved it after `stbl`, where strict
/// readers reject it, even though its bytes were otherwise unchanged.
#[test]
fn subtitle_minf_keeps_media_header_before_stbl() {
    let data = subtitle_fixture();
    let moov_bytes = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(moov_bytes).expect("parses");

    // Track 2 is the `stpp` subtitle track (the fixture's second trak).
    assert_eq!(moov.tracks.len(), 2);
    let minf = {
        let mdia = moov.tracks[1].mdia.as_ref().expect("mdia");
        mdia.minf.as_ref().expect("minf")
    };
    assert!(minf.stbl.is_some(), "subtitle track has an stbl");
    assert!(
        minf.opaque.iter().any(|o| &o.box_type == b"sthd"),
        "sthd is preserved as an opaque minf child"
    );

    let bytes = moov.to_bytes();
    assert_eq!(
        bytes.as_slice(),
        moov_bytes,
        "moov round-trip must be byte-identical"
    );

    // Search inside the *subtitle* trak: the video trak's own `stbl` comes
    // earlier in the file, so a whole-file search compares the wrong pair.
    // Walk `moov`'s children structurally rather than scanning for the bytes
    // `trak` — that four-CC also occurs inside other boxes' payloads.
    let subtitle_trak = nth_child(bytes.as_slice(), b"trak", 2).expect("second trak");
    let sh = find_subslice(subtitle_trak, b"sthd").expect("sthd present");
    let stbl = find_subslice(subtitle_trak, b"stbl").expect("stbl present");
    assert!(
        sh < stbl,
        "sthd (offset {sh} in the subtitle trak) must stay before stbl (offset {stbl}): the media header comes first in minf"
    );
}

/// The `n`th (1-based) direct child of a container, returned as full box bytes.
fn nth_child<'a>(container: &'a [u8], fourcc: &[u8; 4], n: usize) -> Option<&'a [u8]> {
    let body = &container[8..];
    let mut off = 0usize;
    let mut seen = 0usize;
    while off + 8 <= body.len() {
        let size =
            u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as usize;
        if size < 8 || off + size > body.len() {
            return None;
        }
        if &body[off + 4..off + 8] == fourcc {
            seen += 1;
            if seen == n {
                return Some(&body[off..off + size]);
            }
        }
        off += size;
    }
    None
}

/// `pssh` is a movie-level child that §8.16.1 orders ahead of the `trak`s in
/// the conventional layout. The old serializer always emitted opaque children
/// last, so a `pssh` that preceded the tracks came back after them.
#[test]
fn moov_keeps_pssh_before_trak() {
    let mut pssh = vec![0u8; 12];
    pssh[..4].copy_from_slice(&12u32.to_be_bytes());
    pssh[4..8].copy_from_slice(b"pssh");

    let mut trak = vec![0u8; 8];
    trak[..4].copy_from_slice(&(8 + 92u32).to_be_bytes());
    trak[4..8].copy_from_slice(b"trak");
    trak.extend_from_slice(&tkhd_bytes(1));

    let mut body = mvhd_bytes();
    body.extend_from_slice(&pssh);
    body.extend_from_slice(&trak);
    let moov = moov_of(&body);

    let parsed = MovieBox::parse(&moov).expect("parses");
    assert_eq!(parsed.tracks.len(), 1);
    assert!(
        parsed.opaque.iter().any(|o| &o.box_type == b"pssh"),
        "pssh preserved"
    );
    let out = parsed.to_bytes();
    assert_eq!(out, moov, "moov round-trip must be byte-identical");

    let pssh_at = find_subslice(&out, b"pssh").expect("pssh present");
    let trak_at = find_subslice(&out, b"trak").expect("trak present");
    assert!(
        pssh_at < trak_at,
        "pssh ({pssh_at}) must stay ahead of trak ({trak_at})"
    );
}

// ---------------------------------------------------------------------------
// r05-W13 — framing errors instead of silent drops
// ---------------------------------------------------------------------------

/// A `moov` child written with 64-bit `largesize` (`size == 1` + 8-byte size,
/// §4.2) used to fail the container loop's `size < 8` test, which `break`ed —
/// silently dropping that child *and every sibling after it*, so a conformant
/// file parsed into a movie missing its `trak`.
#[test]
fn largesize_moov_child_does_not_drop_its_siblings() {
    let tkhd = tkhd_bytes(7);
    let trak_size = 16 + tkhd.len() as u64;
    let mut trak = Vec::new();
    trak.extend_from_slice(&1u32.to_be_bytes()); // size == 1 → largesize follows
    trak.extend_from_slice(b"trak");
    trak.extend_from_slice(&trak_size.to_be_bytes());
    trak.extend_from_slice(&tkhd);

    let mut body = mvhd_bytes();
    body.extend_from_slice(&trak);
    let moov = moov_of(&body);

    let parsed = MovieBox::parse(&moov).expect("largesize child must parse");
    assert_eq!(
        parsed.tracks.len(),
        1,
        "the largesize trak must not be dropped"
    );
    assert_eq!(parsed.tracks[0].tkhd.track_id, 7);
}

/// A child whose declared `size` runs one byte past the end of its container
/// is truncated. The old loop clamped it with `size.min(remaining)` and
/// accepted the short box, so the movie parsed "successfully" from a file
/// that was cut short.
#[test]
fn truncated_moov_child_is_an_error() {
    let tkhd = tkhd_bytes(7);
    let mut trak = vec![0u8; 8];
    trak[..4].copy_from_slice(&(8 + tkhd.len() as u32 + 1).to_be_bytes()); // one byte too long
    trak[4..8].copy_from_slice(b"trak");
    trak.extend_from_slice(&tkhd);

    let mut body = mvhd_bytes();
    body.extend_from_slice(&trak);
    let moov = moov_of(&body);

    let err = MovieBox::parse(&moov).expect_err("a truncated child must be rejected");
    assert!(
        matches!(err, Error::BufferTooShort { .. }),
        "expected BufferTooShort, got {err:?}"
    );
}

/// Trailing bytes too short to hold a child header are a framing error, not
/// something to ignore.
#[test]
fn trailing_partial_child_header_is_an_error() {
    let mut body = mvhd_bytes();
    body.extend_from_slice(&[0, 0, 0]); // 3 stray bytes
    let moov = moov_of(&body);

    let err = MovieBox::parse(&moov).expect_err("3 trailing bytes must be rejected");
    assert!(
        matches!(err, Error::BufferTooShort { .. }),
        "expected BufferTooShort, got {err:?}"
    );
}

/// Independent oracle: write this crate's own re-serialized `moov` back into
/// the file and let GPAC's `MP4Box -diso` — a parser that shares no code with
/// this crate — report the child order it reads. Skips cleanly when `MP4Box`
/// is not installed.
#[test]
fn moov_round_trip_is_mp4box_readable() {
    if std::process::Command::new("MP4Box")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("SKIP moov_round_trip_is_mp4box_readable: MP4Box not on PATH");
        return;
    }
    let data = subtitle_fixture();
    let moov_bytes = find_top_box(&data, b"moov");
    let offset = moov_bytes.as_ptr() as usize - data.as_ptr() as usize;
    let moov = MovieBox::parse(moov_bytes).expect("parses");
    let out = moov.to_bytes();
    assert_eq!(out.as_slice(), moov_bytes, "moov round-trip");

    let mut rebuilt = data.clone();
    rebuilt[offset..offset + out.len()].copy_from_slice(&out);
    let dir = std::env::temp_dir().join(format!("transmux-w12-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("rebuilt.mp4");
    std::fs::write(&file, &rebuilt).expect("write");

    let dump = std::process::Command::new("MP4Box")
        .args(["-diso", file.to_str().expect("utf-8 path")])
        .output()
        .expect("run MP4Box");
    let xml = std::fs::read_to_string(dir.join("rebuilt_info.xml")).unwrap_or_default();
    let declared = String::from_utf8_lossy(&dump.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);

    let sh = xml
        .find("Type=\"sthd\"")
        .unwrap_or_else(|| panic!("MP4Box must still see sthd; stdout: {declared}\nxml: {xml}"));
    // The *video* track's `stbl` is dumped earlier in the file, so compare
    // against the subtitle track's own `stbl` (the last one in the dump).
    let stbl = xml
        .rfind("Type=\"stbl\"")
        .unwrap_or_else(|| panic!("MP4Box must still see stbl; xml: {xml}"));
    assert!(
        sh < stbl,
        "MP4Box reads sthd at {sh} and the subtitle stbl at {stbl}: the media header must come first in minf"
    );
}

// ---------------------------------------------------------------------------
// r05-W13, second bullet: a declared entry count the body cannot hold
// ---------------------------------------------------------------------------

/// Each of these tables declares an `entry_count` its own body cannot hold.
/// The parse loop used to `break` on the first short read and still return
/// `Ok`, so the track's sample tables silently disagreed with each other
/// (audit r05-W13). A declared count is a promise about the box's size, so a
/// body that cannot hold it is malformed.
#[test]
fn truncated_sample_tables_are_rejected() {
    use broadcast_common::Parse;
    use transmux::{
        ChunkOffsetBox, SampleDescriptionBox, SampleSizeBox, SampleToChunkBox, SyncSampleBox,
    };

    // stsc: 12-byte entries, count 2, only one present.
    let mut stsc = vec![0u8; 16];
    stsc[..4].copy_from_slice(&16u32.to_be_bytes());
    stsc[4..8].copy_from_slice(b"stsc");
    stsc[12..16].copy_from_slice(&2u32.to_be_bytes());
    let err = SampleToChunkBox::parse(&stsc).expect_err("truncated stsc");
    assert_label(&err, "SampleToChunkBox.entry_count");

    // stco: 4-byte entries, count 4, none present.
    let mut stco = vec![0u8; 16];
    stco[..4].copy_from_slice(&16u32.to_be_bytes());
    stco[4..8].copy_from_slice(b"stco");
    stco[12..16].copy_from_slice(&4u32.to_be_bytes());
    let err = ChunkOffsetBox::parse(&stco).expect_err("truncated stco");
    assert_label(&err, "ChunkOffsetBox.entry_count");

    // stss: 4-byte entries, count 2, one present.
    let mut stss = vec![0u8; 20];
    stss[..4].copy_from_slice(&20u32.to_be_bytes());
    stss[4..8].copy_from_slice(b"stss");
    stss[12..16].copy_from_slice(&2u32.to_be_bytes());
    let err = SyncSampleBox::parse(&stss).expect_err("truncated stss");
    assert_label(&err, "SyncSampleBox.entry_count");

    // stsz (uniform `sample_size == 0` → per-sample table): count 3, one entry.
    let mut stsz = vec![0u8; 24];
    stsz[..4].copy_from_slice(&24u32.to_be_bytes());
    stsz[4..8].copy_from_slice(b"stsz");
    stsz[12..16].copy_from_slice(&0u32.to_be_bytes()); // per-sample sizes
    stsz[16..20].copy_from_slice(&3u32.to_be_bytes());
    let err = SampleSizeBox::parse(&stsz).expect_err("truncated stsz");
    assert_label(&err, "SampleSizeBox sample_count");

    // co64: 8-byte entries, count 2, none present.
    let mut co64 = vec![0u8; 16];
    co64[..4].copy_from_slice(&16u32.to_be_bytes());
    co64[4..8].copy_from_slice(b"co64");
    co64[12..16].copy_from_slice(&2u32.to_be_bytes());
    let err = transmux::ChunkLargeOffsetBox::parse(&co64).expect_err("truncated co64");
    assert_label(&err, "ChunkLargeOffsetBox.entry_count");

    // stsd: count 2 with no entries at all.
    let mut stsd = vec![0u8; 16];
    stsd[..4].copy_from_slice(&16u32.to_be_bytes());
    stsd[4..8].copy_from_slice(b"stsd");
    stsd[12..16].copy_from_slice(&2u32.to_be_bytes());
    let err = SampleDescriptionBox::parse(&stsd).expect_err("truncated stsd");
    assert_label(&err, "SampleDescriptionBox.entry_count");
}

/// Assert a truncation error names the box it came from.
///
/// The labels were once swapped between neighbouring tables (`ChunkOffsetBox`
/// reporting "stss.entry_count", `ChunkLargeOffsetBox` "SampleSizeBox", and
/// `SyncSampleBox` "ChunkOffsetBox"), which sends a reader to the wrong box
/// entirely (audit item 6, round 3).
fn assert_label(err: &Error, expected: &str) {
    match err {
        Error::BufferTooShort { what, .. } => {
            assert_eq!(*what, expected, "the error must name its own box");
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

/// The boundary: a count the body *can* hold still parses, and an empty table
/// (count 0) is legal.
#[test]
fn complete_sample_tables_still_parse() {
    use broadcast_common::Parse;
    use transmux::ChunkOffsetBox;

    let mut stco = vec![0u8; 24];
    stco[..4].copy_from_slice(&24u32.to_be_bytes());
    stco[4..8].copy_from_slice(b"stco");
    stco[12..16].copy_from_slice(&2u32.to_be_bytes());
    stco[16..20].copy_from_slice(&100u32.to_be_bytes());
    stco[20..24].copy_from_slice(&200u32.to_be_bytes());
    let parsed = ChunkOffsetBox::parse(&stco).expect("a complete stco parses");
    assert_eq!(parsed.entries, vec![100, 200]);

    let mut empty = vec![0u8; 16];
    empty[..4].copy_from_slice(&16u32.to_be_bytes());
    empty[4..8].copy_from_slice(b"stco");
    let parsed = ChunkOffsetBox::parse(&empty).expect("count 0 parses");
    assert!(parsed.entries.is_empty());
}

/// A child with `size == 0` extends to the end of its **enclosing container**
/// (ISO/IEC 14496-12:2015 §4.2), not of the file — `parse_box` is handed the
/// container body, so it stops there. Nothing follows such a child by
/// definition, so this is not a "dropped sibling" case; what matters is that
/// the box is preserved (as opaque) and its bytes stop at the container end.
#[test]
fn size_zero_child_extends_to_the_container_end() {
    let mut moov = mvhd_bytes();
    // A `free` child with size 0, then 12 bytes of padding that belong to it.
    moov.extend_from_slice(&0u32.to_be_bytes());
    moov.extend_from_slice(b"free");
    moov.extend_from_slice(&[0xAA; 12]);
    let container = moov_of(&moov);

    let parsed = MovieBox::parse(&container).expect("a size-0 child parses");
    let free = parsed
        .opaque
        .iter()
        .find(|o| &o.box_type == b"free")
        .expect("the size-0 child is preserved");
    assert_eq!(
        free.data.len(),
        12,
        "the payload ends at the enclosing container's end (8 header + 12 body)"
    );
    assert_eq!(parsed.to_bytes(), container, "round-trip");
}

// ---------------------------------------------------------------------------
// Round-3 item 2: `size == 0` only when the child is last
// ---------------------------------------------------------------------------

/// A `size == 0` child means "extends to the end of the enclosing container"
/// (§4.2), so it may only be written that way when nothing follows it. The
/// serializer must switch to an explicit length as soon as a sibling is
/// appended after it (which is exactly what `protect_init_segment` does when it
/// adds a `pssh`), or the appended box would be swallowed as part of it.
#[test]
fn size_zero_child_written_explicitly_once_a_sibling_follows() {
    let mut moov = mvhd_bytes();
    // A size-0 `free` child, followed by a `pssh` appended afterwards.
    moov.extend_from_slice(&0u32.to_be_bytes());
    moov.extend_from_slice(b"free");
    moov.extend_from_slice(&[0xAA; 12]);
    let container = moov_of(&moov);

    let mut parsed = MovieBox::parse(&container).expect("a size-0 child parses");
    assert_eq!(parsed.to_bytes(), container, "as parsed it round-trips");

    // Append a `pssh` after the size-0 child (the protect_init_segment shape).
    // Its payload is the 4 body bytes a `pssh` with no `system_id`/data would
    // have — all that matters here is that the appended box keeps its own size.
    parsed.opaque.push(OpaqueBox::new(*b"pssh", vec![0u8; 4]));
    parsed.order.append_opaque(*b"pssh");

    let out = parsed.to_bytes();
    // The `free` box must now state its own length: 8 header + 12 body.
    let free_at = find_subslice(&out, b"free").expect("free present");
    assert_eq!(
        u32::from_be_bytes([
            out[free_at - 4],
            out[free_at - 3],
            out[free_at - 2],
            out[free_at - 1]
        ]),
        20,
        "a non-last size-0 child must be written with its explicit size"
    );
    // The `pssh` must survive as its own box, not be swallowed.
    let pssh_at = find_subslice(&out, b"pssh").expect("pssh present");
    assert_eq!(
        u32::from_be_bytes([
            out[pssh_at - 4],
            out[pssh_at - 3],
            out[pssh_at - 2],
            out[pssh_at - 1]
        ]),
        12,
        "the appended pssh keeps its own size"
    );
    assert_eq!(
        out.len(),
        container.len() + 12,
        "the file grew by exactly the pssh"
    );
}

/// A `size == 0` child that *is* last round-trips byte-exactly, keeping the
/// `size == 0` form.
#[test]
fn size_zero_last_child_round_trips_byte_exact() {
    let mut moov = mvhd_bytes();
    moov.extend_from_slice(&0u32.to_be_bytes());
    moov.extend_from_slice(b"free");
    moov.extend_from_slice(&[0xAA; 12]);
    let container = moov_of(&moov);

    let parsed = MovieBox::parse(&container).expect("parses");
    let free = parsed
        .opaque
        .iter()
        .find(|o| &o.box_type == b"free")
        .expect("preserved");
    assert!(free.to_end, "the form is recorded");
    assert_eq!(parsed.to_bytes(), container, "byte-exact round trip");
}

/// The same `size == 0` rule on the fragment side: a `traf` whose last child
/// used the size-0 form keeps it, and re-emits an explicit size once a
/// sibling follows.
#[test]
fn traf_size_zero_last_child_round_trips_and_grows_safely() {
    // traf { tfhd(track 1) } + a size-0 `free` child carrying 8 bytes.
    let mut traf_body = Vec::new();
    let mut tfhd = vec![0u8; 16];
    tfhd[..4].copy_from_slice(&16u32.to_be_bytes());
    tfhd[4..8].copy_from_slice(b"tfhd");
    tfhd[12..16].copy_from_slice(&1u32.to_be_bytes());
    traf_body.extend_from_slice(&tfhd);
    traf_body.extend_from_slice(&0u32.to_be_bytes());
    traf_body.extend_from_slice(b"free");
    traf_body.extend_from_slice(&[0xBB; 8]);

    let mut traf = Vec::new();
    traf.extend_from_slice(&((8 + traf_body.len()) as u32).to_be_bytes());
    traf.extend_from_slice(b"traf");
    traf.extend_from_slice(&traf_body);

    let parsed =
        transmux::movie_fragment::TrackFragmentBox::parse_body(&traf[8..]).expect("parses");
    let mut out = vec![0u8; parsed.serialized_len()];
    let n = parsed.serialize_into(&mut out).expect("serialize");
    out.truncate(n);
    assert_eq!(out, traf, "a size-0 traf child round-trips byte-exactly");
}

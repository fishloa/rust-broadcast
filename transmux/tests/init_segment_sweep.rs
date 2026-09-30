//! Byte-identity sweep over every real init segment committed to the repo —
//! audit item 9.
//!
//! W12 asserted byte-identical `moov` round trips on two fixtures. This walks
//! *every* `.mp4`/`.m4s` under `fixtures/` (ffmpeg, GPAC/MP4Box and Bento4
//! output alike), parses its `moov` if it has one, and re-serializes it. A file
//! that legitimately cannot round-trip must be listed in
//! [`KNOWN_NON_IDENTICAL`] with its reason — a silent skip is not allowed.
//!
//! A `moov` whose last child leaves 1..7 trailing bytes inside the container is
//! a hard error, not something ignored: `MovieBox::parse` rejects it (audit
//! r05-W13). That matches the one other container walker in the crate
//! (`init_segment::walk_children`, which every container in the module shares):
//! a partial child header cannot be framed, so it is `BufferTooShort`. The
//! other TS/container demuxers (`TsDemux`, `PsDemux`, `WebmDemux`) stop at a
//! short tail instead, because a live capture legitimately ends mid-packet and
//! there is no enclosing length to contradict — an init segment, by contrast,
//! states its own size, so a short tail means the file is malformed.

use broadcast_common::{Parse, Serialize};
use transmux::MovieBox;

/// Fixtures that legitimately cannot round-trip byte-for-byte, with the reason.
/// Empty today: every committed init segment round-trips.
const KNOWN_NON_IDENTICAL: &[(&str, &str)] = &[];

fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 {
            return None;
        }
        if size == 1 {
            // 64-bit largesize form (§4.2).
            if off + 16 > data.len() {
                return None;
            }
            let large = u64::from_be_bytes([
                data[off + 8],
                data[off + 9],
                data[off + 10],
                data[off + 11],
                data[off + 12],
                data[off + 13],
                data[off + 14],
                data[off + 15],
            ]) as usize;
            if large < 16 || off + large > data.len() {
                return None;
            }
            if &data[off + 4..off + 8] == fourcc {
                return Some(&data[off..off + large]);
            }
            off += large;
            continue;
        }
        if off + size > data.len() {
            return None;
        }
        if &data[off + 4..off + 8] == fourcc {
            return Some(&data[off..off + size]);
        }
        off += size;
    }
    None
}

/// Every `.mp4`/`.m4s`/`.mov` under `fixtures/` (and the crate-local ones).
fn fixture_files() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    let mut stack = vec![root.join("fixtures"), root.join("transmux/tests/fixtures")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("mp4" | "m4s" | "mov")
            ) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Every committed init segment's `moov` must parse -> serialize to the same
/// bytes.
#[test]
fn every_committed_init_segment_round_trips_byte_identically() {
    let files = fixture_files();
    assert!(
        files.len() >= 20,
        "the sweep found only {} media files — the glob is wrong",
        files.len()
    );
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
            .unwrap_or(path)
            .display()
            .to_string();
        if KNOWN_NON_IDENTICAL
            .iter()
            .any(|(name, _)| rel.ends_with(name))
        {
            continue;
        }
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let Some(moov_bytes) = find_box(&data, b"moov") else {
            // A media segment with no `moov` (an fMP4 chunk) has no init to
            // round-trip; that is not a skip of the claim, it is the box's
            // absence.
            continue;
        };
        let moov = match MovieBox::parse(moov_bytes) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{rel}: parse failed: {e}"));
                continue;
            }
        };
        let out = moov.to_bytes();
        if out != moov_bytes {
            failures.push(format!(
                "{rel}: round trip differs ({} vs {} bytes)",
                out.len(),
                moov_bytes.len()
            ));
        }
        checked += 1;
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} init segments did not round-trip:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        checked >= 10,
        "the sweep only round-tripped {checked} init segments"
    );
}

/// A `moov` with 1..7 trailing bytes inside the container is `BufferTooShort`:
/// the walk cannot frame a partial child header, and the container states its
/// own length, so the file is malformed rather than merely short.
#[test]
fn a_moov_with_trailing_partial_bytes_is_rejected() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/mp4/h264_high.mp4");
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("SKIP a_moov_with_trailing_partial_bytes_is_rejected: fixture missing");
        return;
    };
    let moov_bytes = find_box(&data, b"moov").expect("a moov");
    for extra in 1..8usize {
        let mut patched = moov_bytes.to_vec();
        // Grow the moov by `extra` bytes of junk and update its size, so the
        // container claims the bytes but cannot frame them.
        patched.extend_from_slice(&alloc_pad(extra));
        let new_size = (moov_bytes.len() + extra) as u32;
        patched[..4].copy_from_slice(&new_size.to_be_bytes());
        let err = MovieBox::parse(&patched).expect_err("trailing partial bytes must be rejected");
        assert!(
            matches!(err, transmux::Error::BufferTooShort { .. }),
            "extra={extra}: expected BufferTooShort, got {err:?}"
        );
    }
}

fn alloc_pad(n: usize) -> Vec<u8> {
    vec![0u8; n]
}

// ---------------------------------------------------------------------------
// Round-3 item 3: a `vp09`/`av01` entry's child order and truncation
// ---------------------------------------------------------------------------

/// Build a minimal `vp09` entry whose `pasp` **precedes** `vpcC` and whose
/// `colr` follows it — the shape real muxers write. The extras must keep their
/// wire positions, or a parse -> serialize reorders them (audit item 3).
#[test]
fn vp09_child_order_is_preserved_around_the_config_box() {
    use broadcast_common::Parse;
    use transmux::{SampleDescriptionBox, SampleEntryVariant};

    // A vpcC v1 record (the only version this crate accepts): 4 bytes of
    // FullBox header + the 8-byte fixed record.
    let mut vpcc = Vec::new();
    vpcc.extend_from_slice(&20u32.to_be_bytes());
    vpcc.extend_from_slice(b"vpcC");
    vpcc.push(1); // version 1
    vpcc.extend_from_slice(&[0, 0, 0]); // flags
    vpcc.push(0); // profile
    vpcc.push(10); // level
    vpcc.push(0x82); // bitDepth(4) | chroma(3) | fullRange(1)
    vpcc.push(0); // colourPrimaries
    vpcc.push(0); // transferCharacteristics
    vpcc.push(0); // matrixCoefficients
    vpcc.extend_from_slice(&0u16.to_be_bytes()); // codecInitializationDataSize

    // pasp { hSpacing, vSpacing }
    let mut pasp = Vec::new();
    pasp.extend_from_slice(&16u32.to_be_bytes());
    pasp.extend_from_slice(b"pasp");
    pasp.extend_from_slice(&1u32.to_be_bytes());
    pasp.extend_from_slice(&1u32.to_be_bytes());

    // colr { nclx colour_type, primaries, transfer, matrix, full_range }
    let mut colr = Vec::new();
    colr.extend_from_slice(&19u32.to_be_bytes());
    colr.extend_from_slice(b"colr");
    colr.extend_from_slice(b"nclx");
    colr.extend_from_slice(&[0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x80]);

    // A 78-byte VisualSampleEntry header/body, then pasp, vpcC, colr.
    let visual_len = 78usize;
    let total = 8 + visual_len + pasp.len() + vpcc.len() + colr.len();
    let mut entry = Vec::new();
    entry.extend_from_slice(&(total as u32).to_be_bytes());
    entry.extend_from_slice(b"vp09");
    let mut visual = vec![0u8; visual_len];
    visual[6..8].copy_from_slice(&1u16.to_be_bytes()); // data_reference_index
    visual[24..26].copy_from_slice(&64u16.to_be_bytes()); // width
    visual[26..28].copy_from_slice(&64u16.to_be_bytes()); // height
    visual[28..32].copy_from_slice(&0x0048_0000u32.to_be_bytes()); // horizresolution
    visual[32..36].copy_from_slice(&0x0048_0000u32.to_be_bytes()); // vertresolution
    visual[40..42].copy_from_slice(&1u16.to_be_bytes()); // frame_count
    entry.extend_from_slice(&visual);
    entry.extend_from_slice(&pasp);
    entry.extend_from_slice(&vpcc);
    entry.extend_from_slice(&colr);

    let parsed = SampleEntryVariant::Vp09(Box::new(
        transmux::vp9::Vp9SampleEntry::parse_entry(&entry).expect("parses"),
    ));
    assert!(
        !parsed_config_first(&parsed),
        "pasp precedes vpcC, so the config box is not first"
    );
    let mut stsd = Vec::new();
    let mut stsd_body = Vec::new();
    stsd_body.extend_from_slice(&0u32.to_be_bytes()); // version/flags
    stsd_body.extend_from_slice(&1u32.to_be_bytes()); // entry_count
    stsd_body.extend_from_slice(&entry);
    stsd.extend_from_slice(&((8 + stsd_body.len()) as u32).to_be_bytes());
    stsd.extend_from_slice(b"stsd");
    stsd.extend_from_slice(&stsd_body);

    let boxed = SampleDescriptionBox::parse(&stsd).expect("parses stsd");
    let out = boxed.to_bytes();
    assert_eq!(out, stsd, "the entry's child order must round-trip exactly");
    // And directly: the emitted bytes put `pasp` before `vpcC`.
    let pasp_at = out.windows(4).position(|w| w == b"pasp").expect("pasp");
    let vpcc_at = out.windows(4).position(|w| w == b"vpcC").expect("vpcC");
    let colr_at = out.windows(4).position(|w| w == b"colr").expect("colr");
    assert!(
        pasp_at < vpcc_at && vpcc_at < colr_at,
        "wire order preserved"
    );
}

/// Whether the config box sits *first* among the entry's children.
fn parsed_config_first(entry: &transmux::SampleEntryVariant) -> bool {
    use transmux::SampleEntryChild;
    let children = match entry {
        transmux::SampleEntryVariant::Vp09(e) => &e.children,
        transmux::SampleEntryVariant::Av01(e) => &e.children,
        _ => panic!("not a vp09/av01 entry"),
    };
    matches!(children.first(), Some(SampleEntryChild::Config))
}

/// A truncated child inside a `vp09` entry is an error, not a silently
/// shortened box: `other_children` used to clamp with `min(len)` and keep a
/// size field with fewer bytes behind it (audit item 3).
#[test]
fn vp09_truncated_child_is_an_error() {
    use transmux::vp9::Vp9SampleEntry;

    // A *complete* vpcC (20 bytes) so the only fault in the entry is the
    // truncated child after it.
    let mut vpcc = vec![0u8; 20];
    vpcc[..4].copy_from_slice(&20u32.to_be_bytes());
    vpcc[4..8].copy_from_slice(b"vpcC");
    vpcc[8] = 1; // version 1
    vpcc[9..12].copy_from_slice(&[0, 0, 0]); // flags
    let mut colr = Vec::new();
    colr.extend_from_slice(&19u32.to_be_bytes());
    colr.extend_from_slice(b"colr");
    colr.extend_from_slice(&[0u8; 7]);

    let visual_len = 78usize;
    let total = 8 + visual_len + vpcc.len() + colr.len();
    let mut entry = Vec::new();
    entry.extend_from_slice(&(total as u32).to_be_bytes());
    entry.extend_from_slice(b"vp09");
    let mut visual = vec![0u8; visual_len];
    visual[6..8].copy_from_slice(&1u16.to_be_bytes());
    visual[24..26].copy_from_slice(&64u16.to_be_bytes());
    visual[26..28].copy_from_slice(&64u16.to_be_bytes());
    visual[28..32].copy_from_slice(&0x0048_0000u32.to_be_bytes());
    visual[32..36].copy_from_slice(&0x0048_0000u32.to_be_bytes());
    visual[40..42].copy_from_slice(&1u16.to_be_bytes());
    entry.extend_from_slice(&visual);
    entry.extend_from_slice(&vpcc);
    // Declare a 400-byte `colr` with only 11 bytes present.
    let mut short = vec![0u8; 11];
    short[..4].copy_from_slice(&400u32.to_be_bytes());
    short[4..8].copy_from_slice(b"colr");
    entry.extend_from_slice(&short);

    let err = Vp9SampleEntry::parse_entry(&entry)
        .expect_err("a truncated sample-entry child must be rejected");

    assert!(
        matches!(err, transmux::Error::BufferTooShort { .. }),
        "expected BufferTooShort, got {err:?}"
    );
}

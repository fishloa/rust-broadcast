//! `uuid` child boxes must survive a parse -> serialize round trip — audit
//! fix wave H, item 1.
//!
//! `parse_box` returns a body that *excludes* the 16-byte `uuid` usertype
//! (`BoxHeader::header_size()` counts it). Every container stores
//! `OpaqueBox::new(child.four_cc, child.body.to_vec())`, so a `uuid` child
//! whose payload was kept from `body` lost its usertype and re-serialised
//! corrupted: 8 bytes shorter than the original and shifted, with the
//! `usertype` replaced by the first 8 payload bytes.
//!
//! Fixtures: real ffmpeg output with real `uuid` children spliced in —
//! see `tests/fixtures/mp4/uuid_boxes/README.md`.

use broadcast_common::{Parse, Serialize, Unpackage};
use transmux::{Fmp4Demux, MovieBox};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mp4/uuid_boxes");

fn read(name: &str) -> Vec<u8> {
    let p = format!("{DIR}/{name}");
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 {
            break;
        }
        if &data[off + 4..off + 8] == fourcc {
            return &data[off..off + size];
        }
        off += size;
    }
    panic!("box not found");
}

/// The `moov` of `moov_uuid.mp4` carries a `uuid` child (a PlayReady/ISMV
/// `pssh`). Parsing and re-serialising must be byte-identical.
#[test]
fn moov_uuid_child_round_trips_byte_exact() {
    let data = read("moov_uuid.mp4");
    let moov_bytes = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(moov_bytes).expect("parses");
    let uuid_child = moov
        .opaque
        .iter()
        .find(|o| &o.box_type == b"uuid")
        .expect("moov has a uuid child");
    assert!(
        uuid_child.data.len() >= 16,
        "the usertype must be carried with the opaque body"
    );
    let out = moov.to_bytes();
    assert_eq!(out, moov_bytes, "moov round-trip must be byte-identical");
}

/// A media segment's `moof` carries a `uuid` child, and its `traf` carries
/// another. `Fmp4Demux` re-serialises neither, but the *parsed* boxes must
/// keep every child so `protect_media_segment` (and any rewrite) does not
/// strip them; this pins the parse side against the real fixture.
#[test]
fn moof_and_traf_uuid_children_are_preserved_on_parse() {
    let data = read("segment_uuid.mp4");
    let media = Fmp4Demux::new()
        .unpackage(data.as_slice())
        .expect("the segment demuxes");
    assert!(
        media.tracks.iter().any(|t| !t.samples.is_empty()),
        "the segment still yields samples with uuid children present"
    );

    // Parse the `moof`/`traf` directly and check the uuid children survive a
    // round trip byte-for-byte.
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > data.len() {
            break;
        }
        if &data[off + 4..off + 8] == b"moof" {
            let moof_bytes = &data[off..off + size];
            let moof = transmux::MovieFragmentBox::parse_body(&moof_bytes[8..]).expect("moof");
            let moof_has_uuid = moof
                .order
                .iter()
                .any(|c| matches!(c, transmux::movie_fragment::MoofChild::Opaque(o) if &o.box_type == b"uuid"));
            assert!(moof_has_uuid, "the moof-level uuid child is preserved");
            let traf_has_uuid = moof.traf.iter().any(|t| {
                t.order.iter().any(|c| {
                    matches!(c, transmux::movie_fragment::TrafChild::Opaque(o) if &o.box_type == b"uuid")
                })
            });
            assert!(traf_has_uuid, "the traf-level uuid child is preserved");
            assert_eq!(
                moof.to_bytes(),
                moof_bytes,
                "moof round-trip must be byte-identical with uuid children"
            );
            return;
        }
        off += size;
    }
    panic!("no moof in the fixture");
}

/// Independent oracle: write this crate's own round-tripped `moov` back into
/// the file and let `mp4dump` (Bento4, no shared code) read the uuid child
/// with its original extended type and size. Skips cleanly when `mp4dump` is
/// absent.
#[test]
fn round_tripped_uuid_is_mp4dump_readable() {
    if std::process::Command::new("mp4dump")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("SKIP round_tripped_uuid_is_mp4dump_readable: mp4dump not on PATH");
        return;
    }
    let data = read("moov_uuid.mp4");
    let moov_bytes = find_top_box(&data, b"moov");
    let offset = moov_bytes.as_ptr() as usize - data.as_ptr() as usize;
    let moov = MovieBox::parse(moov_bytes).expect("parses");
    let out = moov.to_bytes();
    assert_eq!(out.as_slice(), moov_bytes, "moov round-trip");

    let mut rebuilt = data.clone();
    rebuilt[offset..offset + out.len()].copy_from_slice(&out);
    let dir = std::env::temp_dir().join(format!("transmux-uuid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("rebuilt.mp4");
    std::fs::write(&file, &rebuilt).expect("write");
    let dump = std::process::Command::new("mp4dump")
        .arg(file.to_str().expect("utf-8 path"))
        .output()
        .expect("run mp4dump");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&dump.stdout).into_owned();
    assert!(
        text.contains("D08A4F1810F3-4A82-B6C8-32D8-ABA183D3"),
        "mp4dump must still read the uuid extended type: {text}"
    );
    assert!(
        text.contains("size=24+64"),
        "the uuid child keeps its original size (8 + 16 usertype + 64 body): {text}"
    );
}

// ---------------------------------------------------------------------------
// Round-3 item 1: the `largesize` form must round-trip too
// ---------------------------------------------------------------------------

/// A `uuid` child written in the `size == 1` + 64-bit `largesize` form
/// (ISO/IEC 14496-12:2015 §4.2). The opaque payload must exclude the 8
/// `largesize` bytes — the serializer writes its own header — and the form
/// itself must be re-emitted, or the box comes back with 8 junk bytes in its
/// body.
#[test]
fn largesize_uuid_child_round_trips_in_every_container() {
    // A 16-byte usertype + 4 payload bytes, 16-byte header form.
    let usertype: [u8; 16] = *b"0123456789abcdef";
    let payload: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
    let total = 16 + usertype.len() + payload.len();
    let mut child = Vec::new();
    child.extend_from_slice(&1u32.to_be_bytes()); // size == 1 → largesize
    child.extend_from_slice(b"uuid");
    child.extend_from_slice(&(total as u64).to_be_bytes());
    child.extend_from_slice(&usertype);
    child.extend_from_slice(payload);

    // --- a container that models the child opaquely (moov) ---
    let mut mvhd = vec![0u8; 108];
    mvhd[..4].copy_from_slice(&108u32.to_be_bytes());
    mvhd[4..8].copy_from_slice(b"mvhd");
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mvhd[104..108].copy_from_slice(&2u32.to_be_bytes());

    let mut body = mvhd.clone();
    body.extend_from_slice(&child);
    let mut moov = Vec::new();
    moov.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    moov.extend_from_slice(b"moov");
    moov.extend_from_slice(&body);

    let parsed = MovieBox::parse(&moov).expect("a largesize uuid parses");
    let uuid_child = parsed
        .opaque
        .iter()
        .find(|o| &o.box_type == b"uuid")
        .expect("preserved");
    assert!(uuid_child.largesize, "the form is recorded");
    assert_eq!(
        uuid_child.data.len(),
        usertype.len() + payload.len(),
        "the payload is the usertype + body, with no largesize bytes"
    );
    assert_eq!(uuid_child.data[..16], usertype, "the usertype is kept");
    let out = parsed.to_bytes();
    assert_eq!(out, moov, "moov round-trip must be byte-identical");

    // --- a plain largesize child (not a uuid) ---
    let mut free_body = vec![0u8; 12];
    let free_total = 16 + free_body.len();
    let mut free = Vec::new();
    free.extend_from_slice(&1u32.to_be_bytes());
    free.extend_from_slice(b"free");
    free.extend_from_slice(&(free_total as u64).to_be_bytes());
    free.append(&mut free_body);
    let mut body2 = mvhd.clone();
    body2.extend_from_slice(&free);
    let mut moov2 = Vec::new();
    moov2.extend_from_slice(&(8 + body2.len() as u32).to_be_bytes());
    moov2.extend_from_slice(b"moov");
    moov2.extend_from_slice(&body2);
    let parsed = MovieBox::parse(&moov2).expect("a largesize free parses");
    assert_eq!(
        parsed.to_bytes(),
        moov2,
        "a largesize non-uuid child must round-trip too"
    );

    // --- the fragment half: traf and moof ---
    let mut traf = Vec::new();
    let inner = {
        let mut t = vec![0u8; 16];
        t[..4].copy_from_slice(&16u32.to_be_bytes());
        t[4..8].copy_from_slice(b"tfhd");
        t[12..16].copy_from_slice(&1u32.to_be_bytes()); // track_id
        t
    };
    let traf_total = 8 + inner.len() + child.len();
    traf.extend_from_slice(&(traf_total as u32).to_be_bytes());
    traf.extend_from_slice(b"traf");
    traf.extend_from_slice(&inner);
    traf.extend_from_slice(&child);

    let mut mfhd = vec![0u8; 16];
    mfhd[..4].copy_from_slice(&16u32.to_be_bytes());
    mfhd[4..8].copy_from_slice(b"mfhd");
    mfhd[12..16].copy_from_slice(&1u32.to_be_bytes());

    let mut moof_body = mfhd.clone();
    moof_body.extend_from_slice(&traf);
    let mut moof = Vec::new();
    moof.extend_from_slice(&(8 + moof_body.len() as u32).to_be_bytes());
    moof.extend_from_slice(b"moof");
    moof.extend_from_slice(&moof_body);

    let parsed = transmux::MovieFragmentBox::parse_body(&moof[8..]).expect("moof parses");
    let out = {
        let mut b = vec![0u8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut b).expect("serialize");
        b.truncate(n);
        b
    };
    assert_eq!(
        out, moof,
        "moof with a largesize uuid traf child round-trips"
    );
}

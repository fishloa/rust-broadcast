//! A structural box keeps the bytes its declared table does not account for:
//! every committed MP4-family fixture still reads after each of its structural
//! boxes grows by four bytes, the typed results are unchanged, and the bytes
//! survive a rewrite.
//!
//! Growth is applied by rebuilding the file: the box's own size and every
//! ancestor's grow with it, so the box tree stays consistent.

use std::path::{Path, PathBuf};

use isobmff::Error;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep};
use isobmff::reader::{Mode, Reader, ReaderConfig, Source, Step};
use test_fixtures as fixtures;

/// Bytes appended to every grown box.
const TRAILER: [u8; 4] = [0xD1, 0x2C, 0x9B, 0x5E];
/// Width of a compact box header.
const HEADER: usize = 8;
/// The structural boxes this crate carries trailing bytes for, by four-CC.
const TARGETS: [&[u8; 4]; 10] = [
    b"dref", b"stsd", b"stsz", b"saiz", b"sidx", b"tfra", b"ipma", b"tfhd", b"trun", b"chnl",
];

/// Containers the walk descends into, with the offset of their first child
/// inside the body. An entry a walk cannot validate is left alone.
const CONTAINERS: [(&[u8; 4], usize); 19] = [
    (b"moov", 0),
    (b"trak", 0),
    (b"mdia", 0),
    (b"minf", 0),
    (b"stbl", 0),
    (b"dinf", 0),
    (b"moof", 0),
    (b"traf", 0),
    (b"mvex", 0),
    (b"mfra", 0),
    (b"edts", 0),
    (b"udta", 0),
    (b"iprp", 0),
    (b"ipco", 0),
    (b"grpl", 0),
    (b"meta", 4),
    (b"stsd", 8),
    (b"dref", 8),
    // An ISO audio sample entry: 6 reserved, data_reference_index, the
    // QuickTime sound-description fields (8 + 2 + 2 + 2 + 2 + 4).
    (b"mp4a", 28),
];

fn container_prefix(fourcc: &[u8; 4]) -> Option<usize> {
    CONTAINERS
        .iter()
        .find(|(name, _)| *name == fourcc)
        .map(|(_, prefix)| *prefix)
}

/// Rebuild `region` with four bytes appended to every box in `kinds` that is
/// directly in it or in a container below it. Returns the region and how many
/// boxes grew.
fn grow(region: &[u8], kinds: &[&[u8; 4]], seen: &mut Vec<[u8; 4]>) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(region.len() + TRAILER.len());
    let mut grown = 0usize;
    let mut at = 0usize;
    while at + HEADER <= region.len() {
        let size = u32::from_be_bytes([region[at], region[at + 1], region[at + 2], region[at + 3]])
            as usize;
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&region[at + 4..at + HEADER]);
        if size == 0 || size == 1 || size < HEADER || at + size > region.len() {
            // A to-EOF or largesize box is copied as it is: growing it needs a
            // header rewrite this walk does not attempt.
            out.extend_from_slice(&region[at..]);
            at = region.len();
            break;
        }
        let body = &region[at + HEADER..at + size];
        let grew = if kinds.contains(&&fourcc) {
            let mut new_body = body.to_vec();
            new_body.extend_from_slice(&TRAILER);
            grown += 1;
            seen.push(fourcc);
            Some(new_body)
        } else if let Some(prefix) = container_prefix(&fourcc)
            && prefix <= body.len()
        {
            let (child, count) = grow(&body[prefix..], kinds, seen);
            if count == 0 {
                None
            } else {
                let mut new_body = body[..prefix].to_vec();
                new_body.extend_from_slice(&child);
                grown += count;
                Some(new_body)
            }
        } else {
            None
        };
        match grew {
            Some(new_body) => {
                let total = HEADER + new_body.len();
                let Ok(total32) = u32::try_from(total) else {
                    out.extend_from_slice(&region[at..at + size]);
                    at += size;
                    continue;
                };
                out.extend_from_slice(&total32.to_be_bytes());
                out.extend_from_slice(&fourcc);
                out.extend_from_slice(&new_body);
            }
            None => out.extend_from_slice(&region[at..at + size]),
        }
        at += size;
    }
    if at < region.len() {
        out.extend_from_slice(&region[at..]);
    }
    (out, grown)
}

fn read_all(range: isobmff::ByteRange) -> (usize, usize) {
    let start = usize::try_from(range.offset).expect("offset");
    let len = usize::try_from(range.len).expect("length");
    (start, start + len)
}

/// What one movie read typed, in a form two reads can be compared by.
#[derive(Debug, PartialEq, Eq)]
struct MovieRead {
    /// Per track: id, handler, timescale, duration, and its entries' four-CCs.
    tracks: Vec<String>,
    samples: usize,
    fragments: usize,
    warnings: Vec<String>,
}

fn movie(bytes: &[u8], mode: Mode) -> Result<MovieRead, Error> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_source(Source::Seekable)
        .with_mode(mode);
    let mut reader = Reader::new(config);
    let mut read = MovieRead {
        tracks: Vec::new(),
        samples: 0,
        fragments: 0,
        warnings: Vec::new(),
    };
    loop {
        match reader.step()? {
            Step::Need(range) => {
                let (start, end) = read_all(range);
                reader.supply(range.offset, &bytes[start..end])?;
            }
            Step::Tracks(list) => {
                for info in list {
                    let entries: Vec<String> = info
                        .entries
                        .iter()
                        .map(|entry| format!("{:?}", entry.fourcc))
                        .collect();
                    read.tracks.push(format!(
                        "{} {:?} {} {} [{}]",
                        info.track_id.value(),
                        info.handler,
                        info.timescale.get(),
                        info.duration,
                        entries.join(",")
                    ));
                }
            }
            Step::Sample(_) => read.samples += 1,
            Step::Fragment(_) => read.fragments += 1,
            Step::Damage(damage) => read.warnings.push(damage.what().to_owned()),
            Step::Done => break,
            _ => {}
        }
    }
    Ok(read)
}

fn item(bytes: &[u8]) -> Result<(usize, usize, Vec<String>, Vec<u8>), Error> {
    let config = ItemReaderConfig::default().with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    let mut image = None;
    loop {
        match reader.step()? {
            ItemStep::Need(range) => {
                let (start, end) = read_all(range);
                reader.supply(range.offset, &bytes[start..end])?;
            }
            ItemStep::Loaded(model) => image = Some(model.clone()),
            ItemStep::Done => break,
            _ => {}
        }
    }
    let image = image.ok_or(Error::InvalidInput("no item model"))?;
    let mut properties = 0usize;
    for item in image.items() {
        properties += item.properties().len();
    }
    let findings = image
        .structure_findings()
        .iter()
        .map(|finding| finding.name().to_owned())
        .collect();
    let reemit = image.reemit_into(bytes)?;
    Ok((image.items().len(), properties, findings, reemit))
}

fn media_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            media_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("mp4" | "m4s" | "m4a" | "mov" | "heic" | "avif")
        ) {
            out.push(path);
        }
    }
}

fn committed_media() -> Vec<PathBuf> {
    let mut files = Vec::new();
    media_files(&fixtures::root(), &mut files);
    if let Some(private) = fixtures::private("") {
        media_files(&private, &mut files);
    }
    files.sort();
    files
}

/// The boxes whose position inside the movie does not move any byte a sample
/// offset resolves through, so growing them must leave every typed result
/// untouched.
const TABLE_TARGETS: [&[u8; 4]; 6] = [b"dref", b"stsd", b"stsz", b"saiz", b"chnl", b"ipma"];

fn read_counts(name: &str, bytes: &[u8], grown: &[u8], seen: &[[u8; 4]]) {
    let mut reported = false;
    let base = movie(bytes, Mode::Permissive);
    let after = movie(grown, Mode::Permissive);
    match (&base, &after) {
        (Ok(before), Ok(after)) => {
            assert_eq!(
                before.tracks, after.tracks,
                "{name}: the tracks changed by carrying trailing bytes"
            );
            if before.fragments == 0 {
                assert_eq!(
                    before.samples, after.samples,
                    "{name}: sample count changed by carrying trailing bytes; \
                     grew {seen:?}; warnings {:?}",
                    after.warnings
                );
            }
            reported |= after.warnings.iter().any(|w| w == "trailing-bytes");
        }
        (Err(before), Err(after)) => assert_eq!(
            before.to_string(),
            after.to_string(),
            "{name}: growth changed why the read failed"
        ),
        (Ok(_), Err(err)) => panic!("{name}: growth turned a read into {err}"),
        (Err(err), Ok(_)) => panic!("{name}: growth turned {err} into a read"),
    }
    let base = item(bytes);
    let after = item(grown);
    match (&base, &after) {
        (Ok(before), Ok(after)) => {
            assert_eq!(before.0, after.0, "{name}: item count changed");
            assert_eq!(before.1, after.1, "{name}: property count changed");
            assert_eq!(after.3, grown, "{name}: the grown file does not re-emit");
            reported |= after.2.iter().any(|f| f == "trailing-bytes");
        }
        (Err(before), Err(after)) => assert_eq!(
            before.to_string(),
            after.to_string(),
            "{name}: growth changed why the item read failed"
        ),
        (Ok(_), Err(err)) => panic!("{name}: growth turned an item read into {err}"),
        (Err(err), Ok(_)) => panic!("{name}: growth turned {err} into an item read"),
    }
    assert!(
        reported,
        "{name}: the carried bytes went unreported; grew {seen:?}"
    );
}

#[test]
fn growing_a_table_box_leaves_every_typed_result_alone() {
    let mut files = 0usize;
    let mut grown_boxes = 0usize;
    for path in committed_media() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut seen = Vec::new();
        let (grown, count) = grow(&bytes, &TABLE_TARGETS, &mut seen);
        if count == 0 {
            continue;
        }
        files += 1;
        grown_boxes += count;
        let name = path.display().to_string();
        read_counts(&name, &bytes, &grown, &seen);
        if let Ok((_, _, _, reemit)) = item(&grown) {
            assert_eq!(reemit, grown, "{name}: the grown file does not re-emit");
        }
    }
    assert!(files > 0, "no fixture carried a table box to grow");
    println!("table trailing: {files} files, {grown_boxes} boxes grown");
}

#[test]
fn every_structural_box_can_grow_without_costing_the_read() {
    let mut files = 0usize;
    let mut grown_boxes = 0usize;
    let mut reported = 0usize;
    for path in committed_media() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut seen = Vec::new();
        let (grown, count) = grow(&bytes, &TARGETS, &mut seen);
        if count == 0 {
            continue;
        }
        files += 1;
        grown_boxes += count;
        let name = path.display().to_string();
        let mut carried = false;
        // Growing an index box (`sidx`, `tfra`) or a fragment header moves the
        // bytes a sample offset resolves against, so only the outcome, the
        // track facts and the carriage are compared here; the table targets
        // above compare every typed result too.
        let before = movie(&bytes, Mode::Permissive);
        let after = movie(&grown, Mode::Permissive);
        match (&before, &after) {
            (Ok(before), Ok(after)) => {
                assert_eq!(
                    before.tracks, after.tracks,
                    "{name}: the tracks changed by carrying trailing bytes"
                );
                carried |= after.warnings.iter().any(|w| w == "trailing-bytes");
            }
            (Err(before), Err(after)) => assert_eq!(
                before.to_string(),
                after.to_string(),
                "{name}: growth changed why the read failed"
            ),
            (Ok(_), Err(err)) => panic!("{name}: growth turned a read into {err}"),
            (Err(err), Ok(_)) => panic!("{name}: growth turned {err} into a read"),
        }
        let base = item(&bytes);
        let after = item(&grown);
        match (&base, &after) {
            (Ok(before), Ok(after)) => {
                assert_eq!(before.0, after.0, "{name}: item count changed");
                assert_eq!(before.1, after.1, "{name}: property count changed");
                assert_eq!(
                    after.3, grown,
                    "{name}: the grown file does not re-emit byte-exact"
                );
                carried |= after.2.iter().any(|f| f == "trailing-bytes");
            }
            (Err(before), Err(after)) => assert_eq!(
                before.to_string(),
                after.to_string(),
                "{name}: growth changed why the item read failed"
            ),
            (Ok(_), Err(err)) => panic!("{name}: growth turned an item read into {err}"),
            (Err(err), Ok(_)) => panic!("{name}: growth turned {err} into an item read"),
        }
        assert!(
            carried,
            "{name}: a grown box went unreported; grew {seen:?}"
        );
        reported += 1;
    }
    assert!(files > 0, "no fixture carried a structural box to grow");
    assert_eq!(reported, files);
    println!("structural trailing: {files} files, {grown_boxes} boxes grown, {reported} reported");
}

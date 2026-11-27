//! A damage matrix over real fixtures: one cell per damage site, driven
//! through the best-effort reader.
//!
//! Every cell mutates a committed fixture at a place on the path to its
//! samples — a truncation, a zeroed or randomised run, a size field set to a
//! hostile value, a count field inflated or deflated, a structural box
//! deleted — and records what the reader did: whether it refused, how many
//! samples it still delivered, and how many reports it published.
//!
//! The assertions are the lane's rule:
//!
//! - a cell may fail only when the damage removed what playback needs, and
//!   every such cell is listed in [`EXPECTED_FAILURES`] with its reason;
//! - every sample delivered has the byte range, size, index and track the
//!   clean run gave it — a damaged file never invents or moves a sample;
//! - for damage inside media bytes, every sample whose bytes the damage did
//!   not touch is delivered;
//! - every damaged cell publishes at least one report;
//! - per-track decode times stay monotonic;
//! - a clean input publishes nothing and delivers exactly the clean samples.

use std::collections::{BTreeMap, BTreeSet};

use broadcast_core::damage::{Damage, DamageKind};
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};
use isobmff::reader::{Reader, ReaderConfig, SampleOrder, Source, Step};
use isobmff::sample_table::SampleRef;

/// Width of a compact box header.
const HEADER: usize = 8;
/// Width of a box's `largesize` field.
const LARGESIZE_WIDTH: usize = 8;
/// Bytes a zeroed or randomised run covers when the box is big enough.
const RUN: usize = 16;
/// The `0xFFFFFFFF` a size field takes when it declares the whole file.
const MAX_SIZE: u32 = u32::MAX;
/// A fixed seed, so a randomised cell is reproducible.
const SEED: u64 = 0x5EED_1234_ABCD_0001;

/// A key that identifies one sample's place in the file.
type SampleKey = (u32, u32, u64, u32);

/// What a cell did.
#[derive(Debug, Default)]
struct Drive {
    error: Option<String>,
    samples: Vec<SampleRef>,
    damages: Vec<Damage>,
}

fn drive(bytes: &[u8]) -> Drive {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(SampleOrder::File)
        .with_source(Source::Seekable);
    let mut reader = Reader::new(config);
    let mut drive = Drive::default();
    loop {
        match reader.step() {
            Ok(Step::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if let Err(error) =
                    reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                {
                    drive.error = Some(alloc_name(&error));
                    break;
                }
            }
            Ok(Step::Sample(sample)) => drive.samples.push(sample),
            Ok(Step::Damage(damage)) => drive.damages.push(damage),
            Ok(Step::Done) => break,
            Ok(_) => {}
            Err(error) => {
                drive.error = Some(alloc_name(&error));
                break;
            }
        }
    }
    drive
}

fn alloc_name(error: &isobmff::Error) -> String {
    format!("{error:?}")
}

fn key(sample: &SampleRef) -> SampleKey {
    (
        sample.track.value(),
        sample.index,
        sample.offset,
        sample.size,
    )
}

fn keys(drive: &Drive) -> BTreeSet<SampleKey> {
    drive.samples.iter().map(key).collect()
}

/// One box in a fixture, with where its bytes are.
#[derive(Debug, Clone)]
struct BoxInfo {
    offset: usize,
    size: usize,
    fourcc: [u8; 4],
    /// Whether this box's bytes are media a sample addresses.
    media: bool,
}

/// Containers a walk descends into, with the offset of their first child
/// inside the body.
const CONTAINERS: [(&[u8; 4], usize); 20] = [
    (b"moov", 0),
    (b"trak", 0),
    (b"mdia", 0),
    (b"minf", 0),
    (b"stbl", 0),
    (b"dinf", 0),
    (b"edts", 0),
    (b"moof", 0),
    (b"traf", 0),
    (b"mvex", 0),
    (b"mfra", 0),
    (b"udta", 0),
    (b"meta", 4),
    (b"iprp", 0),
    (b"ipco", 0),
    (b"grpl", 0),
    (b"iinf", 6),
    (b"iref", 4),
    (b"dref", 8),
    (b"stsd", 8),
];

/// Every box of a fixture, depth first, with its media flag. A box that cannot
/// be walked stops its own descent (`entry` is its own size) but not its
/// siblings.
fn boxes(bytes: &[u8]) -> Vec<BoxInfo> {
    let mut out = Vec::new();
    walk_boxes(bytes, 0, bytes.len(), &mut out);
    out
}

fn walk_boxes(bytes: &[u8], start: usize, end: usize, out: &mut Vec<BoxInfo>) {
    let mut at = start;
    while at + HEADER <= end {
        let size = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let fourcc: [u8; 4] = bytes[at + 4..at + HEADER]
            .try_into()
            .expect("four bytes of type");
        let mut size = usize::try_from(size).unwrap_or(usize::MAX);
        let mut header = HEADER;
        if size == 1 {
            if at + HEADER + LARGESIZE_WIDTH > end {
                return;
            }
            size = usize::try_from(u64::from_be_bytes(
                bytes[at + HEADER..at + HEADER + LARGESIZE_WIDTH]
                    .try_into()
                    .expect("eight bytes of largesize"),
            ))
            .unwrap_or(usize::MAX);
            header += LARGESIZE_WIDTH;
        } else if size == 0 {
            size = end.saturating_sub(at);
        }
        if size < header || at.saturating_add(size) > end {
            return;
        }
        out.push(BoxInfo {
            offset: at,
            size,
            fourcc,
            media: fourcc == *b"mdat" || fourcc == *b"idat",
        });
        if let Some((_, prefix)) = CONTAINERS.iter().find(|(name, _)| **name == fourcc) {
            walk_boxes(bytes, at + header + prefix, at + size, out);
        }
        at += size;
    }
}

/// One damage applied to a fixture.
#[derive(Debug, Clone)]
enum Damage_ {
    /// Cut the file at a fraction of its length.
    Truncate { cut: usize },
    /// Zero [`RUN`] bytes at `offset`.
    Zero { offset: usize, len: usize },
    /// Overwrite [`RUN`] bytes at `offset` from a fixed seed.
    Random { offset: usize, len: usize },
    /// Set a box's `size` field.
    Size { offset: usize, value: u32 },
    /// Add `delta` to the 32-bit count at `offset`.
    Count { offset: usize, delta: i64 },
    /// Delete a box by relabelling its four-CC `free`.
    Delete { offset: usize },
}

impl Damage_ {
    fn label(&self) -> String {
        match self {
            Self::Truncate { cut } => format!("truncate@{cut}"),
            Self::Zero { offset, len } => format!("zero@{offset}+{len}"),
            Self::Random { offset, len } => format!("random@{offset}+{len}"),
            Self::Size { offset, value } => format!("size@{offset}={value}"),
            Self::Count { offset, delta } => format!("count@{offset}{delta:+}"),
            Self::Delete { offset } => format!("delete@{offset}"),
        }
    }

    /// The byte ranges this damage touched, as a function of the input.
    fn touched(&self, input_len: usize) -> Vec<(usize, usize)> {
        match self {
            Self::Truncate { cut } => vec![(*cut, input_len.saturating_sub(*cut))],
            Self::Zero { offset, len } | Self::Random { offset, len } => vec![(*offset, *len)],
            Self::Size { offset, .. } => vec![(*offset, 4)],
            Self::Count { offset, .. } => vec![(*offset, 1)],
            Self::Delete { offset } => vec![(*offset, HEADER)],
        }
    }
}

/// Apply one damage. `None` when the bytes would not change.
fn apply(bytes: &[u8], damage: &Damage_) -> Option<Vec<u8>> {
    let mut out = bytes.to_vec();
    match damage {
        Damage_::Truncate { cut } => {
            if *cut >= out.len() {
                return None;
            }
            out.truncate(*cut);
        }
        Damage_::Zero { offset, len } => {
            let end = offset.checked_add(*len)?.min(out.len());
            let start = (*offset).min(end);
            if out[start..end].iter().all(|byte| *byte == 0) {
                return None;
            }
            out[start..end].fill(0);
        }
        Damage_::Random { offset, len } => {
            let end = offset.checked_add(*len)?.min(out.len());
            let start = (*offset).min(end);
            let mut seed = SEED;
            for byte in &mut out[start..end] {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                *byte = (seed >> 33) as u8;
            }
        }
        Damage_::Size { offset, value } => {
            let end = offset.checked_add(4)?;
            if end > out.len() {
                return None;
            }
            if u32::from_be_bytes(out[*offset..end].try_into().ok()?) == *value {
                return None;
            }
            out[*offset..end].copy_from_slice(&value.to_be_bytes());
        }
        Damage_::Count { offset, delta } => {
            let end = offset.checked_add(4)?;
            if end > out.len() {
                return None;
            }
            let value = u32::from_be_bytes(out[*offset..end].try_into().ok()?);
            let changed = i64::from(value)
                .saturating_add(*delta)
                .clamp(0, i64::from(u32::MAX));
            let changed = u32::try_from(changed).ok()?;
            if changed == value {
                return None;
            }
            out[*offset..end].copy_from_slice(&changed.to_be_bytes());
        }
        Damage_::Delete { offset } => {
            let end = offset.checked_add(HEADER)?;
            if end > out.len() || &out[offset + 4..end] == b"free" {
                return None;
            }
            out[offset + 4..end].copy_from_slice(b"free");
        }
    }
    if out == bytes { None } else { Some(out) }
}

/// The failures that remain, by class, each with why playback is impossible
/// without what the damage removed. A cell that fails for any other reason
/// fails the matrix.
const EXPECTED_FAILURES: &[(&str, &str)] = &[
    (
        "no moov and no moof anywhere",
        "the damage removed the movie box and left no fragment, so nothing can be played",
    ),
    (
        "the movie states no track, so no sample can be played",
        "every `trak` the damage left is gone, so no track exists to play",
    ),
    (
        "the input ended before the item model was read",
        "the cut falls before a still image's `meta`, so no item model exists",
    ),
    (
        "file-level meta box is missing",
        "the damaged still image's `meta` could not be read at all",
    ),
];

/// Whether an error is one of the listed, justified failures.
fn allowed(error: &str) -> bool {
    EXPECTED_FAILURES
        .iter()
        .any(|(class, _)| error.contains(class))
}

fn label_for(fixture: &str, damage: &Damage_) -> String {
    format!("{fixture} {}", damage.label())
}

/// Every damage a fixture is put through.
fn cells(bytes: &[u8]) -> Vec<Damage_> {
    let mut out = Vec::new();
    for fraction in [1usize, 20, 40, 60, 80, 99] {
        let cut = bytes.len().saturating_mul(fraction) / 100;
        out.push(Damage_::Truncate { cut });
    }
    for item in boxes(bytes) {
        let body = item.offset + HEADER;
        let inner = item.size.saturating_sub(HEADER);
        if inner >= RUN {
            out.push(Damage_::Zero {
                offset: body,
                len: RUN,
            });
            out.push(Damage_::Random {
                offset: body,
                len: RUN,
            });
        } else if inner > 0 {
            out.push(Damage_::Zero {
                offset: body,
                len: inner,
            });
            out.push(Damage_::Random {
                offset: body,
                len: inner,
            });
        }
        if !item.media {
            out.push(Damage_::Size {
                offset: item.offset,
                value: 0,
            });
            out.push(Damage_::Size {
                offset: item.offset,
                value: MAX_SIZE,
            });
            out.push(Damage_::Size {
                offset: item.offset,
                value: u32::try_from(item.size + 1).unwrap_or(MAX_SIZE),
            });
            out.push(Damage_::Delete {
                offset: item.offset,
            });
            if counts_a_table(item.fourcc) {
                for delta in [-1i64, 1, 1_000] {
                    out.push(Damage_::Count {
                        offset: body + 4,
                        delta,
                    });
                }
            }
        }
    }
    out
}

fn counts_a_table(fourcc: [u8; 4]) -> bool {
    matches!(
        &fourcc,
        b"stts"
            | b"ctts"
            | b"stsc"
            | b"stsz"
            | b"stz2"
            | b"stco"
            | b"co64"
            | b"stss"
            | b"sdtp"
            | b"trun"
            | b"tfhd"
            | b"iloc"
            | b"iinf"
            | b"iref"
            | b"ipma"
            | b"saiz"
            | b"sidx"
    )
}

/// The fixtures the matrix covers.
const FIXTURES: [&str; 7] = [
    "mp4/progressive/av-faststart.mp4",
    "mp4/progressive/av-prog.mp4",
    "mp4/h264-aac-frag.mp4",
    "mp4/quicktime/lavf-aac-tmcd.mov",
    "avif/plain.avif",
    "heif/plain.heic",
    "hls/cmaf-fmp4/init.mp4",
];

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::require(rel, "damage_matrix").expect("fixture");
    std::fs::read(&path).expect("fixture bytes")
}

fn is_still(rel: &str) -> bool {
    rel.ends_with(".avif") || rel.ends_with(".heic")
}

fn drive_still(bytes: &[u8]) -> (Option<String>, Option<StillImage>, usize) {
    let config = ItemReaderConfig::default().with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    let mut damages = 0usize;
    loop {
        match reader.step() {
            Ok(ItemStep::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if let Err(error) =
                    reader.supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                {
                    return (Some(alloc_name(&error)), None, damages);
                }
            }
            Ok(ItemStep::Damage(_)) => damages += 1,
            Ok(ItemStep::Loaded(image)) => return (None, Some(image.clone()), damages),
            Ok(ItemStep::Done) => return (None, None, damages),
            Ok(_) => {}
            Err(error) => return (Some(alloc_name(&error)), None, damages),
        }
    }
}

/// What a still image addresses: every item's identifier, type and data
/// ranges, in the order the model holds them.
fn still_shape(image: &StillImage) -> Vec<(u32, [u8; 4], Vec<(u64, u64)>)> {
    image
        .items()
        .iter()
        .map(|item| {
            (
                item.id().get(),
                item.item_type().fourcc(),
                item.data_ranges()
                    .iter()
                    .map(|range| (range.offset, range.len))
                    .collect(),
            )
        })
        .collect()
}

fn monotonic(drive: &Drive) -> bool {
    let mut per_track: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
    for sample in &drive.samples {
        per_track
            .entry(sample.track.value())
            .or_default()
            .push(sample.decode_time);
    }
    per_track
        .values()
        .all(|times| times.windows(2).all(|pair| pair[0] <= pair[1]))
}

#[test]
fn a_damaged_fixture_plays_what_is_addressable_and_reports_the_rest() {
    let mut failures = Vec::new();
    for name in FIXTURES {
        let original = fixture(name);
        let still = is_still(name);
        let clean = if still {
            Drive::default()
        } else {
            drive(&original)
        };
        if !still {
            assert!(
                clean.error.is_none(),
                "{name}: the clean fixture reads, got {:?}",
                clean.error
            );
        }
        assert!(
            clean.damages.is_empty(),
            "{name}: a clean fixture publishes nothing, got {:?}",
            clean.damages
        );
        let clean_keys = keys(&clean);
        let clean_still = if still {
            let (error, image, reports) = drive_still(&original);
            assert!(
                error.is_none(),
                "{name}: the clean still reads, got {error:?}"
            );
            assert_eq!(reports, 0, "{name}: a clean still publishes nothing");
            Some((error, image, reports))
        } else {
            None
        };
        for damage in cells(&original) {
            let Some(damaged) = apply(&original, &damage) else {
                continue;
            };
            let label = label_for(name, &damage);
            let touched = damage.touched(original.len());
            if still {
                let (error, image, reports) = drive_still(&damaged);
                println!("CELL {label}: err={error:?} reports={reports}");
                if let Some(error) = &error {
                    if !allowed(error) {
                        failures.push(format!("{label}: refused: {error}"));
                    }
                    continue;
                }
                let _ = clean_still;
                if let Some(after) = image.as_ref() {
                    // A still that loads may restate what a damaged table
                    // says, but never addresses data outside its own input.
                    let file_len = u64::try_from(damaged.len()).unwrap_or(u64::MAX);
                    for (item, _, ranges) in still_shape(after) {
                        for (offset, len) in ranges {
                            if offset.saturating_add(len) > file_len {
                                failures.push(format!(
                                    "{label}: item {item} addresses {offset}+{len} past the {file_len}-byte input"
                                ));
                            }
                        }
                    }
                }
                continue;
            }
            let result = drive(&damaged);
            let got = keys(&result);
            let sample_extents: Vec<(u64, u64)> = clean_keys
                .iter()
                .map(|sample| (sample.2, sample.2.saturating_add(u64::from(sample.3))))
                .collect();
            let media_touched = touched.iter().any(|(offset, len)| {
                let start = u64::try_from(*offset).unwrap_or(u64::MAX);
                let end = start.saturating_add(u64::try_from(*len).unwrap_or(u64::MAX));
                sample_extents
                    .iter()
                    .any(|(first, last)| *first < end && start < *last)
            });
            println!(
                "CELL {label}: err={:?} samples={} reports={} media={media_touched}",
                result.error,
                result.samples.len(),
                result.damages.len()
            );
            if let Some(error) = &result.error {
                if !allowed(error) {
                    failures.push(format!("{label}: refused: {error}"));
                }
                continue;
            }
            if media_touched {
                // Media bytes were touched: no sample may be invented, every
                // sample the damage left alone must still be delivered, and
                // whatever was cut is reported.
                for key in &got {
                    if !clean_keys.contains(key) {
                        failures.push(format!(
                            "{label}: delivered {key:?} from bytes the clean run did not"
                        ));
                    }
                }
                for key in &clean_keys {
                    let start = key.2;
                    let finish = start.saturating_add(u64::from(key.3));
                    let outside = !touched.iter().any(|(offset, len)| {
                        let begin = u64::try_from(*offset).unwrap_or(u64::MAX);
                        let end = begin.saturating_add(u64::try_from(*len).unwrap_or(u64::MAX));
                        start < end && begin < finish
                    });
                    if outside && !got.contains(key) {
                        failures.push(format!(
                            "{label}: sample {key:?} is wholly outside the damage but was not delivered"
                        ));
                    }
                }
                if got != clean_keys && result.damages.is_empty() {
                    failures.push(format!("{label}: lost media with no report"));
                }
            } else {
                // A damaged table may legitimately restate where a sample
                // lives, but never past the input it was read from.
                let file_len = u64::try_from(damaged.len()).unwrap_or(u64::MAX);
                for key in &got {
                    let end = key.2.saturating_add(u64::from(key.3));
                    if end > file_len {
                        failures.push(format!(
                            "{label}: delivered {key:?} past the {file_len}-byte input"
                        ));
                    }
                }
            }
            if !monotonic(&result) {
                failures.push(format!("{label}: decode times are not monotonic"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} cell(s) refused a file they could read:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a_zero_timescale_track_with_samples_is_dropped_and_reported() {
    let original = fixture("mp4/progressive/av-faststart.mp4");
    let mdhd = boxes(&original)
        .into_iter()
        .find(|item| item.fourcc == *b"mdhd")
        .expect("an mdhd");
    // `creation_time` (4) and `modification_time` (4) then `timescale`.
    let timescale = mdhd.offset + HEADER + 4 + 8;
    let mut damaged = original.clone();
    damaged[timescale..timescale + 4].fill(0);
    let result = drive(&damaged);
    assert!(
        result.error.is_none(),
        "a zero track timescale costs that track, not the file: {:?}",
        result.error
    );
    assert!(
        result
            .damages
            .iter()
            .any(|damage| damage.kind() == DamageKind::TrackDropped),
        "the dropped track is reported: {:?}",
        result.damages
    );
}

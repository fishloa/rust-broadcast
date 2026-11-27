//! Every committed MP4-family fixture reads to the same typed results it did
//! before this lane: the same files read, the same tracks and sample ranges,
//! the same items and properties.
//!
//! The pinned totals and digest come from `origin/main`; a change here is a
//! change in what a real file reads to, not a change in how it is written.

use std::path::{Path, PathBuf};

use isobmff::Error;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep};
use isobmff::reader::{Mode, Reader, ReaderConfig, Source, Step};
use test_fixtures as fixtures;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A stable hash of the typed results, independent of `std` hashing.
struct Hash(u64);

impl Hash {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }
    fn bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }
    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_be_bytes());
    }
    fn text(&mut self, value: &str) {
        self.u64(u64::try_from(value.len()).unwrap_or(u64::MAX));
        self.bytes(value.as_bytes());
    }
}

fn movie(bytes: &[u8]) -> Result<(usize, usize, u64), Error> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_source(Source::Seekable)
        .with_mode(Mode::Permissive);
    let mut reader = Reader::new(config);
    let mut hash = Hash::new();
    let mut tracks = 0usize;
    let mut samples = 0usize;
    loop {
        match reader.step()? {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader.supply(range.offset, &bytes[start..start + len])?;
            }
            Step::Tracks(list) => {
                tracks = list.len();
                for info in list {
                    hash.u64(u64::from(info.track_id.value()));
                    hash.bytes(&info.handler);
                    hash.u64(u64::from(info.timescale.get()));
                    hash.u64(info.duration);
                    hash.u64(u64::try_from(info.entries.len()).unwrap_or(u64::MAX));
                    for entry in &info.entries {
                        hash.bytes(&entry.fourcc);
                        hash.u64(u64::try_from(entry.body.len()).unwrap_or(u64::MAX));
                        hash.bytes(&entry.body);
                    }
                }
            }
            Step::Sample(sample) => {
                samples += 1;
                hash.u64(u64::from(sample.track.value()));
                hash.u64(u64::from(sample.index));
                hash.u64(sample.decode_time);
                hash.u64(sample.duration.into());
                hash.u64(sample.offset);
                hash.u64(u64::from(sample.size));
                hash.u64(u64::from(sample.description_index));
            }
            Step::Done => break,
            _ => {}
        }
    }
    Ok((tracks, samples, hash.0))
}

fn item(bytes: &[u8]) -> Result<(usize, usize, u64), Error> {
    let config = ItemReaderConfig::default().with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    let mut model = None;
    loop {
        match reader.step()? {
            ItemStep::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader.supply(range.offset, &bytes[start..start + len])?;
            }
            ItemStep::Loaded(image) => model = Some(image.clone()),
            ItemStep::Done => break,
            _ => {}
        }
    }
    let image = model.ok_or(Error::InvalidInput("no item model"))?;
    let mut hash = Hash::new();
    let mut properties = 0usize;
    for item in image.items() {
        hash.u64(u64::from(item.id().get()));
        hash.bytes(&item.item_type().fourcc());
        hash.bytes(item.item_name());
        hash.u64(u64::try_from(item.data_ranges().len()).unwrap_or(u64::MAX));
        for range in item.data_ranges() {
            hash.u64(range.offset);
            hash.u64(range.len);
        }
        properties += item.properties().len();
        for property in item.properties() {
            hash.bytes(&property.property().fourcc());
            hash.u64(u64::from(property.essential()));
        }
        for reference in item.references_out() {
            hash.text(reference.reference_type().name());
            hash.u64(u64::from(reference.to().get()));
        }
    }
    Ok((image.items().len(), properties, hash.0))
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

/// A fixture's name relative to its fixture root, so two checkouts of the same
/// tree hash the same label.
fn label(path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(fixtures::root()) {
        return format!("public/{}", relative.display());
    }
    if let Some(private) = fixtures::private("")
        && let Ok(relative) = path.strip_prefix(&private)
    {
        return format!("private/{}", relative.display());
    }
    path.display().to_string()
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

/// Files, movie reads, item reads, track and sample totals, item and property
/// totals, and one digest over every typed result.
#[derive(Debug, Default, PartialEq, Eq)]
struct Totals {
    files: usize,
    movie_reads: usize,
    item_reads: usize,
    tracks: usize,
    samples: usize,
    items: usize,
    properties: usize,
    digest: u64,
}

fn totals() -> Totals {
    let mut total = Totals {
        digest: FNV_OFFSET,
        ..Totals::default()
    };
    let mut hash = Hash::new();
    for path in committed_media() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        total.files += 1;
        hash.text(&label(&path));
        match movie(&bytes) {
            Ok((tracks, samples, digest)) => {
                total.movie_reads += 1;
                total.tracks += tracks;
                total.samples += samples;
                hash.u64(1);
                hash.u64(digest);
            }
            Err(_) => hash.u64(0),
        }
        match item(&bytes) {
            Ok((items, properties, digest)) => {
                total.item_reads += 1;
                total.items += items;
                total.properties += properties;
                hash.u64(1);
                hash.u64(digest);
            }
            Err(_) => hash.u64(0),
        }
    }
    total.digest = hash.0;
    total
}

/// The totals `origin/main` reads from the committed fixtures.
///
/// `origin/main`'s own tree reads these 282 files to exactly these results.
/// The eleven `.mov` fixtures `#4204` added (`quicktime/qt-h263.mov`,
/// `qt-mjpeg.mov`, `qt-pcm-s16le.mov`, `qt-pcm-s24le.mov`, the six
/// `qt-prores-ap*.mov` and `qt-svq1.mov`) are the whole difference from the
/// 271-file totals this lane pinned before that merge: each file's own typed
/// results were compared one by one against `origin/main` and every one of
/// the 271 older files is unchanged. (`mp4/3gpp/h263.3gp` is not in the
/// walked extensions.)
const EXPECTED: Totals = Totals {
    files: 282,
    movie_reads: 282,
    item_reads: 20,
    tracks: 222,
    samples: 259_078,
    items: 50,
    properties: 188,
    digest: 16_550_637_486_676_025_805,
};

#[test]
fn every_committed_fixture_reads_to_the_same_typed_results() {
    let total = totals();
    assert_eq!(
        total, EXPECTED,
        "a committed fixture now reads to different typed results"
    );
}

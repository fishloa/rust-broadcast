//! The progressive reader over a multi-gigabyte sparse MP4: it must reach a
//! trailing `moov` by header skips alone, ask for no `mdat` payload byte, and
//! hold no more than the parsed `moov`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use broadcast_common::Parse;
use isobmff::{ByteRange, Reader, ReaderConfig, SampleOrder, Step};

#[global_allocator]
static PEAK: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// Ceiling on bytes requested beyond the `moov` itself: three top-level
/// headers (`ftyp`, `moov`, `mdat`) plus the `moov` header re-read with its
/// body, at the largest header size (32 bytes each).
#[cfg(feature = "scan-counters")]
const HEADER_ALLOWANCE: u64 = 256;

/// Peak live bytes allowed per byte of `moov`: the supplied `moov` buffer, the
/// parsed box tree (which copies names, sample entries and tables), and the
/// raw sample-entry bodies kept per track.
const PEAK_PER_MOOV: usize = 8;

/// Fixed peak allowance above `moov * PEAK_PER_MOOV` for the reader's own
/// structures and the test harness.
const PEAK_SLACK: usize = 4 << 20;

/// Serves a `Need` by seeking and reading exactly that range, so a sparse
/// multi-gigabyte file costs only its headers.
struct FileSource {
    file: File,
    length: u64,
}

impl FileSource {
    fn supply(&mut self, reader: &mut Reader, offset: u64, len: u64) -> isobmff::Result<()> {
        let end = offset.saturating_add(len).min(self.length);
        let count = usize::try_from(end.saturating_sub(offset)).unwrap_or(usize::MAX);
        let mut buffer = vec![0u8; count];
        self.file.seek(SeekFrom::Start(offset)).expect("seek range");
        self.file.read_exact(&mut buffer).expect("read range");
        reader.supply(offset, &buffer)
    }
}

fn drive(path: &std::path::Path, order: SampleOrder) -> (u64, u64, Vec<ByteRange>) {
    let file = File::open(path).expect("open generated mp4");
    let length = file.metadata().expect("metadata").len();
    let mut source = FileSource { file, length };
    let config = ReaderConfig::default()
        .with_total_len(Some(length))
        .with_order(order);
    let mut reader = Reader::new(config);
    let mut samples = 0u64;
    let mut requests = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                requests.push(range);
                source
                    .supply(&mut reader, range.offset, range.len)
                    .expect("supply");
            }
            Step::Tracks(tracks) => assert_eq!(tracks.len(), 2),
            Step::Sample(_) => samples += 1,
            Step::Done => break,
            _ => break,
        }
    }
    #[cfg(feature = "scan-counters")]
    let requested = reader.bytes_requested();
    #[cfg(not(feature = "scan-counters"))]
    let requested = 0u64;
    (samples, requested, requests)
}

/// `(payload start, box end)` of the file's `mdat`, found from its header.
fn mdat_payload(path: &std::path::Path) -> (u64, u64) {
    let mut file = File::open(path).expect("open generated mp4");
    let length = file.metadata().expect("metadata").len();
    let mut offset = 0u64;
    while offset.saturating_add(8) <= length {
        let mut head = [0u8; 32];
        file.seek(SeekFrom::Start(offset)).expect("seek header");
        let read = file.read(&mut head).expect("read header");
        let header = isobmff::BoxHeader::parse(&head[..read]).expect("header");
        let size = if header.size == 0 {
            length - offset
        } else {
            header.size
        };
        if header.box_type.is(b"mdat") {
            let payload = offset.saturating_add(header.header_size() as u64);
            return (payload, offset.saturating_add(size));
        }
        offset = offset.saturating_add(size);
    }
    panic!("generated file has no mdat");
}

#[test]
fn a_multi_gigabyte_mp4_costs_only_its_headers() {
    let base = test_harness::big_media::BigMp4Options::default();
    for moov_first in [true, false] {
        let path = std::env::temp_dir().join(format!(
            "isobmff-reader-big-{}-{}.mp4",
            std::process::id(),
            if moov_first { "first" } else { "last" }
        ));
        let mut options = base.clone();
        options.moov_first = moov_first;
        let written = test_harness::big_media::write_big_mp4(&path, &options);
        PEAK.reset();
        let (peak, (samples, requested, requests)) =
            PEAK.peak_during(|| drive(&path, SampleOrder::DecodeTime));
        let (payload_start, mdat_end) = mdat_payload(&path);
        std::fs::remove_file(&path).expect("remove generated file");
        assert_eq!(samples, u64::from(base.sample_count) * 2);
        let budget =
            usize::try_from(written.moov_bytes).unwrap_or(usize::MAX) * PEAK_PER_MOOV + PEAK_SLACK;
        assert!(
            peak < budget,
            "moov_first={moov_first}: peak {peak} bytes, budget {budget} (moov {})",
            written.moov_bytes
        );
        #[cfg(feature = "scan-counters")]
        assert!(
            requested < written.moov_bytes + HEADER_ALLOWANCE,
            "moov_first={moov_first}: requested {requested}, moov {}",
            written.moov_bytes
        );
        #[cfg(not(feature = "scan-counters"))]
        let _ = requested;
        for range in requests {
            assert!(
                range.end() <= payload_start || range.offset >= mdat_end,
                "request {range:?} overlaps the mdat payload [{payload_start}, {mdat_end})"
            );
        }
    }
}

/// Peak live bytes allowed beyond the parsed `moov`, independent of how many
/// fragments the file holds: one `moof`, one sample descriptor per fragment
/// sample in flight, and the test harness.
const FRAGMENTED_PEAK_SLACK: usize = 8 << 20;

/// Bytes the reader may request per fragment beyond its `moof` and one box
/// header: the `mdat` header plus a `styp`/`mfra` allowance.
const FRAGMENTED_HEADER_ALLOWANCE: u64 = 64;

/// Drive a fragmented file, returning `(samples, fragments, requested bytes,
/// requests)`.
fn drive_fragments(path: &std::path::Path) -> (u64, u64, u64, Vec<ByteRange>) {
    let file = File::open(path).expect("open generated fragmented mp4");
    let length = file.metadata().expect("metadata").len();
    let mut source = FileSource { file, length };
    let config = ReaderConfig::default()
        .with_total_len(Some(length))
        .with_order(SampleOrder::DecodeTime);
    let mut reader = Reader::new(config);
    let mut samples = 0u64;
    let mut fragments = 0u64;
    let mut requests = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                requests.push(range);
                source
                    .supply(&mut reader, range.offset, range.len)
                    .expect("supply");
            }
            Step::Tracks(tracks) => assert_eq!(tracks.len(), 1),
            Step::Fragment(_) => fragments += 1,
            Step::Sample(_) => samples += 1,
            Step::Done => break,
            _ => break,
        }
    }
    #[cfg(feature = "scan-counters")]
    let requested = reader.bytes_requested();
    #[cfg(not(feature = "scan-counters"))]
    let requested = 0u64;
    (samples, fragments, requested, requests)
}

#[test]
fn a_multi_gigabyte_fragmented_mp4_holds_one_moof() {
    let options = test_harness::big_media::BigFmp4Options::default();
    let path = std::env::temp_dir().join(format!(
        "isobmff-reader-big-frag-{}.mp4",
        std::process::id()
    ));
    let written = test_harness::big_media::write_big_fmp4(&path, &options);
    PEAK.reset();
    let (peak, (samples, fragments, requested, requests)) =
        PEAK.peak_during(|| drive_fragments(&path));
    std::fs::remove_file(&path).expect("remove generated file");

    assert_eq!(samples, u64::from(written.fragment_count));
    assert_eq!(fragments, u64::from(written.fragment_count));
    let budget = usize::try_from(written.moov_bytes).unwrap_or(usize::MAX) * PEAK_PER_MOOV
        + FRAGMENTED_PEAK_SLACK;
    assert!(
        peak < budget,
        "peak {peak} bytes, budget {budget} (moov {}, {} fragments)",
        written.moov_bytes,
        written.fragment_count
    );
    #[cfg(feature = "scan-counters")]
    assert!(
        requested
            < written.moov_bytes
                + u64::from(written.fragment_count)
                    * (written.moof_bytes + FRAGMENTED_HEADER_ALLOWANCE),
        "requested {requested} bytes for {} fragments",
        written.fragment_count
    );
    #[cfg(not(feature = "scan-counters"))]
    let _ = requested;
    for (index, offset) in written.moof_offsets.iter().enumerate() {
        let payload_start = offset + written.moof_bytes + 8;
        let payload_end = payload_start + u64::from(written.sample_bytes);
        for range in &requests {
            assert!(
                range.end() <= payload_start || range.offset >= payload_end,
                "fragment {index}: request {range:?} overlaps the mdat payload [{payload_start}, {payload_end})"
            );
        }
    }
}

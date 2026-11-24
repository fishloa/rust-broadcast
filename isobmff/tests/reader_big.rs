//! The progressive reader over a multi-gigabyte sparse MP4: it must reach a
//! trailing `moov` by header skips alone, ask for no `mdat` payload byte, and
//! hold no more than the parsed `moov`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use isobmff::{ReadStep, Reader, ReaderConfig, SampleOrder, Step};

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

fn drive(path: &std::path::Path, order: SampleOrder) -> (u64, u64) {
    let file = File::open(path).expect("open generated mp4");
    let length = file.metadata().expect("metadata").len();
    let mut source = FileSource { file, length };
    let config = ReaderConfig::default()
        .with_total_len(Some(length))
        .with_order(order);
    let mut reader = Reader::new(config);
    let mut samples = 0u64;
    loop {
        match reader.next().expect("reader step") {
            Step::Need(range) => source
                .supply(&mut reader, range.offset, range.len)
                .expect("supply"),
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
    (samples, requested)
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
        let (peak, (samples, requested)) =
            PEAK.peak_during(|| drive(&path, SampleOrder::DecodeTime));
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
    }
}

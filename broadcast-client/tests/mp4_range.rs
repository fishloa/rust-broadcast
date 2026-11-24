//! The sans-IO HTTP range driver: a moov-last file opens in a bounded number
//! of requests, coalescing is observable, and the refusals are typed.

use broadcast_client::mp4_range::{RangeAction, RangeError, RangeReader};
use http::StatusCode;
use isobmff::driver::DriverStep;
use isobmff::{ByteRange, ReaderConfig, SampleOrder};

/// Per-thread live-byte ceiling for the whole test: the fixture, one held
/// window, and the reader's parsed `moov`.
const BUDGET: usize = 8 << 20;

#[global_allocator]
static ALLOC: test_alloc::ThreadCapped<BUDGET> = test_alloc::ThreadCapped::new();

/// The caller's coalescing policy: a demand below this many bytes fetches this
/// many.
const MIN_FETCH: u64 = 64 * 1024;
/// One byte proves every demand is fetched at its own size.
const TINY_FETCH: u64 = 1;
/// Requests a moov-last open may cost at `MIN_FETCH`: the first window
/// (`ftyp`, a `free` box, the `mdat` header and its first bytes), then the
/// window at the `mdat` end covering the whole `moov` header and body, with one
/// allowance for a `moov` that crosses a window boundary. A multi-megabyte
/// `mdat` costs no more: its payload is skipped by declared size, never read.
const MAX_OPEN_REQUESTS: usize = 3;

/// A test-local range server: a function from `(offset, len)` to bytes over a
/// fixture. It reports the complete length the way `Content-Range` does.
struct Server<'a> {
    bytes: &'a [u8],
    requests: usize,
}

impl Server<'_> {
    fn get(&mut self, range: ByteRange) -> (Vec<u8>, Option<u64>) {
        self.requests += 1;
        let total = u64::try_from(self.bytes.len()).unwrap_or(u64::MAX);
        let start = usize::try_from(range.offset.min(total)).unwrap_or(usize::MAX);
        let end = usize::try_from(range.end().min(total)).unwrap_or(usize::MAX);
        (
            self.bytes.get(start..end).unwrap_or(&[]).to_vec(),
            Some(total),
        )
    }
}

/// Drive one open, returning `(tracks, requests, largest fetch)`.
fn open(bytes: &[u8], min_fetch: u64) -> (usize, usize, u64) {
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = RangeReader::new(config, min_fetch);
    let bound = reader.window_bound();
    let mut server = Server { bytes, requests: 0 };
    let mut tracks = 0usize;
    let mut largest = 0u64;
    loop {
        let fetch = match reader.poll().expect("poll") {
            RangeAction::Fetch { offset, len } => ByteRange { offset, len },
            RangeAction::Step(DriverStep::Tracks(info)) => {
                tracks = info.len();
                continue;
            }
            RangeAction::Step(_) => continue,
            RangeAction::Done => break,
            _ => continue,
        };
        assert!(
            fetch.len <= bound,
            "fetch {fetch:?} exceeds the window bound {bound}"
        );
        largest = largest.max(fetch.len);
        let (body, total) = server.get(fetch);
        reader
            .on_status(StatusCode::PARTIAL_CONTENT)
            .expect("partial content");
        reader
            .on_range(fetch.offset, &body, total)
            .expect("range body");
    }
    (tracks, server.requests, largest)
}

#[test]
fn a_moov_last_file_opens_in_a_bounded_number_of_requests() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let bytes = std::fs::read(&path).expect("read fixture");
    let (tracks, requests, largest) = open(&bytes, MIN_FETCH);
    assert_eq!(tracks, 2);
    assert!(
        requests <= MAX_OPEN_REQUESTS,
        "{requests} requests, expected at most {MAX_OPEN_REQUESTS}"
    );
    assert!(largest > 0);
}

#[test]
fn a_multi_megabyte_mdat_costs_a_bounded_number_of_requests() {
    let Some(bytes) = test_fixtures::require_bytes("mp4/camera/gopro-karma.mp4", "mp4_range")
    else {
        return;
    };
    assert!(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MIN_FETCH * 8,
        "the fixture must carry an mdat larger than one window"
    );
    let (tracks, requests, largest) = open(&bytes, MIN_FETCH);
    assert_eq!(tracks, 5);
    assert!(
        requests <= MAX_OPEN_REQUESTS,
        "{requests} requests over a {}-byte file, expected at most {MAX_OPEN_REQUESTS}",
        bytes.len()
    );
    assert!(largest > 0);
}

#[test]
fn a_tiny_min_fetch_makes_strictly_more_requests() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let bytes = std::fs::read(&path).expect("read fixture");
    let (coalesced_tracks, coalesced, _) = open(&bytes, MIN_FETCH);
    let (tiny_tracks, tiny, _) = open(&bytes, TINY_FETCH);
    assert_eq!(coalesced_tracks, tiny_tracks);
    assert!(
        tiny > coalesced,
        "tiny_fetch={tiny}, coalesced={coalesced}: coalescing had no effect"
    );
}

#[test]
fn a_server_that_ignores_range_is_a_typed_error() {
    let config = ReaderConfig::default();
    let mut reader = RangeReader::new(config, MIN_FETCH);
    match reader.poll().expect("first fetch") {
        RangeAction::Fetch { .. } => {}
        other => panic!("expected a fetch, got {other:?}"),
    }
    let error = reader
        .on_status(StatusCode::OK)
        .expect_err("200 is not a range answer");
    assert!(matches!(error, RangeError::RangeNotSupported));
}

#[test]
fn a_416_is_a_typed_error() {
    let config = ReaderConfig::default();
    let mut reader = RangeReader::new(config, MIN_FETCH);
    match reader.poll().expect("first fetch") {
        RangeAction::Fetch { .. } => {}
        other => panic!("expected a fetch, got {other:?}"),
    }
    let error = reader
        .on_status(StatusCode::RANGE_NOT_SATISFIABLE)
        .expect_err("416 is unsatisfiable");
    assert!(matches!(error, RangeError::RangeNotSatisfiable));
}

#[test]
fn a_response_at_the_wrong_offset_is_a_typed_error() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let bytes = std::fs::read(&path).expect("read fixture");
    let config = ReaderConfig::default();
    let mut reader = RangeReader::new(config, MIN_FETCH);
    let fetch = match reader.poll().expect("first fetch") {
        RangeAction::Fetch { offset, len } => ByteRange { offset, len },
        other => panic!("expected a fetch, got {other:?}"),
    };
    reader
        .on_status(StatusCode::PARTIAL_CONTENT)
        .expect("partial content");
    let error = reader
        .on_range(
            fetch.offset.saturating_add(1),
            &bytes[..16],
            Some(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
        )
        .expect_err("wrong offset");
    assert!(matches!(error, RangeError::UnexpectedRange { .. }));
}

#[test]
fn the_window_never_exceeds_the_stated_bound() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let bytes = std::fs::read(&path).expect("read fixture");
    let config = ReaderConfig::default();
    let mut reader = RangeReader::new(config, MIN_FETCH);
    let bound = reader.window_bound();
    let mut server = Server {
        bytes: &bytes,
        requests: 0,
    };
    loop {
        let fetch = match reader.poll().expect("poll") {
            RangeAction::Fetch { offset, len } => ByteRange { offset, len },
            RangeAction::Step(_) => continue,
            RangeAction::Done => break,
            _ => continue,
        };
        assert!(fetch.len <= bound);
        let (body, total) = server.get(fetch);
        reader
            .on_status(StatusCode::PARTIAL_CONTENT)
            .expect("partial content");
        reader
            .on_range(fetch.offset, &body, total)
            .expect("range body");
    }
    assert!(bound >= MIN_FETCH);
}

#[test]
fn a_range_driver_reports_the_movie_metadata() {
    let Some(bytes) = test_fixtures::require_bytes("mp4/tags/lavf-tagged.m4a", "mp4_range") else {
        return;
    };
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = RangeReader::new(config, MIN_FETCH);
    let mut server = Server {
        bytes: &bytes,
        requests: 0,
    };
    let mut title = None;
    loop {
        let fetch = match reader.poll().expect("poll") {
            RangeAction::Fetch { offset, len } => ByteRange { offset, len },
            RangeAction::Step(DriverStep::Metadata(metadata)) => {
                title = metadata
                    .movie
                    .text(&isobmff::metadata::ItemKey::TITLE)
                    .map(str::to_string);
                continue;
            }
            RangeAction::Step(_) => continue,
            RangeAction::Done => break,
            _ => continue,
        };
        let (body, total) = server.get(fetch);
        reader
            .on_status(StatusCode::PARTIAL_CONTENT)
            .expect("partial content");
        reader
            .on_range(fetch.offset, &body, total)
            .expect("range body");
    }
    assert_eq!(title.as_deref(), Some("Test Tone"));
}

#[test]
fn the_range_driver_passes_a_warning_through() {
    use isobmff::reader::{Mode, Warning};
    let path = test_fixtures::path("mp4/progressive/av-faststart.mp4");
    let mut bytes = std::fs::read(&path).expect("read fixture");
    let mvhd = bytes.windows(4).position(|w| w == b"mvhd").expect("mvhd") - 4;
    let timescale = mvhd + if bytes[mvhd + 8] == 1 { 28 } else { 20 };
    bytes[timescale..timescale + 4].fill(0);
    let config = ReaderConfig::default()
        .with_order(SampleOrder::File)
        .with_mode(Mode::Permissive);
    let mut reader = RangeReader::new(config, MIN_FETCH);
    let mut server = Server {
        bytes: &bytes,
        requests: 0,
    };
    loop {
        match reader.poll().expect("poll") {
            RangeAction::Fetch { offset, len } => {
                let fetch = ByteRange { offset, len };
                let (body, total) = server.get(fetch);
                reader
                    .on_status(StatusCode::PARTIAL_CONTENT)
                    .expect("partial content");
                reader
                    .on_range(fetch.offset, &body, total)
                    .expect("range body");
            }
            RangeAction::Step(DriverStep::Warning(Warning::ZeroMovieTimescale { .. })) => return,
            RangeAction::Done => panic!("no warning"),
            RangeAction::Step(_) => {}
            _ => {}
        }
    }
}

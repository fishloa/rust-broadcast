#![no_main]

use isobmff::{Limits, Mode, Reader, ReaderConfig, SampleOrder, Source, Step};
use libfuzzer_sys::fuzz_target;

/// Caps small enough that a hostile table cannot allocate its way out of the
/// fuzzer's own budget.
fn fuzz_limits() -> Limits {
    Limits::default()
        .with_max_depth(8)
        .with_max_moov_bytes(1 << 20)
        .with_max_samples_per_track(1 << 16)
        .with_max_sample_bytes(1 << 20)
        .with_max_fragment_bytes(1 << 20)
        .with_max_metadata_item_bytes(1 << 20)
}

/// A hostile file cannot describe more steps than this.
const MAX_STEPS: usize = 1 << 18;

/// A seek target derived from the input, so both the `sidx` and the trailing
/// `mfra` paths (and their failure modes) are reachable.
fn seek_target(data: &[u8]) -> u64 {
    let seed = u64::from(data.first().copied().unwrap_or(0));
    seed * 1_000
}

fn drive(
    data: &[u8],
    mode: Mode,
    order: SampleOrder,
    source: Source,
    total_len: Option<u64>,
    seek: bool,
) {
    let config = ReaderConfig::default()
        .with_total_len(total_len)
        .with_order(order)
        .with_source(source)
        .with_mode(mode)
        .with_limits(fuzz_limits());
    let mut reader = Reader::new(config);
    let mut sought = false;
    for _ in 0..MAX_STEPS {
        let mut seek_track = None;
        match reader.step() {
            Ok(Step::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if reader
                    .supply(range.offset, data.get(start..end).unwrap_or(&[]))
                    .is_err()
                {
                    break;
                }
            }
            Ok(Step::Tracks(tracks)) => {
                if seek && !sought {
                    seek_track = tracks.first().map(|info| info.track_id);
                }
            }
            Ok(Step::Done) | Err(_) => break,
            Ok(_) => {}
        }
        if let Some(track) = seek_track {
            sought = true;
            let _ = reader.seek_to(track, seek_target(data));
            if reader.seek_to(track, seek_target(data)).is_err() {
                break;
            }
        }
    }
}

fuzz_target!(|input: &[u8]| {
    let Some((mode_byte, data)) = input.split_first() else {
        return;
    };
    let mode = if mode_byte & 1 == 0 {
        Mode::Strict
    } else {
        Mode::Permissive
    };
    let exact = Some(data.len() as u64);
    drive(
        data,
        mode,
        SampleOrder::DecodeTime,
        Source::Seekable,
        exact,
        false,
    );
    drive(data, mode, SampleOrder::File, Source::Seekable, exact, true);
    drive(
        data,
        mode,
        SampleOrder::File,
        Source::ForwardOnly,
        exact,
        false,
    );
    drive(
        data,
        mode,
        SampleOrder::DecodeTime,
        Source::Seekable,
        None,
        true,
    );
});

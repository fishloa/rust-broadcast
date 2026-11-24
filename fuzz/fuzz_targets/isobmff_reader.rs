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
const MAX_STEPS: usize = 1 << 20;

fn drive(data: &[u8], mode: Mode, order: SampleOrder, source: Source) {
    let config = ReaderConfig::default()
        .with_total_len(Some(data.len() as u64))
        .with_order(order)
        .with_source(source)
        .with_mode(mode)
        .with_limits(fuzz_limits());
    let mut reader = Reader::new(config);
    for _ in 0..MAX_STEPS {
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
            Ok(Step::Done) | Err(_) => break,
            Ok(_) => {}
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
    drive(data, mode, SampleOrder::DecodeTime, Source::Seekable);
    drive(data, mode, SampleOrder::File, Source::Seekable);
    drive(data, mode, SampleOrder::File, Source::ForwardOnly);
});

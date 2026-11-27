#![no_main]

use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep};
use isobmff::Limits;
use libfuzzer_sys::fuzz_target;

fn fuzz_limits() -> Limits {
    Limits::default()
        .with_max_moov_bytes(1 << 20)
        .with_max_metadata_item_bytes(1 << 20)
        .with_max_items(1 << 12)
        .with_max_properties(1 << 12)
        .with_max_references(1 << 12)
}

const MAX_STEPS: usize = 1 << 16;

fuzz_target!(|data: &[u8]| {
    let config = ItemReaderConfig::default()
        .with_total_len(Some(data.len() as u64))
        .with_limits(fuzz_limits());
    let mut reader = ItemReader::new(config);
    for _ in 0..MAX_STEPS {
        match reader.step() {
            Ok(ItemStep::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if reader
                    .supply(range.offset, data.get(start..end).unwrap_or(&[]))
                    .is_err()
                {
                    break;
                }
            }
            Ok(ItemStep::Loaded(image)) => {
                // What the reader accepts, it re-emits byte for byte.
                let reemitted = image
                    .reemit_into(data)
                    .expect("a loaded image re-emits from its own source");
                assert_eq!(reemitted, data, "re-emitted bytes differ from the source");
            }
            Ok(ItemStep::Done) | Err(_) => break,
            Ok(_) => break,
        }
    }
});

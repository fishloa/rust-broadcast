#![no_main]

use broadcast_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = scte104::SingleOperationMessage::parse(data);
    let _ = scte104::MultipleOperationMessage::parse(data);
});

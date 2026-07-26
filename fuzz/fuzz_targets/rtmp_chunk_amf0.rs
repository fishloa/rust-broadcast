#![no_main]

use broadcast_common::Parse;
use libfuzzer_sys::fuzz_target;
use rtmp_runtime::amf0::Amf0Value;
use rtmp_runtime::chunk::ChunkAssembler;

// Two independent lower-layer parse entry points (#738), each fuzzed directly
// (not just via the top-level session, for better coverage of their own
// buffer-slicing logic): the chunk-stream reassembler (§5.3, basic header +
// 4 message-header formats + extended timestamp) and AMF0 value decoding
// (§8.2, used by command/data messages). Neither must panic on arbitrary
// bytes.
fuzz_target!(|data: &[u8]| {
    let mut assembler = ChunkAssembler::new();
    let _ = assembler.push(data);

    let _ = Amf0Value::parse(data);
});

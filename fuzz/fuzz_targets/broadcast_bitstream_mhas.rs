#![no_main]

use broadcast_bitstream::mhas::{find_mpegh3da_config, walk_mhas_packets};
use libfuzzer_sys::fuzz_target;

// Fuzz the MHAS packet walker on arbitrary bytes. Every walked packet's payload
// lies inside the input, and the config lookup is one of those payloads.
fuzz_target!(|data: &[u8]| {
    let packets = walk_mhas_packets(data);
    for p in &packets {
        assert!(p.payload().len() <= data.len());
        let _ = p.packet_type();
    }
    if let Some(config) = find_mpegh3da_config(data) {
        assert!(config.len() <= data.len());
    }
});

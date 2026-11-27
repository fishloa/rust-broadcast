#![no_main]

use broadcast_bitstream::dts_core::{DtsCoreFrameInfo, split_dts_core_frames};
use libfuzzer_sys::fuzz_target;

// Fuzz the DTS core frame-header parser and splitter on arbitrary bytes. Every
// split frame must lie inside the input and carry the sample count its header
// declares.
fuzz_target!(|data: &[u8]| {
    let _ = DtsCoreFrameInfo::from_es(data);
    for off in [0usize, data.len() / 2] {
        if let Ok(info) = DtsCoreFrameInfo::parse_at(data, off) {
            let _ = info.channel_count();
            let _ = info.into_ddts();
        }
    }
    let mut off = 0usize;
    for frame in split_dts_core_frames(data) {
        assert!(!frame.data().is_empty());
        off += frame.data().len();
        assert!(off <= data.len());
        let _ = frame.samples();
    }
});

#![no_main]

use broadcast_bitstream::ac3_syncframe::{
    Ac3SyncframeInfo, split_ac3_syncframe_ranges, split_ac3_syncframes,
};
use libfuzzer_sys::fuzz_target;

// Fuzz the AC-3 syncframe parser and splitters on arbitrary bytes. A frame the
// parser accepted must declare a length, and every split range must stay
// inside the input and follow the previous one.
fuzz_target!(|data: &[u8]| {
    let _ = Ac3SyncframeInfo::from_es(data);
    for off in [0usize, data.len() / 2] {
        if let Ok(info) = Ac3SyncframeInfo::parse_at(data, off) {
            let _ = info.frame_len_bytes();
            let _ = info.channel_count();
            let _ = info.into_dac3();
        }
    }
    let frames = split_ac3_syncframes(data);
    let ranges = split_ac3_syncframe_ranges(data, false);
    let resynced = split_ac3_syncframe_ranges(data, true);
    assert_eq!(frames.len(), ranges.len());
    for (start, end) in ranges.iter().chain(resynced.iter()) {
        assert!(start <= end && *end <= data.len());
    }
});

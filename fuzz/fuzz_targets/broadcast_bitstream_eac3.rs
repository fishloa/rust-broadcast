#![no_main]

use broadcast_bitstream::ac3_syncframe::{
    Ec3SyncframeInfo, ec3_specific_box_from_syncframes, split_eac3_syncframe_ranges,
    split_eac3_syncframes,
};
use libfuzzer_sys::fuzz_target;

// Fuzz the E-AC-3 syncframe parser, its access-unit splitter and the `dec3`
// builder on arbitrary bytes. A split access unit's concatenated bytes are
// exactly the syncframe ranges, and a `dec3` is only produced from frames that
// describe one.
fuzz_target!(|data: &[u8]| {
    let frames = Ec3SyncframeInfo::from_es_all(data);
    let _ = Ec3SyncframeInfo::from_es_first_au(data);
    let _ = Ec3SyncframeInfo::from_es_first_complete_au(data);
    let _ = Ec3SyncframeInfo::from_es_first_au_crc_verified(data, true);
    let _ = Ec3SyncframeInfo::first_crc_verified_bsmod(data, 0);
    let _ = ec3_specific_box_from_syncframes(&frames);

    let units = split_eac3_syncframes(data, &mut broadcast_bitstream::Findings::new());
    let ranges = split_eac3_syncframe_ranges(data, &mut broadcast_bitstream::Findings::new());
    for u in &units {
        let _ = u.info().samples_per_frame();
        for f in &frames {
            let _ = f.channel_count();
        }
    }
    for (start, end) in &ranges {
        assert!(start <= end && *end <= data.len());
    }
});

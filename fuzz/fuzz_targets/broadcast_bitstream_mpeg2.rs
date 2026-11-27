#![no_main]

use broadcast_bitstream::h262::{
    find_sequence_header, parse_sequence_at, picture_user_data_payloads,
};
use broadcast_bitstream::mpeg2::Mpeg2SeqHeader;
use libfuzzer_sys::fuzz_target;

// Fuzz the MPEG-2 video sequence walk on arbitrary bytes: a sequence header the
// base parser accepts must have a non-zero coded size, and the extension walk
// (a superset of that parse) must agree with it on the base 12-bit fields when
// no extension raises them.
fuzz_target!(|data: &[u8]| {
    if let Ok(seq) = Mpeg2SeqHeader::find(data) {
        assert!(seq.width() != 0 && seq.height() != 0);
        if let Some(start) = find_sequence_header(data)
            && let Ok(geometry) = parse_sequence_at(data, start)
        {
            assert!(geometry.coded_width() >= u32::from(seq.width()));
            assert!(geometry.coded_height() >= u32::from(seq.height()));
            // The reduced sample aspect ratio, if any, has non-zero terms.
            if let Some((w, h)) = geometry.sample_aspect_ratio() {
                assert!(w != 0 && h != 0);
            }
        }
    }
    // The picture user-data walk must not panic on any input.
    let _ = picture_user_data_payloads(data);
});

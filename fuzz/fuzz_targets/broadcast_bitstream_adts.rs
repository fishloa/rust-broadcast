#![no_main]

use broadcast_bitstream::aac_asc::{adts_header_len, build_adts_header, parse_adts_header};
use libfuzzer_sys::fuzz_target;

// Fuzz the ADTS header parser on arbitrary bytes: a parsed header must have a
// header length no larger than the frame it declares, and rebuilding the
// header from its own fields must reproduce a header the parser accepts.
fuzz_target!(|data: &[u8]| {
    let Ok(hdr) = parse_adts_header(data) else {
        return;
    };
    let header_len = adts_header_len(&hdr);
    assert!(header_len <= hdr.frame_length() as usize);

    let rebuilt = build_adts_header(
        hdr.profile(),
        hdr.sampling_frequency_index(),
        hdr.channel_configuration(),
        hdr.frame_length(),
    );
    assert_eq!(parse_adts_header(&rebuilt).map(|h| h.frame_length()), Ok(hdr.frame_length()));
});

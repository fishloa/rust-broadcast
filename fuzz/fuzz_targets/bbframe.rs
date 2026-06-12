#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let bbheader_len = dvb_bbframe::header::BBHEADER_LEN;

    // Parse header if possible.
    if let Ok(hdr) = dvb_bbframe::header::Bbheader::parse(data) {
        // Walk user packets over the data field.
        if data.len() > bbheader_len {
            for _up in dvb_bbframe::packet::up_iter(&data[bbheader_len..], &hdr) {}
        }

        // Try CarryOverExtractor: NM path with arbitrary data field.
        let mut extractor = dvb_bbframe::packet::CarryOverExtractor::new();
        if data.len() >= bbheader_len {
            let hdr_bytes: [u8; dvb_bbframe::header::BBHEADER_LEN] =
                match data[..bbheader_len].try_into() {
                    Ok(b) => b,
                    Err(_) => return,
                };
            let _ = extractor.feed_hem(&hdr_bytes, &data[bbheader_len..], false);
        }
    }
});

#![no_main]

use dvb_common::Serialize;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    roundtrip_bbheader(data);
    roundtrip_pat_section(data);
});

fn roundtrip_pat_section(data: &[u8]) {
    use dvb_common::Parse;
    use dvb_si::tables::pat::PatSection;

    let parsed = match PatSection::parse(data) {
        Ok(p) => p,
        Err(_) => return,
    };
    let serialized = parsed.to_bytes();
    let reparsed = match PatSection::parse(&serialized) {
        Ok(r) => r,
        Err(_) => return,
    };
    let reserialized = reparsed.to_bytes();
    assert_eq!(
        serialized, reserialized,
        "pat section roundtrip: serialized bytes differ"
    );
}

fn roundtrip_bbheader(data: &[u8]) {
    let hdr = match dvb_bbframe::header::Bbheader::parse(data) {
        Ok(h) => h,
        Err(_) => return,
    };
    let serialized = hdr.serialize();
    let reparsed = match dvb_bbframe::header::Bbheader::parse(&serialized) {
        Ok(r) => r,
        Err(_) => return,
    };
    let reserialized = reparsed.serialize();
    assert_eq!(
        serialized, reserialized,
        "bbheader roundtrip: serialized bytes differ"
    );
}

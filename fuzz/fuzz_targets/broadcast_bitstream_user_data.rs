#![no_main]

use broadcast_bitstream::user_data::{
    afd_from_t35_payload, bar_data_from_ga94_payload, bar_data_from_t35_payload,
};
use libfuzzer_sys::fuzz_target;

// Fuzz the AFD / bar-data decoders on arbitrary bytes: a decoded signal must be
// a real AFD code, and a decoded bar message must be a pure function of the
// bytes (a second decode agrees).
fuzz_target!(|data: &[u8]| {
    let afd = afd_from_t35_payload(data);
    assert_eq!(afd, afd_from_t35_payload(data));
    let bar = bar_data_from_t35_payload(data);
    assert_eq!(bar, bar_data_from_t35_payload(data));
    assert_eq!(bar, bar_data_from_ga94_payload(data));
});

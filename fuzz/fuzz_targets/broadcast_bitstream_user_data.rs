#![no_main]

use broadcast_bitstream::user_data::{
    T35_HEADER_LEN, USER_IDENTIFIER_LEN, afd_from_t35_payload, bar_data_from_ga94_payload,
    bar_data_from_t35_payload,
};
use libfuzzer_sys::fuzz_target;

// Fuzz the AFD / bar-data decoders on arbitrary bytes: every decoder is a pure
// function of its bytes, and the two bar-data carriages agree on the one thing
// they share — the T.35 entry point drops the country/provider prefix and hands
// the same `user_identifier` + `user_data_type_code` + `bar_data()` bytes to the
// GA94 entry point. They do not accept the same prefix, so one buffer cannot be
// compared across them.
fuzz_target!(|data: &[u8]| {
    let afd = afd_from_t35_payload(data);
    assert_eq!(afd, afd_from_t35_payload(data));
    let bar = bar_data_from_t35_payload(data);
    assert_eq!(bar, bar_data_from_t35_payload(data));
    let ga94 = bar_data_from_ga94_payload(data);
    assert_eq!(ga94, bar_data_from_ga94_payload(data));
    if let Some(bar) = bar {
        if let Some(tail) = data.get(T35_HEADER_LEN - USER_IDENTIFIER_LEN..) {
            assert_eq!(Some(bar), bar_data_from_ga94_payload(tail));
        }
    }
});

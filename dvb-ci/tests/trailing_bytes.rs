//! r10-O-1: a fixed-layout APDU whose `length_field` declares more than the
//! object's fixed body must be rejected, not have the extra bytes silently
//! dropped (which made parse -> serialize non-identical).

use broadcast_common::Parse;
use dvb_ci::Error;
use dvb_ci::objects::host_control::{Replace, Tune};

fn is_trailing(e: &Error) -> bool {
    matches!(
        e,
        Error::InvalidObject { reason, .. } if *reason == "trailing bytes after the fixed body"
    )
}

#[test]
fn tune_rejects_a_trailing_byte() {
    // Tune: 9F 84 00, length 8, network_id/onid/tsid/service_id.
    let exact = [
        0x9F, 0x84, 0x00, 0x08, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
    ];
    let t = Tune::parse(&exact).unwrap();
    assert_eq!(
        (
            t.network_id,
            t.original_network_id,
            t.transport_stream_id,
            t.service_id
        ),
        (0x1122, 0x3344, 0x5566, 0x7788)
    );
    let padded = [
        0x9F, 0x84, 0x00, 0x09, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0xEE,
    ];
    assert!(is_trailing(&Tune::parse(&padded).unwrap_err()));
}

#[test]
fn replace_rejects_a_trailing_byte() {
    // Replace: 9F 84 01, length 5.
    let exact = [0x9F, 0x84, 0x01, 0x05, 0x07, 0xE0, 0x64, 0xE0, 0x65];
    let r = Replace::parse(&exact).unwrap();
    assert_eq!(
        (r.replacement_ref, r.replaced_pid, r.replacement_pid),
        (0x07, 0x0064, 0x0065)
    );
    let padded = [0x9F, 0x84, 0x01, 0x06, 0x07, 0xE0, 0x64, 0xE0, 0x65, 0xEE];
    assert!(is_trailing(&Replace::parse(&padded).unwrap_err()));
}

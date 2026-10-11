//! Hostile-count allocation bounds (audit r10-O-6): an APDU whose wire count
//! claims far more entries than its body holds must be rejected without ever
//! allocating for the claimed count. A counting global allocator records the
//! largest single allocation, so the bound is deterministic (no timing, no
//! RSS sampling).

use broadcast_common::Parse;
use dvb_ci::ci_plus::cicam_player::PlayerCapabilitiesReply;

#[global_allocator]
static A: test_alloc::ProcessCounting = test_alloc::ProcessCounting::new();

#[test]
fn capabilities_reply_count_does_not_drive_allocation() {
    // CICAM_player_capabilities_reply = 9F A0 03, 1-byte length 2, then a
    // 16-bit count of 0xFFFF with no entries following.
    let apdu = [0x9F, 0xA0, 0x03, 0x02, 0xFF, 0xFF];
    A.reset_largest();
    let r = PlayerCapabilitiesReply::parse(&apdu);
    let largest = A.largest();
    assert!(r.is_err(), "a count with no entries must be rejected");
    // 0xFFFF entries x 2 bytes = 131 070 bytes if the count were trusted.
    assert!(
        largest < 1024,
        "largest single allocation was {largest} bytes: the wire count drove it"
    );
}

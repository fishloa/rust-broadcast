//! #2944: the SDP-file and request-body caps bound memory, so their numeric
//! values are pinned literally rather than only exercised through derived
//! boundary arithmetic.

use multimux::origin::DEFAULT_MAX_REQUEST_BODY_BYTES;
use multimux::source::sdp::MAX_SDP_FILE_BYTES;

#[test]
fn the_origin_caps_are_pinned_to_their_values() {
    assert_eq!(MAX_SDP_FILE_BYTES, 65_536);
    assert_eq!(DEFAULT_MAX_REQUEST_BODY_BYTES, 16_384);
}

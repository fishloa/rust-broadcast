//! #2944: the numeric value of the inbound-buffer cap is the security
//! property, so it is pinned literally along with both edges of the
//! accept/reject boundary and the announced-shortfall resume window.

use rtsp_runtime::client::MAX_PENDING_REQUESTS;
use rtsp_runtime::config::FIRST_CSEQ;
use rtsp_runtime::error::MessageParseError;
use rtsp_runtime::{ClientEvent, ClientSession, Error, MessageKind};

const CAP: usize = 1_048_576;

fn response_head(content_length: usize) -> String {
    format!("RTSP/1.0 200 OK\r\nCSeq: {FIRST_CSEQ}\r\nContent-Length: {content_length}\r\n\r\n")
}

/// A response whose head is complete but whose body stops short: the client
/// buffers the whole message unparsed and waits for `content_length` bytes.
fn response_buffered_exactly(total: usize) -> Vec<u8> {
    let head = response_head(2_000_000);
    assert!(head.len() < total);
    let mut wire = head.into_bytes();
    wire.extend(vec![b'x'; total - wire.len()]);
    assert_eq!(wire.len(), total);
    wire
}

#[test]
fn the_inbound_caps_are_pinned_to_their_values() {
    assert_eq!(rtsp_runtime::client::MAX_INBOUND_BYTES, 1_048_576);
    assert_eq!(MAX_PENDING_REQUESTS, 64);
}

#[test]
fn a_buffered_tail_at_the_cap_is_accepted_and_one_more_byte_is_refused() {
    let mut c = ClientSession::default();
    let at_cap = response_buffered_exactly(CAP);
    assert!(
        c.handle_data_strict(&at_cap).is_ok(),
        "a buffered tail exactly at the cap is not an error yet"
    );
    assert!(c.has_buffered_input());

    let err = c
        .handle_data_strict(&[b'x'])
        .expect_err("one byte past the cap is refused");
    match err {
        Error::MessageParse {
            what,
            source: MessageParseError::Buffered { buffered, limit },
        } => {
            assert_eq!(what, MessageKind::Response);
            assert_eq!(buffered, 1_048_577);
            assert_eq!(limit, 1_048_576);
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_message_completing_exactly_at_the_announced_shortfall_is_decoded() {
    let mut c = ClientSession::default();
    c.options("rtsp://h/s").expect("OPTIONS is legal from Init");
    let head = response_head(10);
    let mut wire = head.into_bytes();
    wire.extend_from_slice(b"0123456789");
    let split = wire.len() - 6;

    let first = c.handle_data_strict(&wire[..split]).expect("partial read");
    assert!(first.is_empty(), "the body is still short");
    assert!(c.has_buffered_input());

    let second = c
        .handle_data_strict(&wire[split..])
        .expect("the byte count the codec announced completes the message");
    assert!(
        second
            .iter()
            .any(|e| matches!(e, ClientEvent::Response { method, .. } if *method == rtsp_runtime::Method::Options)),
        "the completed response is delivered in this call: {second:?}"
    );
    assert!(!c.has_buffered_input());
}

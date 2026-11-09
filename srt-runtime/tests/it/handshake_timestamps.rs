//! Handshake packets carry the time since the connection started (§3.2.1),
//! not a constant 0.

use core::time::Duration;
use srt_runtime::HandshakeConfig;
use srt_runtime::caller::CallerHandshake;
use srt_runtime::handshake_sm::HandshakeOutput;
use srt_runtime::listener::ListenerHandshake;
use srt_runtime::packet::ControlPacket;
use srt_runtime::rendezvous::RendezvousHandshake;

fn timestamp(bytes: &[u8]) -> u32 {
    match ControlPacket::parse(bytes).expect("control packet") {
        ControlPacket::Handshake(hp) => hp.timestamp,
        _ => panic!("not a handshake"),
    }
}

fn sent(outputs: Vec<HandshakeOutput>) -> Vec<u8> {
    outputs
        .into_iter()
        .find_map(|o| match o {
            HandshakeOutput::Send(b) => Some(b),
            _ => None,
        })
        .expect("a Send output")
}

#[test]
fn caller_and_listener_stamp_the_elapsed_time_on_every_packet() {
    let mut caller = CallerHandshake::new(0x1111_1111, HandshakeConfig::default());
    let mut listener = ListenerHandshake::new(0x2222_2222, 0xC0FF_EE42, HandshakeConfig::default());

    caller.set_elapsed(Duration::from_millis(5));
    let induction = caller.start().unwrap();
    assert_eq!(timestamp(&induction), 5_000);

    listener.set_elapsed(Duration::from_millis(7));
    let pkt = ControlPacket::parse(&induction).unwrap();
    let response = sent(listener.feed(&pkt).unwrap());
    assert_eq!(timestamp(&response), 7_000);

    caller.set_elapsed(Duration::from_millis(250));
    let pkt = ControlPacket::parse(&response).unwrap();
    let conclusion = sent(caller.feed(&pkt).unwrap());
    assert_eq!(timestamp(&conclusion), 250_000);

    listener.set_elapsed(Duration::from_millis(260));
    let pkt = ControlPacket::parse(&conclusion).unwrap();
    let reply = sent(listener.feed(&pkt).unwrap());
    assert_eq!(timestamp(&reply), 260_000);
}

#[test]
fn rendezvous_stamps_the_elapsed_time() {
    let mut r = RendezvousHandshake::new(0x3333_3333, 0xABCD_0001, HandshakeConfig::default());
    r.set_elapsed(Duration::from_millis(40));
    let first = r.start().unwrap();
    assert_eq!(timestamp(&first), 40_000);
}

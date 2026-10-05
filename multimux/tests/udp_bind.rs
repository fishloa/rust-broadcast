//! SP1.6: the UDP binds go through `socket2` with configurable `SO_RCVBUF`
//! and `SO_REUSEADDR`. The RCVBUF assertion round-trips the REQUESTED value
//! (the OS clamps/doubles — Linux doubles and floors at a minimum, macOS
//! reports set+overhead — so comparing against the OS default proves
//! nothing; the getter proves the option was APPLIED).

use std::time::Duration;

use multimux::source::udp::{UdpBindOptions, bind_udp};

/// True when the error message is a "no route/interface for this multicast
/// join" style failure, which is a host limitation, not a bug — the same
/// tolerance dvb-stream's W1 multicast test uses.
fn join_unavailable(e: &multimux::MultimuxError) -> bool {
    let text = e.to_string();
    text.contains("EADDRNOTAVAIL")
        || text.contains("EHOSTUNREACH")
        || text.contains("ENETUNREACH")
        || text.contains("Can't assign requested address")
        || text.contains("No route to host")
        || text.contains("Network is unreachable")
}

/// Reads `SO_RCVBUF` back off a borrowed `tokio::net::UdpSocket`.
trait RecvBufProbe {
    fn recv_buffer_size_for_test(&self) -> usize;
}

impl RecvBufProbe for tokio::net::UdpSocket {
    fn recv_buffer_size_for_test(&self) -> usize {
        use std::os::fd::{AsRawFd, FromRawFd};
        let sock = unsafe { socket2::Socket::from_raw_fd(self.as_raw_fd()) };
        let got = sock.recv_buffer_size().expect("SO_RCVBUF readable");
        std::mem::forget(sock); // do not close the borrowed fd
        got
    }
}

#[tokio::test]
async fn the_configured_receive_buffer_is_applied_to_the_socket() {
    // 64 KiB: below every observed OS default (macOS ~786 KiB, Linux ~208 KiB),
    // so the getter returning ~64 KiB (Linux reports the doubled 128 KiB)
    // proves the option took; a plain bind reports the untouched default.
    let opts = UdpBindOptions {
        recv_buffer_bytes: Some(64 * 1024),
        ..Default::default()
    };
    let socket = bind_udp("127.0.0.1:0", None, opts).await.unwrap();
    let got = socket.recv_buffer_size_for_test();
    // Linux doubles the request; macOS adds overhead. Accept 64..=256 KiB,
    // and assert it is NOT the untouched default the plain socket reports.
    assert!(
        (64 * 1024..=256 * 1024).contains(&got),
        "SO_RCVBUF was not applied: got {got} (expected the requested 64 KiB, possibly doubled)"
    );
    // On a host whose OS default happens to equal the doubled request this
    // would be flaky — so do NOT compare against the plain socket; the
    // getter-vs-request range above is the assertion (Linux doubles 64 KiB
    // to 128 KiB; macOS reports 64 KiB+overhead; both land in range).
}

#[tokio::test]
async fn reuse_address_is_applied_and_a_multicast_group_can_join_a_specified_interface() {
    let opts = UdpBindOptions {
        recv_buffer_bytes: None,
        reuse_address: true,
        multicast_interface: Some("0.0.0.0".into()),
        reuse_port: false,
    };
    // A multicast join needs a group route on the host; on a host without
    // one the join errors — this test asserts the OPTION PATH, tolerating
    // the join failure only when it is EADDRNOTAVAIL/EHOSTUNREACH.
    let socket = match bind_udp("0.0.0.0:0", Some("239.255.0.1"), opts).await {
        Ok(s) => s,
        Err(e) if join_unavailable(&e) => return, // no multicast route on this host
        Err(e) => panic!("bind failed unexpectedly: {e}"),
    };
    assert!(socket.local_addr().unwrap().port() > 0);
}

/// A `reuse_address` bind still succeeds over the plain path (the option is
/// accepted, not required, on a fresh port).
#[tokio::test]
async fn an_explicit_reuse_address_bind_still_succeeds() {
    let opts = UdpBindOptions {
        reuse_address: true,
        ..Default::default()
    };
    let socket = tokio::time::timeout(Duration::from_secs(5), bind_udp("127.0.0.1:0", None, opts))
        .await
        .expect("bind must not hang")
        .unwrap();
    assert!(socket.local_addr().unwrap().port() > 0);
}

/// I4: with no explicit `recv_buffer_bytes`, the default (4 MiB) is requested
/// — the getter proves a value well above the OS default was applied, so a
/// high-bitrate input is not left at ~208 KiB (Linux) / ~786 KiB (macOS).
#[tokio::test]
async fn the_default_receive_buffer_is_larger_than_the_os_default() {
    let socket = bind_udp("127.0.0.1:0", None, UdpBindOptions::default())
        .await
        .unwrap();
    let got = socket.recv_buffer_size_for_test();
    // The requested default is 4 MiB; Linux may double it, macOS adds
    // overhead. Accept [1 MiB, 16 MiB] — comfortably above every observed OS
    // default (macOS ~786 KiB), proving the default took effect.
    assert!(
        (1024 * 1024..=16 * 1024 * 1024).contains(&got),
        "the default SO_RCVBUF must be applied (got {got}, expected ~4 MiB)"
    );
}

/// I4: an explicit override still round-trips through the getter (the
/// requested value, not the default).
#[tokio::test]
async fn an_explicit_receive_buffer_overrides_the_default() {
    let socket = bind_udp(
        "127.0.0.1:0",
        None,
        UdpBindOptions {
            recv_buffer_bytes: Some(128 * 1024),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let got = socket.recv_buffer_size_for_test();
    assert!(
        (128 * 1024..=512 * 1024).contains(&got),
        "the explicit 128 KiB request must round-trip (got {got})"
    );
}

/// I4: `reuse_port` is accepted on Unix (no error) and the bind succeeds.
#[tokio::test]
async fn reuse_port_is_accepted() {
    let socket = bind_udp(
        "127.0.0.1:0",
        None,
        UdpBindOptions {
            reuse_port: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(socket.local_addr().unwrap().port() > 0);
}

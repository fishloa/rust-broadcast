#![cfg(all(feature = "whip", feature = "whep", feature = "test-hooks"))]
//! A publisher whose connection is reset (`SO_LINGER 0`) makes the session read
//! fail (`ReadOutcome::TransportError`). The session must be finished and reaped
//! so its `max_sessions` slot is free for the next publisher: more resets than
//! `DEFAULT_RTMP_MAX_SESSIONS` in a row would starve admission if the slot leaked.

use std::time::Duration;

use multimux::source::rtmp::DEFAULT_RTMP_MAX_SESSIONS;
use tokio::io::AsyncWriteExt as _;

#[tokio::test]
async fn a_reset_publisher_releases_its_slot_so_later_publishers_are_still_admitted() {
    let (server_addr, handle, token) =
        multimux::test_seam::serve_rtmp_for_test_with_read_load().await;

    let rounds = DEFAULT_RTMP_MAX_SESSIONS + 6;
    let mut admitted = Ok(0);
    for round in 0..rounds {
        let mut publisher = tokio::net::TcpStream::connect(server_addr).await.unwrap();
        publisher.write_all(&[0x03]).await.unwrap();
        socket2::SockRef::from(&publisher)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(publisher);
        admitted = handle
            .wait_for_sessions(round + 1, Duration::from_secs(5))
            .await;
        if admitted.is_err() {
            break;
        }
    }
    token.cancel();
    let admitted = admitted.expect("every reset publisher must still be admitted");
    assert!(admitted >= rounds, "{admitted} of {rounds} admitted");
}

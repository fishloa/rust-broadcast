//! A signalling session and a media transport must stay movable between
//! threads, or neither can be run as its own task on a multi-thread runtime. A
//! `Cell`/`Rc` added later would break that silently; these fail to compile
//! instead.

use webrtc_runtime::whep::server::WhepSession;
use webrtc_runtime::whip::client::WhipClient;

fn assert_send<T: Send>() {}

#[test]
fn signalling_sessions_can_run_as_their_own_task() {
    assert_send::<WhepSession>();
    assert_send::<WhipClient>();
}

#[cfg(feature = "media")]
#[test]
fn a_media_transport_can_run_as_its_own_task() {
    assert_send::<webrtc_runtime::media::MediaTransport>();
}

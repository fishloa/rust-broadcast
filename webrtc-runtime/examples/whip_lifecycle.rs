//! Demonstrate the full WHIP client lifecycle with mock HTTP responses.
//!
//! Usage: cargo run -p webrtc-runtime --example whip_lifecycle

use webrtc_runtime::whip::client::{HttpResponse, WhipClient};

fn main() {
    let mut client = WhipClient::new(
        "https://origin.example/whip/live".into(),
        Some("my-token".into()),
    );

    // Step 1: generate the SDP offer POST request.
    let offer_req = client
        .offer(b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\n".to_vec())
        .expect("offer");
    println!("1. {:?} {}", offer_req.method, offer_req.url);
    println!("   Content-Type: {:?}", offer_req.content_type);

    // Step 2: feed the 201 Created response (SDP answer).
    let event = client
        .on_response(HttpResponse {
            status: 201,
            content_type: Some("application/sdp".into()),
            location: Some("https://origin.example/whip/live/session-abc".into()),
            etag: Some("33a64df5".into()),
            body: b"v=0\r\no=- 1 1 IN IP4 192.0.2.1\r\n".to_vec(),
        })
        .expect("on_response");
    println!("2. Event: {event:?}");
    println!("   State: {:?}", client.state());

    // Step 3: terminate the session.
    let delete_req = client.terminate().expect("terminate");
    println!("3. {:?} {}", delete_req.method, delete_req.url);

    println!("\nLifecycle complete.");
}

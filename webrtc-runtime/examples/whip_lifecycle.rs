//! Demonstrate the full WHIP client lifecycle with mock HTTP responses.
//!
//! Usage: cargo run -p webrtc-runtime --example whip_lifecycle

use http::StatusCode;
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
    println!(
        "   Content-Type: {}",
        offer_req
            .content_type()
            .map(|c| c.to_string())
            .unwrap_or_default()
    );

    // Step 2: feed the 201 Created response (SDP answer).
    let event = client
        .on_response(
            HttpResponse::new(StatusCode::CREATED)
                .with_content_type("application/sdp")
                .expect("content type")
                .with_location("https://origin.example/whip/live/session-abc")
                .expect("location")
                .with_etag("33a64df5")
                .expect("etag")
                .with_body(b"v=0\r\no=- 1 1 IN IP4 192.0.2.1\r\n".to_vec()),
        )
        .expect("on_response");
    println!("2. Event: {event:?}");
    println!("   State: {:?}", client.state());

    // Step 3: terminate the session.
    let delete_req = client.terminate().expect("terminate");
    println!("3. {:?} {}", delete_req.method, delete_req.url);

    println!("\nLifecycle complete.");
}

//! Parse and format ICE server Link headers (RFC 9725 §4.4).
//!
//! Usage: cargo run -p webrtc-runtime --example ice_server_links

use webrtc_runtime::ice::{IceServer, format_ice_server_links, parse_ice_server_links};

fn main() {
    // Parse a Link header containing both STUN and TURN servers.
    let header = concat!(
        r#"<stun:stun.l.google.com:19302>; rel="ice-server", "#,
        r#"<turn:turn.example.com?transport=udp>; rel="ice-server"; "#,
        r#"username="user"; credential="pass""#,
    );

    let servers = parse_ice_server_links(header);
    println!("Parsed {} ICE server(s):\n", servers.len());
    for s in &servers {
        println!("  URL:        {}", s.url);
        println!("  Username:   {:?}", s.username);
        println!("  Credential: {:?}", s.credential);
        println!();
    }

    // Build servers programmatically and format back to a Link header.
    let custom = vec![
        IceServer {
            url: "stun:stun.example.org".into(),
            username: None,
            credential: None,
        },
        IceServer {
            url: "turns:turn.example.org?transport=tcp".into(),
            username: Some("alice".into()),
            credential: Some("secret".into()),
        },
    ];

    let formatted = format_ice_server_links(&custom);
    println!("Formatted Link header:\n  {formatted}");

    // Verify round-trip.
    let reparsed = parse_ice_server_links(&formatted);
    assert_eq!(reparsed, custom, "round-trip mismatch");
    println!("\n(round-trip verified OK)");
}

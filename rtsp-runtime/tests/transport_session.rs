//! `Transport` (RFC 2326 §12.39) and `Session` (§12.37) header parsers/serializers:
//! every example header in the RFC, the `rtsp-types` probe failures, interop shapes,
//! error cases, real interop-test headers, and the symmetric round-trip invariants.

use std::time::Duration;

use rtsp_runtime::session_header::DEFAULT_SESSION_TIMEOUT;
use rtsp_runtime::{Delivery, LowerTransport, SessionHeader, Transport, TransportMode};

/// The three round-trip invariants for any accepted Transport value.
fn check_transport_invariants(input: &str) -> Transport {
    let a = Transport::parse(input).unwrap_or_else(|e| panic!("{input:?}: {e}"));
    let canon = a.to_header_value().unwrap();
    let b = Transport::parse(&canon).unwrap_or_else(|e| panic!("{canon:?}: {e}"));
    assert_eq!(
        a, b,
        "parse -> serialize -> parse differs for {input:?} ({canon:?})"
    );
    assert_eq!(
        b.to_header_value().unwrap(),
        canon,
        "canonical form not idempotent for {input:?}"
    );
    a
}

fn check_session_invariants(input: &str) -> SessionHeader {
    let a = SessionHeader::parse(input).unwrap_or_else(|e| panic!("{input:?}: {e}"));
    let canon = a.to_header_value().unwrap();
    let b = SessionHeader::parse(&canon).unwrap();
    assert_eq!(a, b, "{input:?} -> {canon:?}");
    assert_eq!(b.to_header_value().unwrap(), canon);
    a
}

/// Every `Transport:` / `Session:` header in the RFC text (`docs/rfc2326.md`), with
/// continuation lines joined (a header ends at a line not ending in `;` or `,`).
fn rfc_headers(name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/rfc2326.md"))
        .expect("docs/rfc2326.md");
    let lines: Vec<&str> = text.lines().collect();
    let prefix = format!("{name}:");
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].trim_start().strip_prefix(prefix.as_str()) {
            let mut value = rest.trim().to_string();
            while value.ends_with(';') || value.ends_with(',') {
                i += 1;
                // skip markdown fence/blank lines that interrupt an example
                while i < lines.len()
                    && (lines[i].trim().is_empty() || lines[i].trim_start().starts_with("```"))
                {
                    i += 1;
                }
                value.push_str(lines[i].trim());
            }
            out.push(value);
        }
        i += 1;
    }
    out
}

#[test]
fn every_transport_example_in_rfc_2326_parses_and_round_trips() {
    let headers = rfc_headers("Transport");
    assert!(
        headers.len() >= 25,
        "found {} Transport examples",
        headers.len()
    );
    for h in &headers {
        // The mode="PLAY",<newline>RTP/AVP... example is a comma-continued list.
        check_transport_invariants(h);
    }
    // the §12.39 two-spec example, joined from its continuation line
    let two = Transport::parse(r#"RTP/AVP;multicast;ttl=127;mode="PLAY",RTP/AVP;unicast;client_port=3456-3457;mode="PLAY""#).unwrap();
    assert_eq!(two.specs.len(), 2);
    assert_eq!(two.specs[0].delivery, Some(Delivery::Multicast));
    assert_eq!(two.specs[0].ttl, Some(127));
    assert_eq!(two.specs[1].client_port, Some((3456, 3457)));
    assert_eq!(two.specs[1].mode, vec![TransportMode::Play]);
}

#[test]
fn every_session_example_in_rfc_2326_parses_and_round_trips() {
    let headers = rfc_headers("Session");
    assert!(
        headers.len() >= 40,
        "found {} Session examples",
        headers.len()
    );
    for h in &headers {
        let s = check_session_invariants(h);
        assert!(!s.id.is_empty());
    }
}

/// The inputs `rtsp-types` 0.1.3 mis-parses (`.delegate/rtsp-types-probe.txt`), with the
/// defined result of this parser.
#[test]
fn the_rtsp_types_probe_inputs_have_defined_results() {
    let ms = |v: &str| SessionHeader::parse(v).unwrap();
    let thirty = Some(Duration::from_secs(30));
    assert_eq!(ms("abc; timeout=30").timeout, thirty);
    assert_eq!(ms("abc ; timeout=30").id, "abc");
    assert_eq!(ms("abc ; timeout=30").timeout, thirty);
    assert_eq!(ms("abc;timeout = 30").timeout, thirty);
    assert_eq!(ms("abc;TIMEOUT=30").timeout, thirty);
    assert_eq!(ms("abc ;timeout=30").id, "abc");
    let (h, w) = SessionHeader::parse_with_warnings("abc;timeout=oops").unwrap();
    assert_eq!(
        (h.id.as_str(), h.timeout),
        ("abc", Some(DEFAULT_SESSION_TIMEOUT))
    );
    assert_eq!(w.len(), 1);
    // received ids are kept verbatim (interop), whatever they contain
    assert_eq!(ms("\"weird\"").id, "\"weird\"");
    assert_eq!(ms("a b;timeout=30").id, "a b");
    assert_eq!(ms("abc,def").id, "abc,def");
    assert_eq!(ms("ab\"c").id, "ab\"c");
    for v in ["\"weird\"", "a b;timeout=30", "abc,def", "ab\"c"] {
        check_session_invariants(v);
    }
    let (h, w) = SessionHeader::parse_with_warnings("abc;timeout=0").unwrap();
    assert_eq!(h.timeout, Some(DEFAULT_SESSION_TIMEOUT));
    assert_eq!(w.len(), 1);

    let t = check_transport_invariants("rtp/avp/tcp;interleaved=0-1");
    assert_eq!(
        t.first().unwrap().lower_transport,
        Some(LowerTransport::Tcp)
    );
    assert_eq!(t.first().unwrap().interleaved, Some((0, 1)));
    let t = check_transport_invariants("RTP/AVP/TCP;Interleaved=0-1");
    assert_eq!(t.first().unwrap().interleaved, Some((0, 1)));
    let t = check_transport_invariants("RTP/AVP;UNICAST;client_port=1-2");
    assert_eq!(t.first().unwrap().delivery, Some(Delivery::Unicast));
    let t = check_transport_invariants("RTP/AVP;unicast;SSRC=DEADBEEF");
    assert_eq!(t.first().unwrap().ssrc, Some(0xDEAD_BEEF));
    let t = check_transport_invariants("RTP/AVP;unicast;ssrc=\"DEADBEEF\"");
    assert_eq!(t.first().unwrap().ssrc, Some(0xDEAD_BEEF));
    let t = check_transport_invariants(r#"RTP/AVP;unicast;mode="PLAY,RECORD""#);
    assert_eq!(
        t.first().unwrap().mode,
        vec![TransportMode::Play, TransportMode::Record]
    );
    let t = check_transport_invariants(r#"RTP/AVP;unicast;mode="PLAY, RECORD""#);
    assert_eq!(
        t.first().unwrap().mode,
        vec![TransportMode::Play, TransportMode::Record]
    );
    let t = check_transport_invariants("RTP/AVP;unicast;mode=play");
    assert_eq!(t.first().unwrap().mode, vec![TransportMode::Play]);
    let t = check_transport_invariants("RTP/AVP/TCP ; unicast ; interleaved=0-1");
    assert_eq!(t.first().unwrap().interleaved, Some((0, 1)));
    let t = check_transport_invariants("RTP/AVP;unicast;X-Foo=Bar");
    assert_eq!(
        t.first().unwrap().extensions,
        vec![("X-Foo".to_string(), Some("Bar".to_string()))]
    );
    let t = check_transport_invariants(r#"RTP/AVP;unicast;destination="a,b""#);
    assert_eq!(t.specs.len(), 1, "a quoted comma does not split the specs");
    assert_eq!(t.first().unwrap().destination.as_deref(), Some("a,b"));
    let t = check_transport_invariants(
        "RTP/AVP;unicast;interleaved=0-1,RTP/AVP;unicast;client_port=1-2",
    );
    assert_eq!(t.specs.len(), 2);
}

#[test]
fn mixed_case_lws_and_single_ended_ranges() {
    let t = check_transport_invariants(
        "rTp / aVp / uDp ; UniCast ; Client_Port = 8000 - 8001 ; Interleaved=6 ;Append",
    );
    let s = t.first().unwrap();
    assert_eq!(s.lower_transport, Some(LowerTransport::Udp));
    assert_eq!(s.client_port, Some((8000, 8001)));
    assert_eq!(s.interleaved, Some((6, 6)), "one channel means hi = lo");
    assert!(s.append);
    let t = check_transport_invariants("RTP/AVP;unicast;client_port=10000");
    assert_eq!(t.first().unwrap().client_port, Some((10000, 10000)));
    let t = check_transport_invariants("RTP/AVP;multicast;destination;ttl=5;layers=2");
    assert_eq!(
        t.first().unwrap().destination.as_deref(),
        Some(""),
        "bare destination"
    );
    assert_eq!(t.first().unwrap().layers, Some(2));
}

#[test]
fn malformed_values_are_errors() {
    for bad in [
        "",
        " , ",
        "RTP/AVP;client_port=1-2-3",
        "RTP/AVP;client_port=65536",
        "RTP/AVP;client_port=123456",
        "RTP/AVP;client_port=a-b",
        "RTP/AVP;client_port",
        "RTP/AVP;port=1-",
        "RTP/AVP;ttl=256",
        "RTP/AVP;ttl=1000",
        "RTP/AVP;interleaved=256",
        "RTP/AVP;ssrc=DEADBEE",
        "RTP/AVP;ssrc=DEADBEEF0",
        "RTP/AVP;ssrc=GGGGGGGG",
        "RTP/AVP;unicast=1",
        "RTP/AVP;append=1",
        "RTP/AVP;layers=x",
        "RTP/SAVP",
        "RTP/AVP/SCTP",
        "XYZ/AVP",
        "RTP",
        "RTP/AVP/TCP/X",
        "RTP/AVP;mode=\"PLAY",
        "RTP/AVP;a b=c",
    ] {
        assert!(Transport::parse(bad).is_err(), "{bad:?} must be rejected");
    }
    assert!(SessionHeader::parse("").is_err());
    assert!(
        SessionHeader::parse("a\r\nb").is_err(),
        "control characters are rejected"
    );
}

#[test]
fn canonical_output_is_in_spec_listing_order_then_extensions() {
    let t = Transport::parse(
        "RTP/AVP/UDP;x-a=1;mode=record;ssrc=deadbeef;server_port=2-3;client_port=4-5;port=6-7;layers=2;ttl=9;append;interleaved=0-1;source=s;destination=d;multicast;x-b",
    )
    .unwrap();
    assert_eq!(
        t.to_header_value().unwrap(),
        "RTP/AVP/UDP;multicast;destination=d;source=s;interleaved=0-1;append;ttl=9;layers=2;port=6-7;client_port=4-5;server_port=2-3;ssrc=DEADBEEF;mode=\"RECORD\";x-a=1;x-b"
    );
}

#[test]
fn unknown_parameters_keep_order_and_quoting_semantics() {
    let t = check_transport_invariants(r#"RTP/AVP;z=1;a="x y";m;b"#);
    let ext: Vec<_> = t
        .first()
        .unwrap()
        .extensions
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(ext, ["z", "a", "m", "b"]);
    assert_eq!(t.first().unwrap().extensions[1].1.as_deref(), Some("x y"));
}

/// Headers exactly as they appear in this repository's interop tests (the hand-written
/// RFC-style session fixture `tests/fixtures/session_tcp_interleaved.md`, `tests/io_loopback.rs`,
/// `tests/integration.rs` and multimux's `source/rtsp.rs` tests). No RTSP packet capture exists
/// under `private/fixtures` or `.test-streams`.
#[test]
fn headers_from_the_repository_interop_tests() {
    for t in [
        "RTP/AVP/TCP;interleaved=0-1",
        "RTP/AVP/TCP;unicast;interleaved=0-1",
        "RTP/AVP/TCP;interleaved=4-5",
    ] {
        let tr = check_transport_invariants(t);
        assert_eq!(
            tr.first().unwrap().lower_transport,
            Some(LowerTransport::Tcp)
        );
    }
    assert_eq!(
        check_session_invariants("12345678; timeout=2").timeout,
        Some(Duration::from_secs(2))
    );
    assert_eq!(check_session_invariants("ABC123;timeout=60").id, "ABC123");
    for id in ["12345678", "42", "TLSTEST"] {
        assert_eq!(check_session_invariants(id).id, id);
    }
    // hostile value from multimux's tests: must parse without overflow trouble
    let h = SessionHeader::parse("x;timeout=18446744073709551615").unwrap();
    assert!(h.timeout.is_some());
}

/// Serializer injection safety (review round 3): control characters are rejected, never emitted.
#[test]
fn the_serializer_rejects_control_characters_and_non_token_names() {
    use rtsp_runtime::{Error, TransportSpec};
    // CRLF smuggled into a quoted value
    let mut spec = TransportSpec::default();
    spec.destination = Some("a\r\nSet-Cookie: x".into());
    assert!(matches!(
        Transport::single(spec).to_header_value(),
        Err(Error::HeaderSerialize(_))
    ));
    // CRLF in an unknown parameter's value and name
    let mut spec = TransportSpec::default();
    spec.extensions.push(("x".into(), Some("q\nr".into())));
    assert!(Transport::single(spec).to_header_value().is_err());
    let mut spec = TransportSpec::default();
    spec.extensions.push(("x\r\ny".into(), None));
    assert!(Transport::single(spec).to_header_value().is_err());
    // a non-token mode method
    let mut spec = TransportSpec::default();
    spec.mode.push(TransportMode::Other("A\r\nB".into()));
    assert!(Transport::single(spec).to_header_value().is_err());
    // an id and an extension name on the Session side
    let mut h = SessionHeader::new("ok");
    h.id = "a\r\nb".into();
    assert!(h.to_header_value().is_err());
    let mut h = SessionHeader::new("ok");
    h.extensions.push(("n\r\nm".into(), None));
    assert!(h.to_header_value().is_err());
    // and the parsers reject a CRLF inside a quoted value up front
    assert!(Transport::parse("RTP/AVP;destination=\"a\r\nb\"").is_err());
}

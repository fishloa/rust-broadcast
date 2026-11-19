//! Audit-22 #3045: `TransportSecurity::parse` must accept all three schema
//! values (`none`, `integrity`, `integrityAndAuthenticity`), and the writer
//! must round-trip each. Only `integrityAndAuthenticity` appeared in the
//! fixtures, so two of the three arms were unverified.
#![cfg(feature = "std")]

use dvb_mabr::{MulticastServerConfiguration, TransportSecurity};

const NS: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";

fn session_with_security(value: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastServerConfiguration xmlns="{NS}" schemaVersion="2">
  <MulticastSession serviceIdentifier="urn:example:service">
    <PresentationManifestLocator manifestId="m1" contentType="application/dash+xml">https://example.com/m.mpd</PresentationManifestLocator>
    <MulticastTransportSession id="ts1" sessionIdleTimeout="1000" transportSecurity="{value}">
      <TransportProtocol protocolIdentifier="urn:example:protocol" protocolVersion="1"/>
      <BitRate maximum="1000"/>
    </MulticastTransportSession>
  </MulticastSession>
</MulticastServerConfiguration>"#
    )
}

#[test]
fn each_transport_security_value_parses_to_its_variant_and_round_trips() {
    let cases = [
        ("none", TransportSecurity::None),
        ("integrity", TransportSecurity::Integrity),
        (
            "integrityAndAuthenticity",
            TransportSecurity::IntegrityAndAuthenticity,
        ),
    ];
    for (wire, expected) in cases {
        let xml = session_with_security(wire);
        let parsed = MulticastServerConfiguration::parse_str(&xml)
            .unwrap_or_else(|e| panic!("transportSecurity={wire} must parse: {e}"));
        assert_eq!(
            parsed.sessions[0].transport_sessions[0].transport_security,
            Some(expected),
            "transportSecurity={wire}"
        );

        let out = parsed.to_xml().expect("render");
        assert!(
            out.contains(&format!("transportSecurity=\"{wire}\"")),
            "transportSecurity={wire} must be written back literally:\n{out}"
        );
        let reparsed = MulticastServerConfiguration::parse_str(&out).expect("re-parse");
        assert_eq!(reparsed, parsed, "transportSecurity={wire} round trip");
    }
}

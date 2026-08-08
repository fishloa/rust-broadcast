//! Parse a DVB-MABR configuration from XML, modify it, and serialize back.
//!
//! Usage: cargo run -p dvb-mabr --example build_session

use dvb_mabr::MulticastGatewayConfiguration;

fn main() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration schemaVersion="2"
    xmlns="urn:dvb:metadata:MulticastSessionConfiguration:2024"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <MulticastSession serviceIdentifier="urn:example:service:1">
    <PresentationManifestLocator manifestId="mpd1" contentType="application/dash+xml"
        >https://cdn.example/service1/manifest.mpd</PresentationManifestLocator>
    <MulticastTransportSession id="ts1" sessionIdleTimeout="30000">
      <TransportProtocol protocolIdentifier="urn:dvb:metadata:cs:MulticastTransportProtocolCS:2019:FLUTE" protocolVersion="1"/>
      <EndpointAddress>
        <NetworkDestinationGroupAddress>239.1.1.1</NetworkDestinationGroupAddress>
        <TransportDestinationPort>6000</TransportDestinationPort>
      </EndpointAddress>
      <BitRate maximum="5000000"/>
    </MulticastTransportSession>
  </MulticastSession>
</MulticastGatewayConfiguration>"#;

    let mut config = MulticastGatewayConfiguration::parse_str(xml).expect("parse");

    println!("Original:");
    println!("  Schema version: {}", config.schema_version);
    println!("  Sessions: {}", config.sessions.len());
    let session = &config.sessions[0];
    println!("  Service: {}", session.service_identifier);
    println!("  Transports: {}", session.transport_sessions.len());

    config.sessions[0].service_identifier = "urn:example:service:modified".into();

    let output = config.to_xml();
    println!("\nSerialized XML:\n{output}");

    let reparsed = MulticastGatewayConfiguration::parse_str(&output).expect("round-trip");
    assert_eq!(reparsed, config, "round-trip mismatch");
    println!("(round-trip verified OK)");
}

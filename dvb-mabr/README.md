# dvb-mabr

DVB multicast ABR (ETSI TS 103 769 V1.2.1) session configuration XML parser and serializer.

[![crates.io](https://img.shields.io/crates/v/dvb-mabr.svg)](https://crates.io/crates/dvb-mabr)
[![docs.rs](https://docs.rs/dvb-mabr/badge.svg)](https://docs.rs/dvb-mabr)

## What it does / what it does not

**Does.** DVB-MABR delivers adaptive-bitrate streaming (DASH/HLS) over
multicast. A Multicast server, a Multicast gateway in the home or network, and
a client-facing rendezvous service are all coordinated by one XML document
format: the multicast session configuration instance document (clause 10).
This crate parses it into typed Rust structures and serializes it back. Users
are head-end/gateway software that generates or consumes that configuration.

**Does not.**
- It does not parse DASH MPDs or HLS playlists: presentation manifests are
  referenced by URL only (ISO/IEC 23009-1, RFC 8216 are out of scope; see
  `transmux` and `broadcast-hls`).
- It does not implement the multicast transport itself (FLUTE/ROUTE objects,
  FEC decoding): see `rmt-flute` for the ALC/LCT/FLUTE wire formats.
- It does not implement the clause 11 reporting JSON body, only the
  reporting-destination declarations inside the configuration document.
- It does not store or re-emit extension elements/attributes (Annex A.1); they
  are skipped on parse for forward compatibility.
- It does not validate against the XSD schema; it is a typed reader/writer.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI TS 103 769, DVB; Content Delivery Protocols; Multicast ABR | V1.2.1 (`etsi_ts_103_769_v01.02.01_dvb_mabr`, listed in `private/specs-md/INDEX.md`) | Clause 10.2.1 document roots `MulticastServerConfiguration` and `MulticastGatewayConfiguration` | Full | Namespaces for the 2019 and 2024 schemas are accepted |
| | | Clause 10.2.2 `MulticastSession`, `PresentationManifestLocator` | Full | |
| | | Clause 10.2.3 `MulticastTransportSession`: endpoints, bit rate, transmission mode, security, FEC parameters, unicast repair, in-band object carousel | Full | |
| | | Clause 10.2.4 service component identifiers (DASH, HLS, generic) | Full | |
| | | Clause 10.2.5 `MulticastGatewayConfigurationTransportSession` and configuration macros | Partial | The `ReferencingObjectCarouselType` vs base `ObjectCarouselType` cardinality rule is not enforced by the types |
| | | Clause 10.2.1.0 / 10.2.2.3 reporting-destination declarations | Full | |
| | | Annex A.1 extension points (`xs:any`) | Not implemented | Recognised and skipped on parse, not stored |
| | | Clause 11 reporting JSON; Annex F transport profiles | Not implemented | Reporting body documented in `docs/mabr-reporting.md` only |

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Enables the whole XML API (built on `quick-xml`, which needs `std`). Without it only the `error` module is available (`no_std`). |

## Quick start

```rust
use dvb_mabr::MulticastGatewayConfiguration;

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
      <ServiceComponentIdentifier xsi:type="GenericComponentIdentifierType"
          manifestIdRef="mpd1" componentIdentifier="video"/>
    </MulticastTransportSession>
  </MulticastSession>
</MulticastGatewayConfiguration>"#;

let config = MulticastGatewayConfiguration::parse_str(xml).unwrap();
assert_eq!(config.sessions.len(), 1);
assert_eq!(config.sessions[0].service_identifier, "urn:example:service:1");

let round_tripped = MulticastGatewayConfiguration::parse_str(&config.to_xml()).unwrap();
assert_eq!(config, round_tripped);
```

Runnable examples: `cargo run -p dvb-mabr --example parse_session` and
`--example build_session`.

## What is implemented

- **Roots:** `MulticastServerConfiguration`, `MulticastGatewayConfiguration`
  (`parse_str` / `to_xml`), `BaselineNamespace`.
- **Sessions:** `MulticastSession`, `PresentationManifestLocator`.
- **Transport:** `MulticastTransportSession`, `TransportProtocol`,
  `EndpointAddress`, `BitRate`, `TransmissionMode`, `ContentIngestMethod`,
  `TransportSecurity`, `ForwardErrorCorrectionParameters`,
  `UnicastRepairParameters`, `BaseUrl`.
- **Components and carousel:** `ServiceComponentIdentifier`, `ObjectCarousel`,
  `InitSegments`, `PresentationManifests`, `ResourceLocator`.
- **Gateway bootstrap:** `MulticastGatewayConfigurationTransportSession`,
  `ConfigurationMacro`.
- **Reporting declarations:** `MulticastGatewaySessionReporting`,
  `ReportingLocator`.
- **Namespace constants:** `NS_MULTICAST_SESSION_CONFIGURATION_2019`, `..._2024`,
  `NS_EXTENSIBILITY_2024`, `NS_XSI`; `Error` / `Result` (structured `Error::XmlParse`).

Field-level spec transcriptions (`mabr-signalling.md`, `mabr-transport.md`, `mabr-reporting.md`) are kept
in the private companion repository, not published; see the
[docs pointer](https://github.com/fishloa/rust-broadcast/blob/main/dvb-mabr/docs/README.md#private-spec-transcriptions).

## Round-trip guarantee

`parse_str -> to_xml -> parse_str` yields an equal document (`PartialEq`). The
serialized XML is not byte-identical to the input:

- Attribute order is fixed by the serializer, not preserved.
- Whitespace is normalized to 2-space indentation.
- Comments and processing instructions are dropped (the XML declaration is
  always emitted).
- Empty elements are always self-closed (`<Foo/>`).
- Extension elements/attributes (Annex A.1) are skipped on parse and not
  re-emitted: a known, documented limitation.
- The `ReferencingObjectCarouselType` vs `ObjectCarouselType` cardinality
  distinction (clause 10.2.5) is not enforced by the type system (see
  `carousel.rs`).

## Fixtures

`fixtures/dvb-mabr/` holds three hand-authored XML documents exercising the
data model (a multi-service server configuration, a gateway bootstrap
document, a full gateway configuration); provenance in `fixtures/PROVENANCE.md`.
They are this crate's own worked examples, not a transcription of the ETSI
Annex C examples (ETSI copyright).

## Design notes

- Single pass over `quick_xml::NsReader` events straight into typed structs (no
  document tree, so hostile nesting cannot overflow the stack); the serializer
  writes through `quick_xml::Writer`, so all attribute and text escaping is
  quick-xml's. A `DOCTYPE`, undefined entity, unbound prefix, second root or
  trailing content is rejected. Guarded by `tests/no_dom_guard.rs`.
- Public enums are `#[non_exhaustive]` with `name()` and `Display`.
- Truncation, XML-escaping and golden tests cover the parser.
- `no_std` only for `error`; everything XML needs `std`.

## Related crates

Depends on `broadcast-common` and `quick-xml`. Complementary: `rmt-flute`
(FLUTE/ALC wire format used by the multicast transport), `transmux` and
`broadcast-hls` (the DASH/HLS manifests this configuration points at),
`multimux` (origin that serves such streams).

## MSRV and licence

Rust 1.95.0, edition 2024. MIT OR Apache-2.0.

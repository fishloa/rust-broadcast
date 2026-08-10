# dvb-mabr

DVB multicast ABR (ETSI TS 103 769 V1.2.1) multicast session configuration
XML parser/serializer.

Parses and serializes the multicast session configuration instance document
(clause 10) — the one XML document format the Multicast server, Multicast
gateway, and rendezvous service are all coordinated by. See
[`docs/mabr-signalling.md`](docs/mabr-signalling.md) for the field-level spec
transcription this crate implements, and [`src/lib.rs`](src/lib.rs) for the
API-level documentation and an example.

## Scope

- Two document roots: `MulticastServerConfiguration` and
  `MulticastGatewayConfiguration` (clause 10.2.1).
- The full `MulticastTransportSession` element tree (clause 10.2.3):
  endpoints, bit rate, FEC parameters, unicast repair, in-band object
  carousel, and DASH/HLS/generic service-component identifiers (clause
  10.2.4).
- `MulticastGatewayConfigurationTransportSession` (clause 10.2.5) for the
  in-band gateway-configuration carousel bootstrap method, including its
  macro-expansion elements.
- Document- and session-level reporting-destination declarations (clause
  10.2.1.0/10.2.2.3). The reporting-report JSON body itself (clause 11) is
  out of scope — see `docs/mabr-reporting.md`.

Presentation manifests (DASH MPD / HLS Master Playlist) are referenced by
URL only, never parsed. The multicast transport of the objects themselves
(FLUTE/ROUTE) is out of scope — see `rmt-flute`.

## Round-trip guarantee

`parse_str → to_xml → parse_str` yields an equal document. The serialized
XML is **not** byte-identical to the input — see the "Round-trip guarantee"
section of the crate-root doc comment (`src/lib.rs`) for the full,
enumerated list of what differs (attribute order, whitespace, extension
elements dropped, etc.).

## Fixtures

`fixtures/dvb-mabr/` (this workspace's shared fixture directory) holds three
hand-authored XML documents exercising this crate's full data model:
a multi-service server configuration, a gateway bootstrap document, and a
full gateway configuration. See `fixtures/PROVENANCE.md` for their
provenance — they are this crate's own worked examples, not a verbatim
transcription of the ETSI TS 103 769 Annex C worked examples (which remain
under ETSI copyright and are not reproduced in this repository).

## MSRV

Rust 1.95.0, edition 2024. `no_std` + `alloc` without the `std` feature.

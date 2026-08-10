# dvb-mabr 0.1.0

**Release date:** 2026-08-10

Initial release. `dvb-mabr` is a DVB multicast ABR (ETSI TS 103 769 V1.2.1) session-configuration XML parser/serializer: it parses and serializes the multicast session configuration instance document (clause 10) — the XML document format the Multicast server, Multicast gateway, and rendezvous service are all coordinated by. `no_std` + `alloc` (without the `std` feature). Not yet published — no `dvb-mabr` version exists on crates.io and no `dvb-mabr-v*` tag exists yet.

## What's in it

- `MulticastServerConfiguration` and `MulticastGatewayConfiguration` — the two document roots (clause 10.2.1), with `parse_str` / `serialize`.
- The full `MulticastTransportSession` element tree (clause 10.2.3): endpoints, bit rate, FEC parameters, unicast repair, in-band object carousel, and DASH/HLS/generic service-component identifiers (clause 10.2.4).
- `MulticastGatewayConfigurationTransportSession` (clause 10.2.5) for the in-band gateway-configuration carousel bootstrap method, including its macro-expansion elements.
- Document- and session-level reporting-destination declarations (clause 10.2.1.0/10.2.2.3).
- Round-trip test: `parse_str → serialize → reparse` yields an equal document (the serialized XML is not byte-identical to the input — attribute order, whitespace, and dropped extension elements differ; see the crate-root doc comment's enumerated list).
- Fixtures in `fixtures/dvb-mabr/`: three hand-authored XML documents (a multi-service server configuration, a gateway bootstrap document, and a full gateway configuration) — this crate's own worked examples, not a transcription of the ETSI TS 103 769 Annex C worked examples, which remain under ETSI copyright.

## Out of scope

- The reporting-report JSON body itself (clause 11) — see `docs/mabr-reporting.md`.
- Presentation manifests (DASH MPD / HLS Master Playlist) are referenced by URL only, never parsed.
- The multicast transport of the objects themselves (FLUTE/ROUTE) — see `rmt-flute`.

This is also why the crate carries no `flute`/`dash` crates.io keywords and no `serde` feature — none of the three exist in this crate; a pre-release doc pass (#940) removed a stale `serde` claim and the out-of-scope keywords before this first release shipped.

## Migration

New crate — no migration needed. MSRV is **1.95.0**.

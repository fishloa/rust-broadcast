# rust-dvb

[![CI](https://github.com/fishloa/rust-dvb/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-dvb/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Spec-grounded DVB protocol parsers and builders in Rust.** Feed a transport
stream in; get typed, decoded, serde-ready data out. Every wire layout is cited
to its ETSI / ISO clause, has a symmetric serializer, and is round-trip tested.

```text
TS (T2-MI PID) ─▶ dvb-t2mi ─▶ BBFrame ─▶ dvb-bbframe ─▶ inner TS ─▶ dvb-si ─▶ typed SI
   T2-MI pump        AnyPayload      Bbheader + up_iter        SiDemux      AnyTableSection + collect
```

Each crate is independently useful; together they decode a DVB-T2 modulator
feed all the way down to a service name string.

## The crates

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`dvb-si`](dvb-si/) | [![crates.io](https://img.shields.io/crates/v/dvb-si.svg)](https://crates.io/crates/dvb-si) | [![docs.rs](https://img.shields.io/docsrs/dvb-si)](https://docs.rs/dvb-si) | ETSI EN 300 468 Service Information + MPEG-2 PSI: every table_id and descriptor, DSM-CC carousel, Annex A text, a version-gated demux. |
| [`dvb-t2mi`](dvb-t2mi/) | [![crates.io](https://img.shields.io/crates/v/dvb-t2mi.svg)](https://crates.io/crates/dvb-t2mi) | [![docs.rs](https://img.shields.io/docsrs/dvb-t2mi)](https://docs.rs/dvb-t2mi) | ETSI TS 102 773 DVB-T2 Modulator Interface (T2-MI): all 12 packet types + a feed-and-iterate pump. |
| [`dvb-bbframe`](dvb-bbframe/) | [![crates.io](https://img.shields.io/crates/v/dvb-bbframe.svg)](https://crates.io/crates/dvb-bbframe) | [![docs.rs](https://img.shields.io/docsrs/dvb-bbframe)](https://docs.rs/dvb-bbframe) | DVB-S2 / S2X / T2 BBFRAME headers (MATYPE/UPL/DFL/SYNCD) + user-packet extraction. |
| [`dvb-scte35`](dvb-scte35/) | [![crates.io](https://img.shields.io/crates/v/dvb-scte35.svg)](https://crates.io/crates/dvb-scte35) | [![docs.rs](https://img.shields.io/docsrs/dvb-scte35)](https://docs.rs/dvb-scte35) | ANSI/SCTE 35 2023r1 splice information (DPI cueing): every command + splice descriptor, the segmentation assignment tables, round-trip builders. |
| [`dvb-common`](dvb-common/) | [![crates.io](https://img.shields.io/crates/v/dvb-common.svg)](https://crates.io/crates/dvb-common) | [![docs.rs](https://img.shields.io/docsrs/dvb-common)](https://docs.rs/dvb-common) | The shared `Parse` / `Serialize` traits and CRC-32/MPEG-2 that everything builds on. |
| [`dvb-tools`](dvb-tools/) | [![crates.io](https://img.shields.io/crates/v/dvb-tools.svg)](https://crates.io/crates/dvb-tools) | [![docs.rs](https://img.shields.io/docsrs/dvb-tools)](https://docs.rs/dvb-tools) | Command-line analyzer over the family: `dump` / `services` / `epg` / `pids` / `t2mi`. |
| [`dvb-conformance`](dvb-conformance/) | [![crates.io](https://img.shields.io/crates/v/dvb-conformance.svg)](https://crates.io/crates/dvb-conformance) | [![docs.rs](https://img.shields.io/docsrs/dvb-conformance)](https://docs.rs/dvb-conformance) | ETSI TR 101 290 stream conformance monitor: Priority-1/2 + SI-repetition indicators on a caller-supplied clock. |
| [`dvb-stream`](dvb-stream/) | [![crates.io](https://img.shields.io/crates/v/dvb-stream.svg)](https://crates.io/crates/dvb-stream) | [![docs.rs](https://img.shields.io/docsrs/dvb-stream)](https://docs.rs/dvb-stream) | Async/tokio stream adapters: `SectionStream` and `T2miEventStream` over any `AsyncRead` source (file, TCP, UDP multicast). **Independently versioned** (tokio MSRV moves faster than the workspace). |

For GSE, see the existing [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## Quickstart

Demux a `.ts` capture and print its SI sections — the
[`dvb-tools dump`](dvb-tools/) CLI:

```console
$ cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts
pid=0x0000 PROGRAM_ASSOCIATION v0 sn=0
pid=0x0064 PROGRAM_MAP v1 sn=0
-- packets=1264 sections=47 emitted=3 suppressed=44 crc_failures=0 malformed=0

$ cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts --json
{
  "pat": {
    "transport_stream_id": 1,
    "entries": [ { "program_number": 1025, "pid": 100 } ]
    // … (other fields elided for brevity)
  }
}
```

In code, the section-level pipeline is a feed-and-match loop:

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;

let mut demux = SiDemux::builder().build();
for packet in ts_packets {                       // each aligned 188-byte packet
    for event in demux.feed(&packet) {           // changed sections only
        if let Ok(AnyTableSection::SdtSection(sdt)) = event.table_section() {
            for service in &sdt.services {
                for item in service.descriptors.iter().flatten() {
                    if let AnyDescriptor::Service(svc) = item {
                        println!("{}", svc.service_name.decode()); // Annex A → UTF-8
                    }
                }
            }
        }
    }
}
```

## DVB System Software Update (SSU) chain

`dvb-si` ships complete end-to-end support for the DVB-SSU receiver chain
(ETSI TS 102 006). Every layer is typed:

```text
NIT linkage_descriptor (type 0x0A)
  └─▶ PMT data_broadcast_id_descriptor (tag 0x66, id = 0x000A)
         └─▶ IdSelector::Ssu → SsuIdSelector  (TS 102 006 §7.1 Table 4)
               UNT (table_id 0x4B) on the signalled PID
                 └─▶ UntPlatform × N (compatibilityDescriptor + descriptors)
                       DSM-CC carousel: DSI (messageId 0x1006) + DII + DDB
                         └─▶ GroupInfoIndication  (TS 102 006 §8.1.1 Table 6)
                               ModuleReassembler → complete firmware module bytes
```

To decode an SSU stream:

1. Parse a `NitSection`; find a `linkage_descriptor` with `linkage_type = 0x0A`
   — it points to the network carrying the UNT.
2. Parse the `PmtSection` for the SSU service; find a `DataBroadcastIdDescriptor`
   with `data_broadcast_id = 0x000A`. Its `id_selector` will be
   `IdSelector::Ssu(SsuIdSelector { oui_entries, … })`.
3. The same PMT ES entry's PID carries UNT sections (`table_id 0x4B`). Parse
   `UntSection`; each `UntPlatform` describes a compatible device group with its
   own `CompatibilityDescriptor` and operational descriptors.
4. Feed the carousel PID into `SiDemux` + `DsmccSection` → `UnMessage::Dsi`.
   Decode `dsi.private_data` as `GroupInfoIndication::parse(dsi.private_data)` to
   find the update groups and their sizes.
5. Parse `UnMessage::Dii` to enumerate modules; feed `DownloadDataBlock` messages
   into `ModuleReassembler` to reconstruct complete firmware bytes.

## Why these crates

These are not "good enough to parse the common case" parsers. The defining
discipline is spec fidelity, verified several ways over:

- **Grounded in the ETSI deliverables.** The PDFs are vendored in the repo and
  their syntax tables transcribed into reviewable markdown under
  [`dvb-si/docs/`](dvb-si/docs/); every module doc cites its spec, section, and
  tag/table_id. No magic numbers — every hex literal outside tests is a named
  constant or enum.
- **Symmetric and round-trip tested — these crates *emit* as well as parse.**
  Every table and descriptor implements `Serialize`, not just `Parse`: build a
  `PatSection` / `PmtSection` / `CaDescriptor` and call `serialize_into` to get a
  complete section (CRC-32 included). Parse → serialize → parse is byte-identical,
  a hard project invariant enforced by tests. So there's no need to hand-roll PSI
  encoders.
- **Decoded, not just typed.** Spec-enumerated codes are typed enums with decoded
  names — `running_status` is a `RunningStatus`, `stream_type` a `StreamType`,
  `service_type` a `ServiceType`; content genre, parental-rating age, AC-3/E-AC-3
  (0x6A/0x7A typed descriptors), and more decode in the library, so consumers
  never re-implement an ETSI lookup table.
- **Five adversarial spec-audit rounds** against the transcriptions, plus
  fixture tests run against **real transponder captures** (e.g. a live French
  TNT / M6 HbbTV mux; a 10 s satellite capture decoding "Emission Spéciale
  Politique" out of an EIT).
- **Complete coverage.** Every allocated `table_id` in EN 300 468 V1.19.1
  Table 2 and every `descriptor_tag` in Table 12; all 12 T2-MI packet types.

## Documentation

- Per-crate front pages: [dvb-si](dvb-si/README.md) · [dvb-t2mi](dvb-t2mi/README.md) · [dvb-bbframe](dvb-bbframe/README.md) · [dvb-common](dvb-common/README.md) · [dvb-tools](dvb-tools/README.md) · [dvb-conformance](dvb-conformance/README.md)
- [Adding a parser crate](docs/extending.md) — how a new sibling crate (e.g. `dvb-scte35`) plugs its own wire types into the existing dispatch via the runtime registries and open `*Def` traits, with zero breaking change.
- [`dvb-si` 4.0 migration guide](dvb-si/MIGRATION-4.0.md) — 3.x → 4.0 breaking changes: section parser names (`NitSection`, `SitSection`, …), `AnyTableSection`, CamelCase `TableId`, and complete multi-section table collection.
- [`dvb-si` 3.1 migration guide](dvb-si/MIGRATION-3.1.md) — 1.x / 2.x → 3.1 breaking changes (typed `DescriptorLoop`, Serialize-only serde, typed SIT, optional `yoke`) with before/after code.
- [`dvb-si` 2.0 migration guide](dvb-si/MIGRATION-2.0.md) — 1.x → 2.0 breaking changes with before/after code.
- API docs: [docs.rs/dvb-si](https://docs.rs/dvb-si) (each crate's docs.rs front page carries a runnable quickstart).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are accepted under the same dual license.

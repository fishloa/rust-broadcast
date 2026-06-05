# rust-dvb

[![CI](https://github.com/fishloa/rust-dvb/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-dvb/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Spec-grounded DVB protocol parsers and builders in Rust.** Feed a transport
stream in; get typed, decoded, serde-ready data out. Every wire layout is cited
to its ETSI / ISO clause, has a symmetric serializer, and is round-trip tested.

```text
TS (T2-MI PID) ─▶ dvb-t2mi ─▶ BBFrame ─▶ dvb-bbframe ─▶ inner TS ─▶ dvb-si ─▶ typed SI
   T2-MI pump        AnyPayload      Bbheader + up_iter        SiDemux      AnyTable / AnyDescriptor
```

Each crate is independently useful; together they decode a DVB-T2 modulator
feed all the way down to a service name string.

## The crates

| Crate | Version | Docs | What it does |
|---|---|---|---|
| [`dvb-si`](dvb-si/) | [![crates.io](https://img.shields.io/crates/v/dvb-si.svg)](https://crates.io/crates/dvb-si) | [![docs.rs](https://img.shields.io/docsrs/dvb-si)](https://docs.rs/dvb-si) | ETSI EN 300 468 Service Information + MPEG-2 PSI: every table_id and descriptor, DSM-CC carousel, Annex A text, a version-gated demux. |
| [`dvb-t2mi`](dvb-t2mi/) | [![crates.io](https://img.shields.io/crates/v/dvb-t2mi.svg)](https://crates.io/crates/dvb-t2mi) | [![docs.rs](https://img.shields.io/docsrs/dvb-t2mi)](https://docs.rs/dvb-t2mi) | ETSI TS 102 773 DVB-T2 Modulator Interface (T2-MI): all 12 packet types + a feed-and-iterate pump. |
| [`dvb-bbframe`](dvb-bbframe/) | [![crates.io](https://img.shields.io/crates/v/dvb-bbframe.svg)](https://crates.io/crates/dvb-bbframe) | [![docs.rs](https://img.shields.io/docsrs/dvb-bbframe)](https://docs.rs/dvb-bbframe) | DVB-S2 / S2X / T2 BBFRAME headers (MATYPE/UPL/DFL/SYNCD) + user-packet extraction. |
| [`dvb-common`](dvb-common/) | [![crates.io](https://img.shields.io/crates/v/dvb-common.svg)](https://crates.io/crates/dvb-common) | [![docs.rs](https://img.shields.io/docsrs/dvb-common)](https://docs.rs/dvb-common) | The shared `Parse` / `Serialize` traits and CRC-32/MPEG-2 that everything builds on. |

For GSE, see the existing [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## Quickstart

Demux a `.ts` capture and print its SI tables — the
[`si_dump`](dvb-si/examples/si_dump.rs) example, in full:

```console
$ cargo run -p dvb-si --example si_dump -- dvb-si/tests/fixtures/m6-single.ts
pid=0x0000 PROGRAM_ASSOCIATION v0 sn=0
pid=0x0064 PROGRAM_MAP v1 sn=0
-- packets=1264 sections=47 emitted=3 suppressed=44 crc_failures=0 malformed=0

$ cargo run -p dvb-si --example si_dump -- dvb-si/tests/fixtures/m6-single.ts --json
{
  "pat": {
    "transport_stream_id": 1,
    "entries": [ { "program_number": 1025, "pid": 100 } ]
    // … (other fields elided for brevity)
  }
}
```

In code, the same pipeline is a feed-and-match loop:

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{parse_loop, AnyDescriptor};
use dvb_si::tables::AnyTable;

let mut demux = SiDemux::builder().build();
for packet in ts_packets {                       // each aligned 188-byte packet
    for event in demux.feed(&packet) {           // changed sections only
        if let Ok(AnyTable::Sdt(sdt)) = event.table() {
            for service in &sdt.services {
                for item in parse_loop(service.descriptors).flatten() {
                    if let AnyDescriptor::Service(svc) = item {
                        println!("{}", svc.service_name.decode()); // Annex A → UTF-8
                    }
                }
            }
        }
    }
}
```

## Why these crates

These are not "good enough to parse the common case" parsers. The defining
discipline is spec fidelity, verified several ways over:

- **Grounded in the ETSI deliverables.** The PDFs are vendored in the repo and
  their syntax tables transcribed into reviewable markdown under
  [`dvb-si/docs/`](dvb-si/docs/); every module doc cites its spec, section, and
  tag/table_id. No magic numbers — every hex literal outside tests is a named
  constant or enum.
- **Symmetric and round-trip tested.** Every parser has a serializer; parse →
  serialize → parse is byte-identical, and this is a hard project invariant
  enforced by tests.
- **Five adversarial spec-audit rounds** against the transcriptions, plus
  fixture tests run against **real transponder captures** (e.g. a live French
  TNT / M6 HbbTV mux; a 10 s satellite capture decoding "Emission Spéciale
  Politique" out of an EIT).
- **Complete coverage.** Every allocated `table_id` in EN 300 468 V1.19.1
  Table 2 and every `descriptor_tag` in Table 12; all 12 T2-MI packet types.

## Documentation

- Per-crate front pages: [dvb-si](dvb-si/README.md) · [dvb-t2mi](dvb-t2mi/README.md) · [dvb-bbframe](dvb-bbframe/README.md) · [dvb-common](dvb-common/README.md)
- [`dvb-si` 2.0 migration guide](dvb-si/MIGRATION-2.0.md) — 1.x → 2.0 breaking changes with before/after code.
- API docs: [docs.rs/dvb-si](https://docs.rs/dvb-si) (each crate's docs.rs front page carries a runnable quickstart).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are accepted under the same dual license.

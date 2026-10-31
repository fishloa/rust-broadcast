# dvb-si

[![crates.io](https://img.shields.io/crates/v/dvb-si.svg)](https://crates.io/crates/dvb-si)
[![docs.rs](https://img.shields.io/docsrs/dvb-si)](https://docs.rs/dvb-si)
[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)

**DVB Service Information and MPEG-2 PSI for Rust — a typed parser _and_ builder for every
table and descriptor a DVB transport stream carries, cited clause-by-clause to the specs.**

`dvb-si` turns the signalling inside an MPEG transport stream — the PAT/PMT that say what
programmes exist, the NIT/SDT/BAT that name networks and services, the EIT that carries the
programme guide, the TDT/TOT clock, the AIT that launches HbbTV apps, the DSM-CC carousels that
deliver files and software updates — into typed Rust values, and turns those values back into
byte-identical wire sections.

## At a glance

<!-- BEGIN GENERATED: dvb-si-at-a-glance (tools/gen-dvbsi-coverage.py) -->

| | |
|---|---|
| Section types (table_id) | **31** typed `*Section` parsers covering **72** table_id values — every allocated EN 300 468 Table 2 id |
| Descriptors (tag 0x00–0x7F) | **116** auto-dispatched: **52** MPEG-2 / DSM-CC (0x02–0x3F) + all **64** DVB tags (0x40–0x7F) |
| DVB extension descriptors (0x7F) | **30** typed `descriptor_tag_extension` bodies — every one EN 300 468 V1.20.1 Table 109 allocates |
| MPEG-2 extension descriptors (0x3F) | **24** typed `extension_descriptor_tag` bodies (7 MPEG-H 3D audio, plus HEVC/VVC/EVC/LCEVC/JPEG XS timing and structure, green, quality, OD update, timed metadata, media service kind) |
| AIT descriptors (TS 102 809 / MHP) | **21** typed in the separate AIT descriptor namespace |
| INT / UNT / RNT descriptors | **26** typed in their own scoped namespaces (`AnyIntDescriptor`, `AnyUntDescriptor`, `AnyRntDescriptor`) |
| Private / regional descriptors | **9** PDS- or registration-scoped opt-ins (NorDig, EACEM/D-Book, China GY/T + T/AI AVS2/AVS3, T/UWA) |
| Text (EN 300 468 Annex A) | **7** charset decoders covering every Table A.3 selector + Annex A.1 control codes |
| DSM-CC carousel | **12** layers: section framing, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel file tree |
| Registries | **171** CA_system_id and **174** private_data_specifier names (TSDuck data, build-time) |
| Standards | **19** standards / families — see the table below |
| Known gaps | **7** allocated codes not yet typed — listed in `private/specs-verified/dvb/dvb-si/COVERAGE.md` (private repo) |

### Standards supported

| Standard | Title | Edition implemented | Scope covered | Status |
|---|---|---|---|---|
| ETSI EN 300 468 | DVB Service Information (SI) | V1.20.1 (2026-09); RFU defaults audited back to V1.3.1 | all SI tables (NIT, BAT, SDT, EIT, TDT, TOT, RST, ST, SAT, DIT, SIT), Table 12 descriptors 0x40–0x7F, Table 109 extension descriptors, Annex A text, Annexes D/G/H/L audio descriptors | Full (98/98 allocated codes) |
| Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 | MPEG-2 Systems — PSI and descriptors | 08/2023 text (clause numbers in COVERAGE.md); MPEG-H 3D audio descriptors 06/2021 | PAT, CAT, PMT, TSDT; Table 2-45 descriptors; Table 2-110 extension descriptors | Partial (82/85 allocated codes) |
| ISO/IEC 13818-6 | DSM-CC (data/object carousel) | as profiled by ETSI TR 101 202 V1.2.1 (13818-6 itself is not vendored) | DSM-CC sections, DSI/DII/DDB, module reassembly, BIOP object carousel, carousel_identifier + association_tag descriptors | Partial (3/5 allocated codes) |
| ETSI EN 301 192 | DVB data broadcasting | V1.7.1 | MPE datagram, MPE-FEC, INT section; time_slice_fec_identifier + ECM_repetition_rate descriptors; DII carousel descriptors | Full (29/29 allocated codes) |
| ETSI TS 102 006 | DVB System Software Update (SSU) | V1.4.1 | UNT section, compatibilityDescriptor, SSU data_broadcast_id selector | Full (16/16 allocated codes) |
| ETSI TS 102 809 | Application signalling (HbbTV AIT) | V1.3.1 | AIT section, AIT descriptor namespace, application_signalling + service_identifier descriptors, protection message section + descriptor | Full (16/16 allocated codes) |
| ETSI TS 102 727 | DVB-MHP 1.1.3 | V1.1.1 (descriptor subset) | DVB-J application + location AIT descriptors, XAIT_location, XAIT_PID | Partial (11/12 allocated codes) |
| ETSI TS 102 323 | TV-Anytime carriage in DVB | V1.4.1 | container, RCT, CIT, RNT sections; default_authority, related_content, TVA_id, content_identifier descriptors | Full (11/11 allocated codes) |
| ETSI TS 102 772 | MPE-IFEC | V1.1.1 | MPE-IFEC section | Full (1/1 allocated codes) |
| ETSI EN 303 560 | DVB TTML subtitling | V1.1.1 | TTML_subtitling descriptor, downloadable font info section (DFIS) | Full (2/2 allocated codes) |
| ETSI TS 102 825-9 | CPCM system adaptation | V1.2.1 | cpcm_delivery_signalling extension descriptor | Full (1/1 allocated codes) |
| ETSI TS 101 154 | Audio/video coding in DVB | V2.10.1 | PMT `StreamType` values incl. AVS2/AVS3 (0xD2–0xD5); AVS private_data_specifier rules | Partial |
| ETSI TS 101 162 | DVB identifiers | registry data from TSDuck (see registries/README.md) | CA_system_id and private_data_specifier value → name lookups | Partial |
| NorDig Unified Requirements | Nordic IRD requirements | v3.2.2 (2025-11) | logical channel descriptor v1 (0x83) + v2 (0x87), PDS 0x00000029 | Full (scoped) |
| EACEM / D-Book | Logical channel numbering | private (PDS 0x00000028); EACEM TR-030 is not vendored | logical_channel descriptor (0x83), 10-bit-LCN layout rests on real captures only; the vendored NorDig spec defines LCD v1 (0x83) and v2 (0x87) in §12.2.9 (both typed in `nordig.rs`) | Full (scoped) |
| GY/T 270-2013 | China DTV closed captions | 2013 | caption_service_descriptor (0x86), `StreamType::ChinaClosedCaption` | Full (scoped) |
| GY/T 420-2025, GY/T 422-2025 | China UHD encapsulation / multi-audio | 2025 | AVS2_video (0x40 under 'AVSV'), AVS3_video; GY/T 422 audio_preselection profile | Full (scoped) |
| T/AI 109.6-2022, T/AI 109.7-2024 | AVS3 media format / audio transport | 2022 / 2024 | AVS3_video (0xD1), AVS3_audio (0xD2), AVS2_audio (0xD3) | Full (scoped) |
| T/UWA 012.2-2023 | UWA UHD TS encapsulation | 2023 | older AVS3 video descriptor (tag 62) | Full (scoped) |

Status: **Full** = every code the edition allocates is typed; **Partial** = some are (the tally counts allocated codes, so a single untyped one shows); **Full (scoped)** = typed, but only decoded inside the private_data_specifier / registration scope the standard requires (opt in via `DescriptorRegistry`); **Not implemented** = bytes are preserved but not typed. Every count above is generated by `tools/gen-dvbsi-coverage.py` from the crate's dispatch lists and CI fails if it goes stale.

<!-- END GENERATED: dvb-si-at-a-glance -->

The full per-code reference — every table_id, descriptor tag and tag extension with its
standard, verified clause, Rust type and status, plus every code the standards define that is
**not** implemented — is [`dvb-si/docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md).

## What it does — and what it does not

**It does**

- Parse and serialize every DVB SI table (NIT, BAT, SDT, EIT p/f + schedule, TDT, TOT, RST, ST,
  SAT, DIT, SIT), the MPEG-2 PSI tables (PAT, CAT, PMT, TSDT), and the DVB companion tables
  (AIT, UNT, INT, MPE / MPE-FEC / MPE-IFEC, container, RCT, CIT, RNT, protection message, DFIS).
- Type every descriptor those tables carry: all of EN 300 468 Table 12, the MPEG-2 systems
  descriptors, every EN 300 468 extension descriptor, the MPEG-H 3D audio descriptors, the AIT
  descriptor namespace, and opt-in regional descriptors (NorDig, EACEM/D-Book, China GY/T and
  T/AI AVS2/AVS3, T/UWA).
- Decode DVB text (EN 300 468 Annex A: default Latin, ISO 8859-n, UCS-2, KS X 1001, GB-2312,
  Big5, UTF-8) and DVB time (MJD + BCD, including the 2038 wrap rule).
- Pump sections out of a TS (`SiDemux`: PID filtering, CRC check, version gating), collect
  multi-section tables (`collect`), build an EPG (`epg::EpgStore`), and reassemble DSM-CC data and
  object carousels into modules and files (`carousel`).

**Who uses it**: TS analysers and monitoring probes, set-top-box and middleware stacks, playout
and multiplexers that must *emit* SI, EPG and channel-list tools, test-stream generators, and
anything that needs to reason about a broadcast stream's signalling.

**It deliberately does not**

- Frame TS packets or reassemble/packetise sections itself — that is
  [`mpeg-ts`](https://crates.io/crates/mpeg-ts), which `dvb-si` uses.
- Touch elementary-stream payloads (video/audio/subtitles) — see
  [`mpeg-pes`](https://crates.io/crates/mpeg-pes), [`dvb-subtitle`](https://crates.io/crates/dvb-subtitle),
  [`dvb-vbi`](https://crates.io/crates/dvb-vbi).
- Descramble, talk to a CAM, or interpret CA-system-private ECM/EMM payloads — see
  [`dvb-csa`](https://crates.io/crates/dvb-csa), [`dvb-ci`](https://crates.io/crates/dvb-ci).
- Measure stream conformance (TR 101 290) — that is
  [`dvb-conformance`](https://crates.io/crates/dvb-conformance).
- Guess. A field whose syntax lives in a standard that is not vendored stays a byte slice and is
  documented as such; codes a standard allocates but the crate does not type are listed as
  **Not implemented** in [`dvb-si/docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md#not-implemented),
  never silently half-decoded.

## Quick start

```toml
[dependencies]
dvb-si = "12"
```

### Parse one section, walk it, serialize it back

The bytes below are the PAT of a real French TNT capture (5°W, 12 732 MHz V) committed in this
repository.

```rust
use broadcast_common::{Parse, Serialize};
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::AnyTableSection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let section: [u8; 36] = [
        0x00, 0xB0, 0x21, 0x00, 0x06, 0xE5, 0x00, 0x00, 0x00, 0x00, 0xE0, 0x10,
        0x06, 0x01, 0xE0, 0x64, 0x06, 0x06, 0xE2, 0x58, 0x06, 0x08, 0xE1, 0xF4,
        0x06, 0x09, 0xE2, 0xBC, 0x06, 0x0A, 0xE0, 0xC8, 0x98, 0xF3, 0xCC, 0x2D,
    ];

    // Typed parse of a section you already know the type of …
    let pat = PatSection::parse(&section)?;
    assert_eq!(pat.transport_stream_id, 6);
    for entry in &pat.entries {
        // program_number 0 points at the NIT, the others at PMTs.
        println!("program {:#06x} -> PID {}", entry.program_number, entry.pid);
    }

    // … or dispatch on table_id when you don't.
    assert!(matches!(AnyTableSection::parse(&section)?, AnyTableSection::PatSection(_)));

    // Every parser has a symmetric serializer: the round trip is byte-identical.
    let mut out = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut out)?;
    assert_eq!(out, section);
    Ok(())
}
```

### Pump a whole capture: services, PMT streams, descriptors

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Any aligned 188-byte TS works; this is the capture committed in this repository.
    let ts = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts"
    ))?;

    // Follows the PAT to every PMT PID; emits each *changed* CRC-valid section once.
    let mut demux = SiDemux::builder().build();
    let mut services = Vec::new();
    for packet in ts.chunks_exact(188) {
        for event in demux.feed(packet) {
            match event.table_section()? {
                AnyTableSection::SdtSection(sdt) => {
                    for service in &sdt.services {
                        for d in service.descriptors.iter().flatten() {
                            if let AnyDescriptor::Service(svc) = d {
                                // Annex A text decodes on demand.
                                services.push(svc.service_name.decode().into_owned());
                            }
                        }
                    }
                }
                AnyTableSection::PmtSection(pmt) => {
                    for stream in &pmt.streams {
                        let tags: Vec<&str> =
                            stream.es_info.iter().flatten().map(|d| d.name()).collect();
                        println!("PID {} {} {:?}", stream.elementary_pid, stream.stream_type, tags);
                    }
                }
                _ => {}
            }
        }
    }
    assert!(!services.is_empty());
    println!("services: {services:?}");
    Ok(())
}
```

More: [`examples/build_and_parse_pat.rs`](examples/build_and_parse_pat.rs),
[`examples/list_services.rs`](examples/list_services.rs) (`cargo run -p dvb-si --example
list_services`), and the [`dvb-tools`](https://crates.io/crates/dvb-tools) CLI
(`dvb-tools dump file.ts --json`) for a complete analyser built on this crate.

## Which API do I use?

| You have … | Use | Notes |
|---|---|---|
| a TS file / UDP feed / tuner | [`demux::SiDemux`](https://docs.rs/dvb-si/latest/dvb_si/demux/struct.SiDemux.html) → `SectionEvent::table_section()` | feature `ts`; CRC-checked, version-gated, follows the PAT |
| complete section bytes of unknown type | `tables::AnyTableSection::parse` | `AnyTableSection::parse_with(&TableRegistry, …)` adds private table_ids |
| complete section bytes of a known type | `tables::<table>::<Name>Section::parse` | e.g. `tables::sdt::SdtSection::parse` |
| a table that spans several sections | `collect::SectionSetCollector` (`.nit()`, `.bat()`, `.sdt()`, `.eit()`, `.table::<T>()`) | `collect::EitCollector` for EIT schedules (spans table_ids) |
| a programme guide to query | `epg::EpgStore` (`now_and_next`, `schedule`) | feature `chrono` |
| a descriptor loop (`DescriptorLoop`) | `.iter()` → `AnyDescriptor` | unknown tags surface as `AnyDescriptor::Unknown`, never panic |
| private / regional descriptors | `descriptors::DescriptorRegistry` + `.iter_with(&reg)` | PDS-scoped: NorDig LCN, EACEM LCN, AVS2/AVS3, China captions, your own |
| private extension descriptors (0x7F) | `descriptors::ExtensionRegistry` + `.iter_with_extensions` | |
| an AIT | `tables::ait::AitSection` + `descriptors::ait::parse_ait_loop` | the AIT has its own descriptor namespace |
| a DSM-CC carousel | `carousel::{Dsi, Dii, DownloadDataBlock, ModuleReassembler}` → `carousel::CarouselFs` | `flate2` feature for compressed modules |
| a value to emit | build the struct (`new(..)`), then `Serialize::serialize_into` | then [`mpeg-ts`](https://crates.io/crates/mpeg-ts) packetises it |
| JSON for a UI / log | the `serde` feature (`serde_json::to_string(&section)`) | serialize-only: re-parse from wire bytes, never from JSON |
| a parsed view to keep (cache, other thread) | `owned::Owned<T>` | feature `yoke`; no re-parse, `Send + Sync + 'static` |

### Private descriptors in their scope

```rust
use dvb_si::descriptors::nordig::NordigLogicalChannelV1;
use dvb_si::descriptors::{AnyDescriptor, DescriptorLoop, DescriptorRegistry};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // private_data_specifier = 0x00000029 (NorDig), then a NorDig LCD v1 (tag 0x83):
    // service 0x0001, visible, LCN 5 (NorDig Unified Requirements v3.2.2 Table 12.12).
    let loop_bytes = [0x5F, 0x04, 0x00, 0x00, 0x00, 0x29, 0x83, 0x04, 0x00, 0x01, 0xC0, 0x05];
    let descriptors = DescriptorLoop::new(&loop_bytes);

    // Without opting in, tag 0x83 stays Unknown (it is private) …
    let plain: Vec<_> = descriptors.iter().collect::<Result<_, _>>()?;
    assert!(matches!(plain[1], AnyDescriptor::Unknown { tag: 0x83, .. }));

    // … with the NorDig registry it decodes, because the PDS in front scopes it.
    let mut registry = DescriptorRegistry::new();
    registry.with_nordig_lcn();
    for d in descriptors.iter_with(&registry) {
        if let AnyDescriptor::Other { value, .. } = d? {
            let lcd = value.downcast_ref::<NordigLogicalChannelV1>().ok_or("not NorDig")?;
            assert_eq!(lcd.entries[0].logical_channel_number, 5);
        }
    }
    Ok(())
}
```

## Feature flags

| Feature | Default | Enables |
|---|---|---|
| `std` | yes | Links `std`. Off → `#![no_std]` + `alloc`. The non-Latin text charsets (`encoding_rs`) and wall-clock time need it. |
| `chrono` | yes | MJD + BCD fields also as `chrono::DateTime<Utc>` (EIT `start_time_chrono()`, TDT/TOT `utc_time()`), local time offsets as `chrono::Duration`; the `epg` module. Off → `MjdBcdDateTime` and raw bytes only. |
| `ts` | yes | `demux::SiDemux` over 188-byte packets (framing from [`mpeg-ts`](https://crates.io/crates/mpeg-ts)). Off → bring your own section bytes. |
| `serde` | yes | `Serialize` on every table, descriptor and text type (`DvbText` as its decoded string). Serialize-only by design. |
| `yoke` | no | `Yokeable` on the zero-copy views + `owned::Owned<T>`. |
| `flate2` | no | zlib-decompress `compressed_module_descriptor` modules in object carousels (implies `std`). |

```toml
dvb-si = { version = "12", default-features = false }   # no_std + alloc, bring your own sections
```

## Detailed coverage

The generated tables above summarise; the per-code lists live in
[`dvb-si/docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md):

- **Tables** — every table_id with standard, verified clause, Rust type and status.
- **Descriptors** — MPEG-2 (0x00–0x3F), DVB (0x40–0x7F), DVB extension (0x7F / tag_extension),
  MPEG-2 extension (0x3F / extension_descriptor_tag), private/regional (with their PDS scope and
  `DescriptorRegistry` opt-in), AIT, INT, UNT, DSM-CC carousel and RNT-scoped descriptors.
- **DSM-CC carousel**, **Annex A text** charsets, **time** handling and the higher layers.
- **Not implemented** — every code the vendored standards allocate that `dvb-si` does not type.
- **Citation cross-check** — module docs whose clause number disagrees with the verified edition.

Every count and row is generated by [`tools/gen-dvbsi-coverage.py`](https://github.com/fishloa/rust-broadcast/blob/main/tools/gen-dvbsi-coverage.py)
from the crate's dispatch lists (`declare_tables!`, `declare_descriptors!`,
`declare_extension_bodies!`, `declare_ait_descriptors!`) and module citations, joined with the
hand-transcribed allocation lists in [`dvb-si/docs/coverage/allocations.tsv`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/coverage/allocations.tsv).
The `docs_coverage_drift` test fails CI when any of it is stale. The spec transcriptions behind
every layout are indexed in [`docs/README.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/README.md).

## Design notes

- **Zero-copy parse, symmetric serialize.** Every wire structure implements
  `broadcast_common::Parse<'a>` (borrowing from the input — descriptor loops, names and selector
  tails are `&'a [u8]` / `DescriptorLoop<'a>` / `DvbText<'a>`) and `broadcast_common::Serialize`
  (`serialized_len` + `serialize_into`). Every type has a round-trip test (parse → serialize →
  byte-identical; serialize → parse → equal), and `tests/real_capture_byte_identity.rs`
  re-serializes every descriptor found in the real-capture archive and requires byte identity.
- **Reserved bits: parse tolerant, serialize preserving.** Any value of a `reserved` /
  `reserved_future_use` / `reserved_zero_future_use` run is accepted on parse, captured in a
  typed, spec-named field, and re-emitted verbatim — so a sender that deviates from the edition
  still round-trips byte-identically, with no raw passthrough blob. Values built with `new(..)` /
  `Default` carry the governing edition's fill (ones for `reserved`, zeros for
  `reserved_zero_future_use`, documented per-field exceptions). Evidence per field:
  [`dvb-si/docs/reserved-bits-edition-audit.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/reserved-bits-edition-audit.md).
- **Checked serializers.** A value wider than its wire field is `Error::FieldOverflow`, never
  masked into a different, valid-looking value. PIDs are `mpeg_ts::pid::Pid`.
- **One dispatch list per namespace.** `AnyTableSection`, `AnyDescriptor`, `ExtensionBody` and
  `AnyAitDescriptor` are generated from a single declarative list (`declare_*!`), with a drift test
  pinning each code to its type's `TABLE_ID_RANGES` / `TAG` / `TAG_EXTENSION` const — the
  dispatcher cannot drift from the implemented set, and neither can these docs.
- **Labels.** Every spec/field enum has `name()` (the spec token, `"reserved"` for reserved arms)
  and a `Display` generated by `broadcast_common::impl_spec_display!`; a `label_coverage` test
  enforces it.
- **`#[non_exhaustive]`** on public structs and enums that can grow, with `new(..)` constructors,
  so additions are not breaking.
- **`no_std` + `alloc`** with default features off; never panics on hostile input (fuzzed; bounded
  collectors and EPG store).
- **Spec grounding.** Every module's `//!` doc names the standard, clause and tag; every hex
  literal outside tests is a named constant.

## Related crates

| Crate | Relationship |
|---|---|
| [`broadcast-common`](https://crates.io/crates/broadcast-common) | the `Parse` / `Serialize` traits, CRC-32/MPEG-2, MJD/BCD time, `impl_spec_display!` |
| [`mpeg-ts`](https://crates.io/crates/mpeg-ts) | TS packets, section reassembly + packetisation, PIDs — `SiDemux` sits on it, and it turns serialized sections back into packets |
| [`mpeg-pes`](https://crates.io/crates/mpeg-pes) | PES depacketisation for the elementary streams the PMT points at |
| [`dvb-t2mi`](https://crates.io/crates/dvb-t2mi), [`dvb-bbframe`](https://crates.io/crates/dvb-bbframe) | the layers *below* the TS on DVB-T2 / S2 / S2X (lockstep releases with `dvb-si`) |
| [`dvb-conformance`](https://crates.io/crates/dvb-conformance) | ETSI TR 101 290 monitoring; uses `dvb-si` for its SI checks |
| [`dvb-tools`](https://crates.io/crates/dvb-tools) | CLI analyser: `dump` / `services` / `epg` / `pids` / `t2mi` |
| [`dvb-ci`](https://crates.io/crates/dvb-ci) | EN 50221 Common Interface; builds CA_PMT from a `dvb-si` PMT |
| [`scte35-splice`](https://crates.io/crates/scte35-splice) | SCTE 35 splice_info sections (ad cueing), carried alongside SI |
| `dvb-si-py` (`bindings/python/`) | Python bindings: `parse_section(bytes) -> dict`, `Demux` |

## Upgrading

[`MIGRATION-4.0.md`](MIGRATION-4.0.md) (3.x → 4.0: `*Section` names, `AnyTableSection`,
`table_section()`, multi-section collection), [`MIGRATION-3.1.md`](MIGRATION-3.1.md),
[`MIGRATION-2.0.md`](MIGRATION-2.0.md); per-release detail in [`CHANGELOG.md`](CHANGELOG.md).

## MSRV and licence

MSRV **1.95.0** (the workspace `rust-version`; CI tests it). Licensed under either of MIT or
Apache-2.0, at your option. The vendored TSDuck registry data in `registries/` is BSD-2-Clause
(see [`registries/README.md`](registries/README.md)).

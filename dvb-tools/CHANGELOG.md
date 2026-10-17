# Changelog

## [Unreleased]

### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `dvb-t2mi` 11 -> 12, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

### Fixed

- `t2mi` (event dump and `--inner`) treats a closed stdout pipe (`| head`) as a clean stop: it returns immediately with success instead of continuing through the capture and failing on the final flush, and the event dump no longer panics inside `println!` (r11-W-DT-3, #1100).
- Every subcommand now streams the capture in 1 MiB reads through the stateful TS resynchroniser instead of reading the whole file and then building a second full-size `Vec` of packets: peak memory on a 152 MB capture is 19 MB (r11-W-DT-4). A mid-file read error is reported and exits non-zero.

- `t2mi --inner` without `--plp` on a non-seekable input (a pipe via `/dev/stdin`, a FIFO, process substitution) no longer writes zero packets and exits 0: the PLP-discovery pass added by #1034 consumed the stream before the real pass. A non-regular input is now recovered in a single pass (output buffered in memory, written only if exactly one PLP was seen; several PLPs still exit non-zero listing them). Regular files keep the constant-memory two-pass scan.

### Changed

- **BREAKING (#1034):** `t2mi --inner` without `--plp` now scans the capture first and, when it carries more than one PLP, exits non-zero listing the PLPs and asking for `--plp N` — each PLP is an independent transport stream, and the old behaviour interleaved them into one garbled mux on stdout. A single-PLP capture needs no `--plp`.
- `t2mi --pid` rejects a PID above `0x1FFF` as a usage error (was accepted and matched nothing). `dump --json` output grows (found by the differential sweep, docs/DIFFERENTIAL-SWEEP-2026-10.md; e.g. `fixtures/ts/m6-single.ts` 9715 -> 9895 bytes): descriptors and tables now carry their reserved bits as typed `reserved_future_use*` / `reserved*` keys, list-bearing structures carry `"trailing": []`, and descriptor tag 0x3F (`extension_descriptor`) decodes to a typed `Mpeg2Extension` body instead of `unknown`. Existing keys keep their names and values; a consumer that rejects unknown keys must be updated. `services`, `epg`, `pids` and `t2mi` output is byte-identical except where `t2mi` now parses individual-addressing packets it previously failed on (dvb-t2mi function_length framing fix).

## [11.0.1] - 2026-10-11
### Changed
- Version bump with the lockstep (11.0.1); no functional changes in this crate.

## [11.0.0] - 2026-10-05

Lockstep minor alongside `dvb-si` 10.1.0; no source changes in this crate.

### Fixed
- `pids` bitrate estimate: pick one PCR PID and use only its own first/last
  PCR readings, instead of mixing the first PCR seen on any PID with the
  last PCR seen on any (possibly different) PID — a multiplex carries one
  independent PCR clock per program (ISO/IEC 13818-1 §2.4.4.9), so comparing
  two different PCR PIDs' timestamps is meaningless and previously reported
  "n/a" on any multi-program capture with more than one PCR PID (issue #1100).
  Also unwrap the PCR's own ~26.5h rollover (`2^33 * 300` 27 MHz ticks,
  ISO/IEC 13818-1 §2.4.2.2/§2.4.3.5) across samples on the selected PID, so
  a capture whose PCR crosses that wrap no longer reports "n/a" either.
- `services`: key services and logical channel numbers by
  `(original_network_id, transport_stream_id, service_id)` instead of
  `service_id` alone — `service_id` is only unique within one transport
  stream's namespace (ETSI EN 300 468 §5.2.2/§5.2.3), so a numerically
  identical `service_id` on a second transport stream (SDT-other, or a
  second TS entry in the NIT) previously overwrote the first service and
  could be shown with the wrong LCN (issue #1100).

## [10.0.1] - 2026-08-30
Lockstep patch alongside `dvb-si` 10.0.1; no source changes in this crate.

## [10.0.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

## [9.2.0] - 2026-08-05
Lockstep minor alongside `dvb-conformance` 9.2.0; no functional changes of
this crate's own.

## [9.1.1] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [9.1.0] - 2026-07-30
Lockstep release alongside `dvb-conformance` 9.1.0; no functional changes.

## [9.0.0] - 2026-07-27
### Changed (Breaking)
- Lockstep major bump alongside `broadcast-common` 9.0.0, whose `Encrypt::encrypt`
  now takes `&mut self` instead of `&self` (needed so a stateful implementor can
  own a running per-key IV counter — see `broadcast-common`'s own CHANGELOG for
  the full rationale and the migration note for external `impl Encrypt`s).
  `dvb-tools` itself does not implement `Encrypt`/`Decrypt` and has no functional
  or public API change of its own in this release.

## [8.5.0] - 2026-07-21
### Changed
- Widen the internal `mpeg-ts` dependency to `0.3` (issue #663; private
  dependency — no public API change to `dvb-tools`).

## [8.4.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [8.3.0] - 2026-07-03
### Changed
- Lockstep release with the DVB core crates; no functional changes to `dvb-tools`.

## [8.2.1] — 2026-07-02

### Changed
- Lockstep release tracking `broadcast-common` 8.2.1 (mux-trait documentation). No API/behaviour change to this crate.

## [8.2.0] — 2026-07-02

### Changed
- Lockstep release tracking `broadcast-common` 8.2.0 (new `mux` container-mux
  traits). No API or behavioural change to this crate.

## [8.1.0] — 2026-06-29

### Changed
- Lockstep release. Internal dependency `dvb-common` renamed to `broadcast-common`; no API change.

## [8.0.0] — 2026-06-27

### Changed
- CLI rebuilt on `clap` (derive) per the workspace CLI standard
  (`docs/CLI-STANDARD.md`): auto-generated `--help`/`--version`, per-subcommand
  `--help`, and proper argument validation. Subcommands take a positional
  `<FILE>` plus named flags (`--json`, `--pid`, `--inner`, `--plp`).

## [7.9.0] — 2026-06-22

### Changed

- Lockstep release; no functional changes to this crate.

## [7.8.0] — 2026-06-21

### Added
- `dump` human mode now decodes and prints the descriptor loops of PMT
  (program-info + per-ES), SDT (per-service), and NIT (network + per-TS).
  Descriptors resolve through a registry with the PDS-scoped 0x83
  logical_channel enabled for the EACEM/NorDig `private_data_specifier`s, so
  LCNs show as `LOGICAL_CHANNEL` instead of `UNKNOWN(0x83)`.

## [7.7.1] — 2026-06-21

### Changed
- Lockstep release; no functional changes to this crate.

## [7.7.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.6.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.5.0] — 2026-06-19

### Changed
- Lockstep release; no functional changes to this crate.

## [7.4.0] — 2026-06-18

Lockstep release; no functional changes.

## [7.3.0] — 2026-06-17

### Changed
- Lockstep release; no functional changes to this crate.

## [7.2.0] — 2026-06-16

### Fixed
- `services` now resolves logical channel numbers: the NIT walk uses a
  `DescriptorRegistry` with PDS-scoped logical_channel enabled for EACEM/NorDig,
  so 0x83 decodes as `LogicalChannel` instead of `Unknown` (#211).

## [7.1.0] — 2026-06-15

### Changed
- Lockstep release; rebuilt against the dvb-* parser-hardening pass (#207). No
  functional changes.

## [7.0.0] — 2026-06-14

**BREAKING (MSRV 1.75 → 1.81).**

### Changed
- MSRV **1.81**; tracks the breaking library changes (BIOP typing, typed coded
  fields, `#[non_exhaustive]` enums). Stays a std-only application.

## [6.7.0] — 2026-06-14

### Changed
- Lockstep release with the library crates; no functional changes (consumes the
  #204 `name()`/`Display` label convention).

## [6.6.0] — 2026-06-14

Version-lockstep release with the workspace (dvb-t2mi L1-pre/L1-post signalling parser #54; dvb-si BIOP object-carousel layer #64; criterion benchmark suites #62). No changes to this crate.

## [6.5.0] — 2026-06-13

Version-lockstep release with the workspace (#47 T2 emission-time accessors; #50 SSU GroupInfoIndication + data_broadcast_id 0x000A selector; #53 S2Xv2 0x24 extension descriptor). No changes to this crate.

## [6.4.0] — 2026-06-13

Version-lockstep release with the workspace (#158 spec-table drift-guards + spec-fidelity audit; dvb-si PMT section/last-section fields; dvb-bbframe DVB-S2 BUFSTAT ISSY decode). No changes to this crate.

## [6.3.0] — 2026-06-13

### Changed
- `for_each_packet` now drives the shared `dvb_si::ts::TsResync` (188/204-byte
  resync helper) instead of ad-hoc 188-byte chunking (#61); behaviour for
  aligned input is unchanged.

## [6.2.0] — 2026-06-13

### Added
- **`dvb-tools`** (#59) — new published binary crate over the `rust-dvb`
  family. Five subcommands, all driven over aligned 188-byte `.ts` captures
  (no new dependencies — argument parsing stays on `std::env::args`):
  - `dump [--json]` — SI section dump (`SiDemux`-driven, ported from the old
    `si_dump` example).
  - `services` — SDT + NIT service tree with LCNs
    (`SectionSetCollector` + `CompleteSdt`/`CompleteNit`).
  - `epg [--json]` — EPG schedule via `EpgStore`; EIT events with service
    names attached from the SDT (`feed_sdt`).
  - `pids` — per-PID packet counts, sorted by descending packet count,
    with bitrate estimated from the first/last PCR observed.
  - `t2mi [--pid 0xNNN|raw] [--inner] [--plp N]` — T2-MI pump
    (ported from the old `t2mi_dump` example); with `--inner`, chain-unwrap
    to the inner MPEG-TS via `InnerTsRecovery` and write the recovered
    188-byte packets to stdout. `--plp` targets one baseband frame's PLP.

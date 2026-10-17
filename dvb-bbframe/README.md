# dvb-bbframe

[![crates.io](https://img.shields.io/crates/v/dvb-bbframe.svg)](https://crates.io/crates/dvb-bbframe)
[![docs.rs](https://img.shields.io/docsrs/dvb-bbframe)](https://docs.rs/dvb-bbframe)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#related-crates-and-license)

**DVB-S2 / S2X / T2 Base-Band Frame (BBFRAME) header parser and builder, ISSY decoding, and Transport Stream user-packet extraction in Normal and High Efficiency Mode.**

## What it does / what it does not

A BBFRAME is the unit of payload in a DVB-S2/S2X/T2 baseband stream: a 10-byte
BBHEADER followed by a data field of user packets (a Transport Stream, or a
generic stream). This crate parses and rebuilds the BBHEADER (including the
CRC-8-recovered Normal/High-Efficiency mode), decodes the ISSY field, and walks
the data field to recover 188-byte inner TS packets, with per-PLP carry-over
across frame boundaries. It is what `dvb-t2mi` chains into for inner-TS
recovery, and what you use to analyse BBFrame captures from a DVB-S2 receiver
or a T2 gateway.

It does not implement the physical layer (LDPC/BCH coding, modulation), does not
parse GSE (Generic Stream Encapsulation: hand the data field to the third-party
[`dvb-gse`](https://crates.io/crates/dvb-gse) crate), and does not reinsert
deleted null packets: an HEM frame with `MATYPE.NPD` set yields **no output**
for that frame and increments `CarryOverStats::npd_unsupported`. NPD is common
on real DVB-S2 streams, so check that counter before assuming full coverage.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI EN 302 755, *DVB-T2 frame structure, channel coding and modulation* | V1.4.1 (vendored; verified) | §5.1.6 per-UP CRC-8; §5.1.7 BBHEADER and High Efficiency Mode; §5.1.8 user-packet carriage (stride derived from `UPL`/ISSYI/NPD); Annex C ISSY; Annex F CRC-8 | Full for the header and TS user packets; Partial for HEM | HEM with NPD set is skipped (see above). |
| ETSI EN 302 307-1, *DVB-S2* | V1.4.1 (vendored; verified) | §5.1.4 BBHEADER and CRC-8; Annex D ISSY | Full for the header and ISSY | Same BBHEADER layout as T2 for NM. |
| ETSI EN 302 307-2, *DVB-S2X* | V1.4.1 (vendored; verified) | The shared BBHEADER; MATYPE roll-off `0b11` is exposed as `RollOff::Reserved` (reserved / S2X low roll-off). | Partial: header fields only | S2X-specific content (MODCOD, constellations, VL-SNR) is transcribed in `docs/` but not implemented; the S2X-extended BBHEADER semantics were not audited against the vendored text. |
| ISO/IEC 13818-1 §2.4.3.2 | H.222.0 08/2023 | Transport Error Indicator set on a packet whose NM CRC-8 check failed | Used | |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Links `std`; off gives `#![no_std]` + `alloc` |
| `serde` | no | `serde::Serialize` (serialize-only) on `Bbheader`, `Matype`, `TsGs`, `Mode`, `Issy`, `SignallingKind`, `BufsUnit`, `RollOff` |
| `yoke` | no | `yoke::Yokeable` on `NmTsIter` and `HemTsIter` |

```toml
[dependencies]
dvb-bbframe = "12"
```

## Quick start

```rust
use dvb_bbframe::header::{BBHEADER_LEN, Bbheader, Matype, Mode, TsGs};
use dvb_bbframe::packet::nm_stride_bytes;

let hdr = Bbheader {
    matype: Matype { ts_gs: TsGs::Ts, sis: true, ccm: true, issyi: false, npd: false, ext: 0, isi: 0 },
    upl: 1504, sync: 0x47, dfl: 1504, syncd: 0, mode: Mode::Normal, issy_in_header: None,
};
let bytes = hdr.serialize()?;                       // 10-byte BBHEADER with CRC-8
assert_eq!(bytes.len(), BBHEADER_LEN);
assert_eq!(Bbheader::parse(&bytes)?, hdr);          // byte-exact round trip
assert_eq!(nm_stride_bytes(&hdr), Some(188));       // UPL 1504 bits = 188 bytes per UP
# Ok::<(), dvb_bbframe::Error>(())
```

Per-PLP inner-TS extraction from `(plp_id, data_field)` pairs:

```rust,no_run
use dvb_bbframe::pump::BbframePump;
# let frames: Vec<(u8, Vec<u8>)> = vec![];
let mut pump = BbframePump::new();
for (plp_id, bbframe) in &frames {                  // each: BBHEADER + data field
    for ts_packet in pump.feed(*plp_id, bbframe) {  // 188-byte inner TS packets, sync restored
        assert_eq!(ts_packet[0], 0x47);
    }
}
```

When `matype.ts_gs == TsGs::Gse`, hand `&frame[BBHEADER_LEN..]` to `dvb-gse`.
For a T2-MI stream, `dvb_t2mi::inner_ts::InnerTsRecovery` drives this crate for
you. Examples: `cargo run -p dvb-bbframe --example parse_bbheader` and
`--example walk_capture` (reassemble and parse every BBHEADER of the committed
`tnt-5w-12732v-bbframe.ts` capture).

## What is implemented

| Area | Public API |
|---|---|
| BBHEADER | `header::{Bbheader, Matype, TsGs, Mode, RollOff, BBHEADER_LEN, DFL_MAX_BITS}`; `Bbheader::{parse, serialize, issy}`. Fields: `matype` (`ts_gs`, `sis`, `ccm`, `issyi`, `npd`, `ext`, `isi`), `upl`, `sync`, `dfl`, `syncd`, `mode` (recovered via `crc8(hdr[0..9]) ^ hdr[9]`), `issy_in_header` (HEM) |
| CRC-8 | `crc::{crc8, CRC8_POLY, CRC8_INIT}` |
| ISSY | `issy::{Issy, SignallingKind, BufsUnit, decode_issy_short, decode_issy_long}`: ISCR short (15 bit), ISCR long (22 bit), signalling (`Bufs`, `Tto`, `BufStat`, `Reserved`); `bufs_bits()`/`bufs_bytes()`, `bufstat_bits()`/`bufstat_bytes()`, `tto_t_over_256()` |
| User packets | `packet::{nm_stride_bytes, NmTsIter, HemTsIter, UpIter, up_iter, CarryOverExtractor, CarryOverStats, NM_UP_SIZE, HEM_UP_SIZE, TS_SYNC_BYTE}`; `CarryOverExtractor::{feed_nm, feed_hem, feed_nm_into, feed_hem_into}`; `SYNCD = 0xFFFF` handled |
| Multi-PLP | `pump::{BbframePump, BbframePumpStats}`: independent per-PLP carry-over state, heap-allocated (256 slots) |
| Errors | `Error` (including `InvalidStride`), `Result` |

`CarryOverStats` counters make every skip observable: `npd_unsupported`,
`header_parse_failures`, `mode_mismatches`, `partial_discards`, `nm_upl_invalid`
and `crc8_mismatches` (a CRC-8 mismatch still emits the packet, flagged with the
Transport Error Indicator).

## Design notes

- **Symmetric, checked serializer.** `Bbheader::serialize` returns `Result`:
  it rejects `dfl > DFL_MAX_BITS` and mode inconsistencies (NM cannot carry
  `issy_in_header`; HEM cannot carry non-zero `upl`/`sync`) rather than writing
  a header the parser refuses.
- **Stride from `UPL`, not a constant.** NM user packets are framed at the
  stride `UPL`/ISSYI/NPD imply (an off-air ISSYI=1 capture has a 190-byte
  stride); an invalid `UPL` yields nothing rather than mis-framing.
- **RFU policy.** `reserved_future_use` bits are emitted as 1 and
  `reserved_zero_future_use` bits as 0 (DVB convention); parsers accept any
  value for forward compatibility.
- **Real captures.** `tests/integration_fixture.rs` runs against the committed
  DVB-S2 captures (`rai-5w-12606v-bbframe.ts`, `tnt-5w-12732v-bbframe.ts`).
- Public enums carry `name()` and `Display`; `#![warn(missing_docs)]`;
  `no_std` + `alloc`.

## Related crates and license

Depends on `broadcast-common`. Consumed by `dvb-t2mi` (T2-MI inner-TS
recovery). Lockstep version with `dvb-si`, `dvb-t2mi`,
`dvb-conformance`, `dvb-tools`. Spec transcriptions:
[`docs/`](https://github.com/fishloa/rust-broadcast/tree/main/dvb-bbframe/docs).

MSRV 1.95.0. Licensed under [MIT](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-MIT) or
[Apache-2.0](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-APACHE), at your option.

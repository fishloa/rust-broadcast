# mpeg-ps

[![Crates.io](https://img.shields.io/crates/v/mpeg-ps.svg)](https://crates.io/crates/mpeg-ps)
[![docs.rs](https://img.shields.io/docsrs/mpeg-ps)](https://docs.rs/mpeg-ps)

**MPEG-2 Program Stream framing: pack header, system header, program stream map, with PES parsed through `mpeg-pes`.**

## What it does / what it does not

Program Streams are the `.mpg` / `.vob` / DVD container: a sequence of *packs*,
each opened by a pack header (42-bit SCR + `program_mux_rate`), optionally
carrying a system header and a program stream map (PSM), followed by PES
packets. This crate parses and serializes that framing and hands each PES
packet to `mpeg-pes`. It is for demuxers, analyzers and container-detection
code.

It does not decode the elementary streams, does not mux a Program Stream from
scratch (it round-trips parsed structures), and does not support MPEG-1 System
Streams: an ISO/IEC 11172-1 pack (`'0010'` prefix) is recognised and rejected
with `Error::Mpeg1NotSupported`, never mis-parsed as corrupt data.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1, *Systems* | H.222.0 v9 (08/2023), verified against the vendored copy | §2.5 Program Stream: pack header (§2.5.3.3), system header (Table 2-40, incl. per-stream P-STD buffer bounds and `stream_id_extension`), Program Stream Map (§2.5.4, Table 2-41, CRC-32), `MPEG_program_end_code`; PES via `mpeg-pes` | Full for the listed syntax | The Program Stream Directory is not implemented; PSM `program_stream_info` descriptors are carried as opaque bytes. |
| ISO/IEC 11172-1 (MPEG-1 Systems) | not vendored (unverified) | Detection of the MPEG-1 pack layout only | Not implemented | Reported as `Error::Mpeg1NotSupported`. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Links `std`; off gives `#![no_std]` + `alloc` |
| `serde` | no | `serde::Serialize` derives on the public types |

### Streaming

`mpeg_ps::PsScanner` is the sans-IO sibling of `program_stream::scan_packs`:
feed it bytes as they arrive and poll one structural unit at a time (a pack
header, a system header, a program stream map, or a complete PES packet). It
holds at most one incomplete unit, bounds a PES at `PsLimits::max_pes_bytes`,
and resynchronises on the next `pack_start_code` after damage, reporting the
skipped byte count.

```rust
use mpeg_ps::{PsEvent, PsLimits, PsScanner};

let bytes = [0x00, 0x00, 0x01, 0xB9];
let mut scanner = PsScanner::new(PsLimits::new());
scanner.feed(&bytes);
while let Ok(Some(event)) = scanner.poll() {
    if let PsEvent::Pes { stream_id, packet } = event {
        println!("PES stream_id {stream_id:#04X}, {} bytes", packet.len());
    }
}
scanner.finish();
```

```toml
[dependencies]
mpeg-ps = "0.10"
```

## Quick start

```rust
use broadcast_common::Parse;
use mpeg_ps::{PackHeader, program_stream};

// A minimal pack header: start_code 0x000001BA, SCR = 0, mux_rate = 3, no stuffing.
let bytes = [
    0x00, 0x00, 0x01, 0xBA,
    0x44, 0x00, 0x04, 0x00, 0x04, 0x01,
    0x00, 0x00, 0x0F, 0x00,
];
let h = PackHeader::parse(&bytes)?;
assert_eq!(h.program_mux_rate, 3);
assert_eq!(h.scr.ticks(), 0);

// Walk a real capture (committed fixture; path is relative to the crate root).
let data = std::fs::read("../fixtures/ps/ffmpeg-mpeg2-ps.mpg").unwrap();
let scan = program_stream::scan_packs(&data);
assert!(!scan.packs.is_empty());
assert!(scan.skipped.is_empty());
for pack in &scan.packs {
    let _ = (pack.pack_header.scr.ticks(), pack.pes_packets.len());
}
# Ok::<(), mpeg_ps::Error>(())
```

`program_stream::scan_packs` is the one walker. It returns a `PackScan`:
every pack that parsed (`packs`), every span it had to skip together with the
error (`skipped`, resynchronising on the next `pack_start_code`), and the
`remaining` tail; a damaged pack never discards the packs parsed before it.

Examples: `cargo run -p mpeg-ps --example parse_pack_header` and
`--example walk_ps`.

## What is implemented

| Area | Public API |
|---|---|
| Pack header | `PackHeader<'a>` (`Parse`/`Serialize`; `header_len`), `Scr` (42-bit @ 27 MHz; `ticks`, `seconds`), `PACK_START_CODE` |
| System header | `SystemHeader` (`rate_bound`, `audio_bound`, `video_bound`, lock/CSPS/fixed flags, `reserved_bits`; `Parse`/`Serialize`), `StdBufferBound`, `SYSTEM_HEADER_START_CODE` |
| Program stream map | `ProgramStreamMap<'a>` (`version`, `current_next_indicator`, `program_stream_info`, `elementary_stream_map: Vec<EsMapEntry>`, `crc`), `EsMapEntry<'a>`, `MAP_STREAM_ID` |
| Stream walking | `program_stream::{Pack, parse_pack, scan_packs, PackScan, SkippedRegion}` (one walker: `scan_packs` keeps every pack that parsed and reports each span it had to skip); `Pack` holds the pack header, optional system header, optional PSM and `pes_packets: Vec<mpeg_pes::PesPacket>` |
| Constants / errors | `PROGRAM_END_CODE`, `PACKET_START_CODE_PREFIX` (re-exported from `mpeg-pes`), `Error`, `Result` |

## Design notes

- **Zero-copy, symmetric.** Parsed types borrow the input and implement
  `Parse` + `Serialize`; serialize -> parse is the identity for the stream
  loop (invalid combinations are `Error::InvalidField`, not silently
  re-encoded differently).
- **Checked widths.** Every bit-field (SCR base/extension, `rate_bound`,
  `audio_bound`, `video_bound`, P-STD buffer size, `stream_id_extension`) is
  written with a checked `fit_bits` and returns `FieldOverflow` rather than
  masking.
- **Resynchronising scan.** `scan_packs` walks the whole stream and reports a
  skipped region instead of failing, so one damaged pack costs one span.
- `no_std` + `alloc`, `#![warn(missing_docs)]`.

## Related crates

Depends on `broadcast-common` and `mpeg-pes`. Used by `container-probe` (to
recognise Program Streams) and `transmux` (PS demux). The TS counterpart is
`mpeg-ts`.

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.

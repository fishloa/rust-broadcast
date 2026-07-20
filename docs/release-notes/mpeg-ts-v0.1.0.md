# mpeg-ts v0.1.0 — 2026-06-27

Initial release of `mpeg-ts` — MPEG-2 Transport Stream framing extracted from
`dvb-si` at the 8.0.0 breaking boundary.

## What's included

- **`ts`** — `TsPacket`, `TsHeader`, `AdaptationField`, `SectionReassembler`:
  parse ITU-T H.222.0 §2.4.3 TS packets and reassemble PSI sections from their
  payloads.
- **`mux`** — `SectionPacketiser`, `SiMux`: packetise PSI sections back into
  TS packets with correct CC and padding.
- **`resync`** — `TsResync`: lost-sync recovery via sliding-window 0x47 search.
- **`pid`** — `Pid`: typed 13-bit PID newtype with well-known constants.
- **`packet_buf`** — `TsPacketBuf`: owned aligned 188-byte buffer for
  zero-copy async handoff.
- **`section`** — `Section`: generic PSI/SI section header parser (table_id,
  section_length, version, CRC validation).

## `no_std` + embedded

`mpeg-ts` is `#![no_std]` + `alloc` by default. Enable `std` for
`std::error::Error` impls. Tested on `thumbv7em-none-eabi` in CI.

## Spec

ITU-T H.222.0 / ISO/IEC 13818-1 §2.4.

## Dependency

Requires `dvb-common` (lockstep, ≥ 8.0). Publish `dvb-common` (via the
lockstep `v8.0.0` tag) before tagging `mpeg-ts-v0.1.0`.

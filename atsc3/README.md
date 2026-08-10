# atsc3

ATSC 3.0 (NextGen TV) Low-Level Signalling — the A/331 LLS binary envelope
and the Service List Table (SLT) it carries.

Implements two pieces of the ATSC 3.0 signalling stack:

- **LLS binary envelope** (A/331 §6.2) — the 4-byte common `LLS_table()`
  header plus its gzip-compressed table body.
- **Service List Table (SLT)** (A/331 §6.3) — XML parse of the
  rapid-channel-scan bootstrap table, covering the fields named in the
  crate's initial scope (see [`src/slt.rs`](src/slt.rs) doc for the exact
  subset).

See "Planned" below for what this crate does **not** yet implement.

## Wire structures

- **`LlsEnvelope`** — the binary envelope carrying LLS tables (§6.2), with
  optional gzip decompression (`std` feature).
- **`LlsTableId`** — table type discriminant (SLT, RRT, SystemTime, AEAT,
  OnscreenMessageNotification, CDT, DRCT). `LLS_table_id` 0x07 is the DRCT
  (A/323 — Dedicated Return Channel Table), not CAP; it was modelled as CAP
  until commit 57b79e93 corrected it.
- **`Slt`** / **`SltService`** — Service List Table XML parse (§6.3).
- **`ServiceCategory`**, **`SlsProtocol`** — typed field enums.

## Usage

```rust
use atsc3::{LlsEnvelope, LlsTableId, slt::Slt};
use broadcast_common::Parse;

let bytes: &[u8] = &[/* LLS binary envelope */];
let envelope = LlsEnvelope::parse(bytes).unwrap();
if envelope.table_id == LlsTableId::Slt {
    let xml_bytes = envelope.decompress().unwrap();
    let xml = std::str::from_utf8(&xml_bytes).unwrap();
    let slt = Slt::parse(xml).unwrap();
    for svc in &slt.services {
        println!("{}: {:?}", svc.service_id, svc.short_service_name);
    }
}
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Enables gzip decompression of LLS payloads |
| `serde` | no      | Derives `Serialize`/`Deserialize` |

`no_std` + `alloc` when `default-features = false` (gzip decompression
unavailable; raw XML payloads still accessible).

## Planned

Not implemented yet — tracked as future work, transcribed in `docs/` ahead
of any code (see `docs/README.md`):

- **A/321** — System Discovery and Signalling: physical-layer bootstrap
  signalling.
- **A/331 ROUTE/DASH** — LCT-based object carriage over ALC/FLUTE (Annex A);
  the underlying LCT/ALC/FLUTE layer already exists in `rmt-flute`.
- **A/331 MMT** — MMTP-based delivery signalling (§7.2).
- **Service Layer Signalling (SLS)** — USBD, S-TSID, APD, HELD, DWD (§7.1).
- The remaining `SLT`/`Service` XML attributes not yet modeled by
  [`Slt`](src/slt.rs) (`globalServiceID`, `sltSvcSeqNum`, `protected`,
  `hideInGuide`, `SvcInetUrl`, `OtherBsid`/`OtherRf`, …).

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your
option.

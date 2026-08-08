# atsc3

ATSC 3.0 (NextGen TV) signalling — A/321 bootstrap + A/331 ROUTE/DASH and MMT.

Implements the signalling layer of the ATSC 3.0 broadcast system:

- **A/321** — System Discovery and Signalling: bootstrap signalling that lets a
  receiver discover available services and their delivery parameters.
- **A/331** — Signalling, Delivery, Synchronization, and Error Protection:
  ROUTE/DASH delivery (LCT-based object carriage over ALC/FLUTE) and MMT
  signalling, plus the Service List Table (SLT) and Service Layer Signalling
  (SLS).

## Wire structures

- **`LlsEnvelope`** — the binary envelope carrying LLS tables (§6.2), with
  optional gzip decompression (`std` feature).
- **`LlsTableId`** — table type discriminant (SLT, RRT, SystemTime, AEAT,
  OnscreenMessage, CDT, CAP).
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

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your
option.

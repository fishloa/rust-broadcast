# st2022

SMPTE ST 2022-6:2012 / ST 2022-7:2019 — SDI-over-IP transport.

ST 2022-6 defines the **High Bit Rate Media Transport (HBRMT)** RTP payload
format for carrying uncompressed SDI signals (SD/HD/3G) over IP networks.
The entire serial digital interface payload — video, embedded audio, VANC,
HANC — is encapsulated as a single RTP stream.

ST 2022-7 adds **seamless protection switching** across two redundant network
paths, allowing hitless failover with no visible glitch.

## Wire structures

- **`PayloadHeader`** — the 4/8/12+-byte HBRMT header preceding the media
  payload in each RTP datagram (§6.4).
- **`VideoSourceFormat`** — the MAP/FRAME/FRATE/SAMPLE fields describing the
  SDI signal structure.
- **`ClockFrequency`**, **`FecUsage`**, **`TimestampRef`**, **`Scrambling`**,
  **`VideoSourceId`** — typed field enums.

## Usage

```rust
use st2022::PayloadHeader;
use broadcast_common::Parse;

let bytes: &[u8] = &[/* RTP payload after fixed header */];
let header = PayloadHeader::parse(bytes).unwrap();
println!("Video source: {:?}", header.video_source_format);
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Enables `std::error::Error` impls |
| `serde` | no      | Derives `Serialize`/`Deserialize` |

`no_std` + `alloc` when `default-features = false`.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your
option.

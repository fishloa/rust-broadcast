# isobmff-tokio

Async (tokio) driver over the [`isobmff`](../isobmff) sans-IO MP4 reader.

`isobmff::Reader` asks for structure byte ranges and yields descriptors; this
crate answers those demands over an `AsyncRead + AsyncSeek` source and hands
back `DriverStep::{Tracks, Fragment, Sample, Done}`. Sample bytes are fetched
one at a time with `read_sample`, so no whole file and no media payload is
held. It reuses the blocking driver's buffer-sizing helpers and `DriverError`,
so the two drivers cannot drift.

## Example

```rust,no_run
use isobmff::driver::DriverStep;
use isobmff::ReaderConfig;
use isobmff_tokio::AsyncFileReader;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = tokio::fs::File::open("movie.mp4").await?;
    let mut reader = AsyncFileReader::new(file, ReaderConfig::default()).await?;
    loop {
        match reader.step().await? {
            DriverStep::Sample(sample) => println!("track {} at {}", sample.track, sample.decode_time),
            DriverStep::Done => break,
            _ => {}
        }
    }
    Ok(())
}
```

Two runnable examples ship with the crate:

```bash
cargo run -p isobmff-tokio --example list_samples -- fixtures/mp4/progressive/av-prog.mp4
cargo run -p isobmff-tokio --example read_sample -- fixtures/mp4/progressive/av-prog.mp4
```

## Crate layout

- `AsyncFileReader` — the driver.

## Features

No optional features. The blocking `isobmff::driver` is behind `isobmff`'s
`std` feature, which this crate always enables.

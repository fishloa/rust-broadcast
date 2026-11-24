//! Read the first sample of an MP4 through the async driver and print its size.
//!
//! ```bash
//! cargo run -p isobmff-tokio --example read_sample -- fixtures/mp4/progressive/av-prog.mp4
//! ```

use std::env;

use isobmff::driver::DriverStep;
use isobmff::{ReaderConfig, SampleOrder};
use isobmff_tokio::AsyncFileReader;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: read_sample <FILE>");
        std::process::exit(2);
    });
    let file = tokio::fs::File::open(&path)
        .await
        .expect("open the input file");
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let mut reader = AsyncFileReader::new(file, config)
        .await
        .expect("wrap the input file");
    let mut sample = None;
    loop {
        match reader.step().await {
            Ok(DriverStep::Sample(found)) => {
                sample = Some(found);
                break;
            }
            Ok(DriverStep::Tracks(_) | DriverStep::Fragment(_)) => {}
            Ok(DriverStep::Done) => break,
            Ok(_) => {}
            Err(error) => {
                eprintln!("stop: {error}");
                std::process::exit(1);
            }
        }
    }
    let Some(sample) = sample else {
        eprintln!("no sample in {path}");
        std::process::exit(1);
    };
    let mut bytes = Vec::new();
    reader
        .read_sample(&sample, &mut bytes)
        .await
        .expect("read the sample");
    println!(
        "track {} sample {} at {}: {} bytes",
        sample.track.value(),
        sample.index,
        sample.decode_time,
        bytes.len()
    );
}

//! Basic: drive the async `SectionStream` over an in-memory TS and count the
//! SI sections it yields.
//!
//! Run with: `cargo run -p dvb-stream --example count_sections`
//!
//! Opens the committed `m6-single.ts` fixture (from the sibling `dvb-si` crate)
//! as an async file and streams its packets in bounded chunks, so the capture
//! is never held whole.

use dvb_stream::SectionStream;
use futures_util::StreamExt;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = test_fixtures::path("ts/m6-single.ts");
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let reader = tokio::io::BufReader::new(file);
    let mut stream = SectionStream::new(reader);

    let mut sections = 0u32;
    while let Some(_event) = stream.next().await {
        sections += 1;
    }

    println!("section events: {sections}");
}

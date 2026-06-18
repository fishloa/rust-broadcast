//! Basic: drive the async `SectionStream` over an in-memory TS and count the
//! SI sections it yields.
//!
//! Run with: `cargo run -p dvb-stream --example count_sections`
//!
//! Reads the committed `m6-single.ts` fixture (from the sibling `dvb-si` crate)
//! into memory and feeds it through the stream via a `Cursor`.

use dvb_stream::SectionStream;
use futures_util::StreamExt;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dvb-si/tests/fixtures/m6-single.ts"
    );
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let reader = tokio::io::BufReader::new(std::io::Cursor::new(data));
    let mut stream = SectionStream::new(reader);

    let mut sections = 0u32;
    while let Some(_event) = stream.next().await {
        sections += 1;
    }

    println!("section events: {sections}");
}

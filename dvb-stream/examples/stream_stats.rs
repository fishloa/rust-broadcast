//! Advanced: stream SI sections asynchronously, tally the table types, and
//! report the demux + resync statistics.
//!
//! Run with: `cargo run -p dvb-stream --example stream_stats`

use dvb_si::tables::AnyTableSection;
use dvb_stream::SectionStream;
use futures_util::StreamExt;
use std::collections::BTreeMap;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let reader = tokio::io::BufReader::new(std::io::Cursor::new(data));
    let mut stream = SectionStream::new(reader);

    let mut tables: BTreeMap<&'static str, u32> = BTreeMap::new();
    while let Some(event) = stream.next().await {
        let name = match event.table_section() {
            Ok(AnyTableSection::PatSection(_)) => "PAT",
            Ok(AnyTableSection::PmtSection(_)) => "PMT",
            Ok(AnyTableSection::SdtSection(_)) => "SDT",
            Ok(AnyTableSection::NitSection(_)) => "NIT",
            Ok(AnyTableSection::EitSection(_)) => "EIT",
            Ok(_) => "other",
            Err(_) => "malformed",
        };
        *tables.entry(name).or_default() += 1;
    }

    println!("table sections:");
    for (name, n) in &tables {
        println!("  {name:<10} {n}");
    }

    let stats = stream.stats();
    let resync = stream.resync_stats();
    println!("\ndemux  : {stats:?}");
    println!(
        "resync : {} resyncs, {} bytes discarded",
        resync.resyncs, resync.bytes_discarded
    );
}

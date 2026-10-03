//! Golden of `SectionStream` over a real capture (W1-R-low-a): the full event
//! sequence (pid, table_id, version, section length, first 8 bytes) plus the
//! demux and resync counters. Regenerate only with `GOLDEN_UPDATE=1` on `main`.

use std::pin::Pin;

use dvb_stream::SectionStream;
use futures_core::Stream;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");

#[tokio::test]
async fn section_stream_events_are_byte_identical_to_golden() {
    let data = std::fs::read(FIXTURE).expect("m6-single.ts");
    let mut stream = SectionStream::new(std::io::Cursor::new(data));
    let mut out = String::new();
    while let Some(ev) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
        out.push_str(&format!("{ev:?}\n"));
    }
    out.push_str(&format!(
        "stats={:?}\nresync={:?}\n",
        stream.stats(),
        stream.resync_stats()
    ));
    let path = format!(
        "{}/tests/golden/m6_single_events.golden",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(
        out,
        std::fs::read_to_string(&path).expect("golden"),
        "events differ"
    );
}

//! Soak: `dvb-tools`'s input path over a generated multi-gigabyte capture
//! holds one packet (plus the SI demux's small table state), never the file
//! (streaming-first Phase 6, slice 4).
//!
//! The tool is a binary crate, so its reader is included here verbatim from
//! `src/util.rs`; the per-section work is the same `SiDemux` → typed table
//! decode `dump`, `services`, `epg` and `pids` all drive.
//!
//! Always runs at a small size; `RUST_BROADCAST_SOAK=1` raises it to the
//! 4 GiB the gate names. `RUST_BROADCAST_SOAK_BYTES` overrides either.

#[path = "../src/util.rs"]
mod util;

use dvb_si::demux::{SiDemux, SiDemuxConfig};
use test_alloc::ThreadCapped;

use crate::util::PacketReader;

/// Live-byte budget for the whole test: generous enough for generating the
/// file, small enough that a whole-capture read could never fit.
const BUDGET: usize = 64 * 1024 * 1024;

/// Extra live bytes the analysis pass may hold above its level at entry.
const STREAM_BOUND: usize = 16 * 1024 * 1024;

const ALWAYS_ON_BYTES: u64 = 64 * 1024 * 1024;
const SOAK_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

fn target_bytes() -> u64 {
    if let Some(bytes) = std::env::var_os("RUST_BROADCAST_SOAK_BYTES")
        .and_then(|value| value.to_str().and_then(|text| text.parse::<u64>().ok()))
    {
        return bytes;
    }
    if std::env::var_os("RUST_BROADCAST_SOAK").as_deref() == Some(std::ffi::OsStr::new("1")) {
        SOAK_BYTES
    } else {
        ALWAYS_ON_BYTES
    }
}

#[test]
fn a_big_ts_analyses_under_a_fixed_budget() {
    let target = target_bytes();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::create_dir_all(&dir);
    let input = dir.join("soak-dvb-tools.ts");
    let written = test_harness::big_media::write_big_ts(
        &input,
        &test_fixtures::path("ts/h264-aac.ts"),
        target,
    );
    assert!(
        written.bytes >= target,
        "generated {} bytes, wanted {target}",
        written.bytes
    );

    let baseline = ThreadCapped::<BUDGET>::live();
    ThreadCapped::<BUDGET>::reset_peak();

    let mut demux = SiDemux::new(SiDemuxConfig::default());
    let mut sections = 0u64;
    let mut bytes = 0u64;
    let mut packets = PacketReader::open(
        input.to_str().expect("UTF-8 scratch path"),
        "dvb-tools soak",
    )
    .expect("generated file opens");
    for packet in packets.by_ref() {
        bytes = bytes.saturating_add(packet.len() as u64);
        for event in demux.feed(&packet) {
            if let Ok(table) = event.table_section() {
                // The typed decode every subcommand performs on a section.
                let _ = table.name();
                sections += 1;
            }
        }
    }
    packets.finish().expect("a generated stream reads cleanly");

    let peak = ThreadCapped::<BUDGET>::peak();
    let extra = peak.saturating_sub(baseline);
    println!(
        "soak_streaming (dvb-tools): {bytes} bytes in, {sections} sections, \
         peak {peak} bytes ({extra} above entry), bound {STREAM_BOUND}"
    );
    assert!(
        extra <= STREAM_BOUND,
        "the analysis held {extra} live bytes above entry, over the {STREAM_BOUND}-byte bound"
    );
    let _ = std::fs::remove_file(&input);
}

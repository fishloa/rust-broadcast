//! Soak: repairing a generated multi-gigabyte TS holds one packet (plus the
//! engine's per-PID state), never the capture (streaming-first Phase 6,
//! slice 4).
//!
//! Always runs at a small size; `RUST_BROADCAST_SOAK=1` raises it to the
//! 4 GiB the gate names. `RUST_BROADCAST_SOAK_BYTES` overrides either.

use std::fs::File;
use std::io::{self, Write};

use mpeg_ts::reader::PacketReader;
use test_alloc::ThreadCapped;
use ts_fix::TsFix;

/// Live-byte budget for the whole test: generous enough for generating the
/// file, small enough that a whole-capture read could never fit.
const BUDGET: usize = 64 * 1024 * 1024;

/// Extra live bytes the repair pass may hold above its level at entry.
const STREAM_BOUND: usize = 16 * 1024 * 1024;

const ALWAYS_ON_BYTES: u64 = 64 * 1024 * 1024;
const SOAK_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

/// An output sink that counts what it is given and stores none of it.
struct CountingSink {
    bytes: u64,
}

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buf.len() as u64);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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
fn a_big_ts_repairs_under_a_fixed_budget() {
    let target = target_bytes();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::create_dir_all(&dir);
    let input = dir.join("soak-ts-fix.ts");
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

    let mut engine = TsFix::builder()
        .repair_continuity()
        .build()
        .expect("engine builds");
    let file = File::open(&input).expect("generated file opens");
    let mut sink = CountingSink { bytes: 0 };
    for packet in PacketReader::new(file) {
        let packet = packet.expect("a generated stream reads cleanly");
        engine
            .push(&packet, |emitted| {
                let _ = sink.write_all(emitted);
            })
            .expect("aligned packets");
    }
    engine.finish(|emitted| {
        let _ = sink.write_all(emitted);
    });
    let _ = sink.flush();

    let peak = ThreadCapped::<BUDGET>::peak();
    let extra = peak.saturating_sub(baseline);
    println!(
        "soak_streaming (ts-fix): {target} bytes in, {} bytes out, peak {peak} bytes \
         ({extra} above entry), bound {STREAM_BOUND}",
        sink.bytes
    );
    assert!(
        extra <= STREAM_BOUND,
        "repair held {extra} live bytes above entry, over the {STREAM_BOUND}-byte bound"
    );
    assert_eq!(sink.bytes % 188, 0, "the output is whole TS packets");
    let _ = std::fs::remove_file(&input);
}

#![no_main]

use libfuzzer_sys::fuzz_target;
use mpeg_ps::scan::MAX_PES_BYTES;
use mpeg_ps::{PsLimits, PsScanner};

/// The scanner may hold one unit plus the 4-byte boundary lookahead that tells
/// it a complete PES really ends where its declared length says.
const BUFFER_SLACK: usize = 4;

fuzz_target!(|data: &[u8]| {
    let Some((&chunk_byte, stream)) = data.split_first() else {
        return;
    };
    // Arbitrary chunking: 1..=64 bytes per feed.
    let chunk = usize::from(chunk_byte) % 64 + 1;
    let mut scanner = PsScanner::new(PsLimits::new());
    let mut steps = 0u32;
    for piece in stream.chunks(chunk) {
        scanner.feed(piece);
        loop {
            match scanner.poll() {
                Ok(Some(_)) => steps += 1,
                Ok(None) | Err(_) => break,
            }
            // Every event consumes input, so this only bounds a pathological
            // scanner bug rather than a legitimate stream.
            assert!(steps < 4_000_000, "the scanner stopped consuming input");
        }
        assert!(
            scanner.buffered_len() <= MAX_PES_BYTES + BUFFER_SLACK,
            "held {} bytes, past the PES cap plus lookahead",
            scanner.buffered_len()
        );
    }
    scanner.finish();
    loop {
        match scanner.poll() {
            Ok(Some(_)) => steps += 1,
            Ok(None) | Err(_) => break,
        }
        assert!(steps < 4_000_000, "the scanner stopped consuming input");
    }
});

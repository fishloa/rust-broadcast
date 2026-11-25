/// Walk the real `.mpg` fixture at runtime, report pack count + SCR timeline.
///
/// ```sh
/// cargo run -p mpeg-ps --example walk_ps
/// ```
///
/// The stream is read in bounded chunks and scanned with `mpeg_ps::PsScanner`,
/// so the file is never held whole.
use std::fs::File;
use std::io::{self, Read};

use mpeg_ps::{PsEvent, PsLimits, PsScanner};

/// Read granularity for streaming the capture.
const READ_CHUNK_LEN: usize = 1 << 20;

/// What the first (pre-report) pass learns about the stream.
#[derive(Default)]
struct StreamSummary {
    /// One rendered line per skipped byte region.
    skipped: Vec<String>,
    /// Trailing bytes that never formed a complete unit.
    trailing: usize,
    /// Number of pack headers seen.
    packs: usize,
    /// Total bytes scanned.
    size: usize,
}

/// Scan the whole stream once, collecting what the header reports.
fn summarise(path: &std::path::Path) -> io::Result<StreamSummary> {
    let mut scanner = PsScanner::new(PsLimits::new());
    let mut summary = StreamSummary::default();
    let mut reader = io::BufReader::with_capacity(READ_CHUNK_LEN, File::open(path)?);
    let mut chunk = vec![0u8; READ_CHUNK_LEN];

    loop {
        let read = read_chunk(&mut reader, &mut chunk)?;
        if read == 0 {
            break;
        }
        summary.size += read;
        scanner.feed(&chunk[..read]);
        drain(&mut scanner, false, &mut summary);
    }
    scanner.finish();
    drain(&mut scanner, true, &mut summary);
    Ok(summary)
}

/// Drain every unit the scanner can yield.
fn drain(scanner: &mut PsScanner, finished: bool, summary: &mut StreamSummary) {
    while let Ok(Some(event)) = scanner.poll() {
        match event {
            PsEvent::Pack(_) => summary.packs += 1,
            PsEvent::Resync { skipped } => {
                if finished {
                    summary.trailing = summary
                        .trailing
                        .saturating_add(usize::try_from(skipped).unwrap_or(usize::MAX));
                } else {
                    summary
                        .skipped
                        .push(format!("Skipped {skipped} bytes during resynchronisation"));
                }
            }
            PsEvent::End => break,
            _ => {}
        }
    }
}

/// One `read`, retried on `Interrupted`.
fn read_chunk(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under the
    // crate. A committed fixture that cannot be read is a bug, not a reason
    // to skip — so a missing/unreadable fixture is a hard failure.
    let path = test_fixtures::path("ps/ffmpeg-mpeg2-ps.mpg");
    let summary = summarise(&path).unwrap_or_else(|e| {
        panic!(
            "committed fixture {} could not be read: {e}",
            path.display()
        )
    });

    for line in &summary.skipped {
        println!("{line}");
    }

    println!("File: {}", path.display());
    println!("Size: {} bytes", summary.size);
    println!("Trailing bytes: {}", summary.trailing);
    println!("Packs: {}", summary.packs);
    println!();
    println!(
        "{:<6} {:<20} {:<15} {:<10}",
        "Pack", "SCR (ticks)", "SCR (s)", "mux_rate (B/s)"
    );
    println!("{:-<6} {:-<20} {:-<15} {:-<10}", "", "", "", "");

    let mut scanner = PsScanner::new(PsLimits::new());
    let mut reader = io::BufReader::with_capacity(
        READ_CHUNK_LEN,
        File::open(&path).unwrap_or_else(|e| {
            panic!(
                "committed fixture {} could not be read: {e}",
                path.display()
            )
        }),
    );
    let mut chunk = vec![0u8; READ_CHUNK_LEN];
    let mut pack_index = 0usize;
    let mut pes_total = 0usize;

    loop {
        let read = read_chunk(&mut reader, &mut chunk)
            .unwrap_or_else(|e| panic!("committed fixture {} read failed: {e}", path.display()));
        if read == 0 {
            break;
        }
        scanner.feed(&chunk[..read]);
        list_events(&mut scanner, &mut pack_index, &mut pes_total);
    }
    scanner.finish();
    list_events(&mut scanner, &mut pack_index, &mut pes_total);

    println!();
    println!("Total PES packets: {pes_total}");
}

fn list_events(scanner: &mut PsScanner, pack_index: &mut usize, pes_total: &mut usize) {
    while let Ok(Some(event)) = scanner.poll() {
        match event {
            PsEvent::Pack(header) => {
                let scr_ticks = header.scr.ticks();
                let scr_s = header.scr.seconds();
                let mux_rate = header.program_mux_rate * 50;
                println!(
                    "{:<6} {:<20} {:<15.6} {:<10}",
                    *pack_index, scr_ticks, scr_s, mux_rate
                );
                *pack_index += 1;
            }
            PsEvent::SystemHeader(sh) => {
                println!(
                    "       system_header: rate_bound={}, audio_bound={}, video_bound={}, streams={}",
                    sh.rate_bound,
                    sh.audio_bound,
                    sh.video_bound,
                    sh.std_buffer_bounds.len(),
                );
            }
            PsEvent::Pes { packet, .. } => {
                *pes_total += 1;
                if let Ok(pes) = mpeg_pes::PesPacket::parse(packet)
                    && let Some(pts) = pes.header.as_ref().and_then(|h| h.pts)
                {
                    let pts_s = pts.seconds();
                    println!(
                        "         PES stream_id={:#04x} PTS={:.6}s payload={}B",
                        pes.stream_id.get(),
                        pts_s,
                        pes.payload.len(),
                    );
                }
            }
            PsEvent::End => break,
            _ => {}
        }
    }
}

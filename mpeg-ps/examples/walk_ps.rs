/// Walk the real `.mpg` fixture at runtime, report pack count + SCR timeline.
///
/// ```sh
/// cargo run -p mpeg-ps --example walk_ps
/// ```
use std::fs;

use mpeg_ps::program_stream;

fn main() {
    // Resolve relative to the crate so it runs from any cwd.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ffmpeg-mpeg2-ps.mpg");
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };
    let (packs, trailing) = program_stream::parse_all_packs(&data).unwrap();

    println!("File: {path}");
    println!("Size: {} bytes", data.len());
    println!("Trailing bytes: {}", trailing.len());
    println!("Packs: {}", packs.len());
    println!();
    println!(
        "{:<6} {:<20} {:<15} {:<10}",
        "Pack", "SCR (ticks)", "SCR (s)", "mux_rate (B/s)"
    );
    println!("{:-<6} {:-<20} {:-<15} {:-<10}", "", "", "", "");

    let mut pes_total = 0usize;
    for (i, pack) in packs.iter().enumerate() {
        let scr_ticks = pack.pack_header.scr.ticks();
        let scr_s = pack.pack_header.scr.seconds();
        let mux_rate = pack.pack_header.program_mux_rate * 50;
        println!(
            "{:<6} {:<20} {:<15.6} {:<10}",
            i, scr_ticks, scr_s, mux_rate,
        );

        if let Some(ref sh) = pack.system_header {
            println!(
                "       system_header: rate_bound={}, audio_bound={}, video_bound={}, streams={}",
                sh.rate_bound,
                sh.audio_bound,
                sh.video_bound,
                sh.std_buffer_bounds.len(),
            );
        }

        for pes in &pack.pes_packets {
            pes_total += 1;
            if let Some(ref hdr) = pes.header {
                if let Some(pts) = hdr.pts {
                    let pts_s = pts.seconds();
                    println!(
                        "         PES stream_id={:#04x} PTS={:.6}s payload={}B",
                        pes.stream_id.0,
                        pts_s,
                        pes.payload.len(),
                    );
                }
            }
        }
    }

    println!();
    println!("Total PES packets: {pes_total}");
}

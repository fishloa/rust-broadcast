mod demo;

use demo::SPLICE_EVENT_ID;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let demo = demo::run()?;

    println!(
        "SSAI ad-stitch demo — output written to {}\n",
        demo.out_dir.display()
    );

    println!("splice_event_id  : {SPLICE_EVENT_ID}");
    println!("raw cue          : {} bytes", demo.raw_cue.len());
    println!("discontinuities  : {}", demo.discontinuity_points.len());
    for p in &demo.discontinuity_points {
        println!(
            "  track {} sample {} @ pts {}",
            p.track_id, p.sample_index, p.presentation_time
        );
    }
    println!();
    println!("--- EXT-X-DATERANGE ---");
    println!("{}", demo.daterange.to_tag_line()?);
    println!();
    println!("--- HLS media playlist (video) ---");
    println!("{}", demo.m3u8);
    println!("--- DASH MPD ---");
    println!("{}", demo.mpd);
    println!("emsg box: {} bytes", demo.emsg_bytes.len());

    Ok(())
}

//! Record the real DVB-T capture with programme tracking and list the
//! programme boundaries the archive holds.
//!
//! ```text
//! cargo run -p dvr-runtime --example list_programmes --all-features -- <input.ts>
//! ```

use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_common::bounded::{Limit, read_file_bounded_string};
use bytes::Bytes;
use dvr_runtime::fs::FsRecorder;
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::{
    ArchiveConfig, ArchiveContainer, ArchiveName, PeriodNum, RetentionLimits, layout,
};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, Trunk, TrunkConfig};

/// A whole archive period playlist, bounded by the ceiling `broadcast-hls`
/// itself accepts.
const PLAYLIST_LIMIT: Limit = Limit::new(
    "archive period playlist",
    broadcast_hls::limits::MAX_PLAYLIST_BYTES as u64,
);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join("dvr-runtime-example-programmes");
    let _ = std::fs::remove_dir_all(&root);
    let input = std::env::args()
        .nth(1)
        .ok_or("usage: list_programmes <input.ts>")?;
    let bytes = std::fs::read(&input)?;
    let trunk = Trunk::new(
        TrunkConfig::default().with_segment_capacity(std::num::NonZeroUsize::new(16).unwrap()),
    );
    let config = ArchiveConfig::new(
        ArchiveContainer::Ts,
        Duration::from_secs(2),
        RetentionLimits::new().with_periods(std::num::NonZeroUsize::new(4).unwrap()),
    )
    .with_period_duration(Some(Duration::from_secs(3600)))
    .with_programme_service(0x0601);
    let name = ArchiveName::new("tf1")?;
    let mut recorder = FsRecorder::new(&root, name, config, &trunk)?;
    recorder.feed_si(&bytes)?;
    let writer = trunk
        .segment_writer()
        .ok_or("the segment writer is already taken")?;
    let slice = bytes.len().div_ceil(2).div_ceil(188) * 188;
    for (index, chunk) in bytes.chunks(slice).enumerate() {
        let seq = u32::try_from(index)? + 1;
        writer.publish_segment(SegmentEntry::new(
            Bytes::copy_from_slice(chunk),
            SegmentSeq::new(seq),
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            index == 0,
        ))?;
        let wall = jiff::Timestamp::from_second(1_716_055_200 + i64::try_from(index * 2)?)?;
        recorder.poll(
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            |_| Some(wall),
            &[],
        )?;
    }
    recorder.poll(
        Timestamp::from_nanos(10_000_000_000),
        |_| jiff::Timestamp::from_second(1_716_055_210).ok(),
        &[],
    )?;

    let dir = layout::route_dir(&root, &ArchiveName::new("tf1")?);
    println!("archive: {}", dir.display());
    for period in 0..4u32 {
        let path = dir.join(layout::playlist_file_name(PeriodNum::new(period)));
        let Ok(text) = read_file_bounded_string(&path, PLAYLIST_LIMIT) else {
            continue;
        };
        let playlist = parse_period_playlist(
            PeriodNum::new(period),
            ArchiveContainer::Ts,
            text.as_bytes(),
        )?;
        for programme in playlist.programmes {
            println!(
                "p{} seq {}: service {} event {} {:?}",
                programme.period,
                programme.first_seq,
                programme.service_id,
                programme.event_id,
                programme.title
            );
        }
    }
    Ok(())
}

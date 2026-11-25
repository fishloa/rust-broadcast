//! Record a real TS capture into a dvr-runtime archive and print what landed.
//!
//! ```text
//! cargo run -p dvr-runtime --example record_ts_file --all-features -- <input.ts>
//! ```

use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use dvr_runtime::fs::FsRecorder;
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, RetentionLimits, layout};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, Trunk, TrunkConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join("dvr-runtime-example-record");
    let _ = std::fs::remove_dir_all(&root);
    let input = std::env::args()
        .nth(1)
        .ok_or("usage: record_ts_file <input.ts>")?;
    let bytes = std::fs::read(&input)?;
    let trunk = Trunk::new(
        TrunkConfig::default().with_segment_capacity(std::num::NonZeroUsize::new(16).unwrap()),
    );
    let config = ArchiveConfig::new(
        ArchiveContainer::Ts,
        Duration::from_secs(2),
        RetentionLimits::new().with_periods(std::num::NonZeroUsize::new(4).unwrap()),
    )
    .with_period_duration(Some(Duration::from_secs(4)));
    let name = ArchiveName::new("capture")?;
    let mut recorder = FsRecorder::new(&root, name, config, &trunk)?;
    let writer = trunk
        .segment_writer()
        .ok_or("the segment writer is already taken")?;

    let slice = bytes.len().div_ceil(5).div_ceil(188) * 188;
    let mut now = Timestamp::ZERO;
    for (index, chunk) in bytes.chunks(slice).enumerate() {
        let seq = u32::try_from(index)? + 1;
        writer.publish_segment(SegmentEntry::new(
            Bytes::copy_from_slice(chunk),
            SegmentSeq::new(seq),
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            index == 0,
        ))?;
        now = now.saturating_add(Duration::from_secs(2));
        recorder.poll(now, |_| None, &[])?;
    }
    recorder.poll(now, |_| None, &[])?;

    let dir = layout::route_dir(&root, &ArchiveName::new("capture")?);
    println!("archive: {}", dir.display());
    for entry in std::fs::read_dir(&dir)? {
        println!("  {}", entry?.file_name().to_string_lossy());
    }
    let playlist = std::fs::read_to_string(dir.join("p0.m3u8"))?;
    println!(
        "\n{}",
        playlist.lines().take(9).collect::<Vec<_>>().join("\n")
    );
    Ok(())
}

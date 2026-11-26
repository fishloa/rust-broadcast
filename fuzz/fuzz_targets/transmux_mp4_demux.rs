#![no_main]

use broadcast_common::stage::{Stage, Timestamp};
use isobmff::{Limits, SampleOrder};
use libfuzzer_sys::fuzz_target;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};

/// Caps small enough that a hostile table cannot allocate its way out of the
/// fuzzer's own budget.
fn fuzz_limits() -> Limits {
    Limits::default()
        .with_max_depth(8)
        .with_max_moov_bytes(1 << 20)
        .with_max_samples_per_track(1 << 14)
        .with_max_sample_bytes(1 << 20)
        .with_max_fragment_bytes(1 << 20)
        .with_max_metadata_item_bytes(1 << 18)
}

/// A hostile file cannot describe more demands or events than this.
const MAX_STEPS: usize = 1 << 16;

/// Upper bound on how many fuzz-chosen chunks one sequential segment spans.
const MAX_SEGMENT_CHUNKS: usize = 8;

fn drain(demux: &mut Mp4Demux) {
    while demux.poll().is_some() {}
}

fn drive_seekable(data: &[u8], order: SampleOrder) {
    let config = Mp4DemuxConfig::default()
        .with_total_len(Some(data.len() as u64))
        .with_order(order)
        .with_limits(fuzz_limits());
    let mut demux = Mp4Demux::seekable(config);
    for _ in 0..MAX_STEPS {
        drain(&mut demux);
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(data.len());
        let bytes = data.get(start..end).unwrap_or(&[]).to_vec();
        if demux
            .feed(
                Mp4Input::At {
                    offset,
                    bytes: bytes.into(),
                },
                Timestamp::ZERO,
            )
            .is_err()
        {
            break;
        }
    }
    let _ = demux.finish();
    drain(&mut demux);
}

fn drive_sequential(data: &[u8], chunk: usize, per_segment: usize) {
    let config = Mp4DemuxConfig::default()
        .with_total_len(Some(data.len() as u64))
        .with_limits(fuzz_limits());
    let mut demux = Mp4Demux::sequential(config);
    let mut offset = 0usize;
    let mut fed = 0usize;
    for _ in 0..MAX_STEPS {
        drain(&mut demux);
        if demux.demand().saturated {
            break;
        }
        if offset >= data.len() {
            break;
        }
        let take = chunk.min(data.len() - offset);
        if demux
            .feed(
                Mp4Input::Next(&data[offset..offset + take]),
                Timestamp::ZERO,
            )
            .is_err()
        {
            break;
        }
        offset += take;
        fed += 1;
        if fed % per_segment == 0 && demux.feed(Mp4Input::SegmentEnd, Timestamp::ZERO).is_err() {
            break;
        }
    }
    let _ = demux.feed(Mp4Input::SegmentEnd, Timestamp::ZERO);
    let _ = demux.finish();
    drain(&mut demux);
}

fuzz_target!(|input: &[u8]| {
    let Some((mode, rest)) = input.split_first() else {
        return;
    };
    let Some((chunk, data)) = rest.split_first() else {
        return;
    };
    let order = if mode & 1 == 0 {
        SampleOrder::DecodeTime
    } else {
        SampleOrder::File
    };
    if mode & 2 == 0 {
        drive_seekable(data, order);
    }
    if mode & 4 == 0 {
        let per_segment = usize::from(*chunk) % MAX_SEGMENT_CHUNKS + 1;
        drive_sequential(data, usize::from(*chunk) + 1, per_segment);
    }
});

#![no_main]

use broadcast_common::stage::Timestamp;
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use media_plane::{ByteMerge, MergePolicy, SourceId};
use std::time::Duration;

// `ByteMerge` (media-plane step 3f acceptance): the one place N byte sources
// reduce to one stream in the byte layer, and this project's
// unbounded-allocation incidents have all been in code eating remote input
// directly -- exactly what `feed()` does here. Drive an arbitrary
// interleaving of `feed`/`poll`/`on_deadline` under both `FirstArrival` and
// `Failover`, with fuzzer-chosen (and deliberately sometimes out-of-range)
// source ids, and assert the output queue never exceeds the `max_queued`
// bound it was constructed with -- a bound violation here would be the same
// failure class as those four earlier incidents.
fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let num_sources = (data[0] % 4) as usize + 1;
    let max_queued = (data[1] % 32) as usize + 1;
    let policy = if data[2] % 2 == 0 {
        MergePolicy::FirstArrival
    } else {
        MergePolicy::Failover {
            primary: SourceId(0),
            secondary: SourceId((data[3] as usize) % num_sources),
            silence_timeout: Duration::from_millis(data[3] as u64 + 1),
        }
    };

    let mut merge = ByteMerge::new(policy, num_sources, max_queued);
    let mut ts = 0u64;
    for rec in data[4..].chunks(6) {
        if rec.len() < 2 {
            break;
        }
        // Deliberately allow `source` to land one past `num_sources - 1` so
        // `MergeError::UnknownSource` is exercised too, not just in-range
        // sources.
        let source = SourceId((rec[1] as usize) % (num_sources + 1));
        ts = ts.wrapping_add(1 + rec.get(2).copied().unwrap_or(0) as u64);
        match rec[0] % 3 {
            0 => {
                let payload = rec.get(3..).unwrap_or(&[]);
                let _ = merge.feed(
                    source,
                    Bytes::copy_from_slice(payload),
                    Timestamp::from_nanos(ts),
                );
            }
            1 => {
                let _ = merge.poll();
            }
            _ => merge.on_deadline(Timestamp::from_nanos(ts)),
        }
        assert!(
            merge.len() <= max_queued,
            "ByteMerge output queue exceeded its constructed bound"
        );
    }
});

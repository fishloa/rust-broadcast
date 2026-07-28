#![no_main]

use broadcast_common::stage::Timestamp;
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use media_plane::{ByteTap, TapItem, TapPoint};

// `ByteTap` (media-plane step 3f acceptance): a positional observer fed
// directly from remote bytes in flight, whose entire contract is "never
// blocks, never grows its ring past `capacity`, and never silently drops a
// gap without reporting it". Drive an arbitrary interleaving of
// `record`/`poll` from fuzzer bytes and check both halves of that contract:
// the ring bound holds under flood, and every recorded item is accounted
// for by `poll()` as either real data or a `Lagged { skipped }` count -- no
// item may vanish unaccounted for.
fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let capacity = (data[0] % 16) as usize + 1;
    let point = if data[0] & 0x80 == 0 {
        TapPoint::Wire
    } else {
        TapPoint::PostTransform
    };
    let mut tap = ByteTap::new(point, capacity);

    let mut recorded: u64 = 0;
    let mut observed: u64 = 0;
    for rec in data[1..].chunks(4) {
        if rec.is_empty() {
            break;
        }
        if rec[0] % 2 == 0 {
            let payload = rec.get(1..).unwrap_or(&[]);
            tap.record(Bytes::copy_from_slice(payload), Timestamp::from_nanos(recorded));
            recorded += 1;
            assert!(tap.len() <= capacity, "ByteTap ring exceeded its constructed bound");
        } else {
            match tap.poll() {
                Some(TapItem::Data(_, _)) => observed += 1,
                Some(TapItem::Lagged { skipped }) => observed += skipped,
                None => {}
                // `TapItem` is `#[non_exhaustive]`: a future loss class must
                // not silently break this target's observed==recorded
                // accounting by being counted as nothing.
                Some(other) => panic!("unhandled TapItem variant: {other:?}"),
            }
        }
    }
    // Drain whatever is left so the accounting below covers everything
    // recorded this iteration, not just what was polled during the loop.
    while let Some(item) = tap.poll() {
        match item {
            TapItem::Data(_, _) => observed += 1,
            TapItem::Lagged { skipped } => observed += skipped,
            other => panic!("unhandled TapItem variant: {other:?}"),
        }
    }
    assert_eq!(
        observed, recorded,
        "every recorded item must be observed as data or accounted for as skipped -- none may vanish"
    );
});

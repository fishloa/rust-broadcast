#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;

#[global_allocator]
static ALLOC: test_alloc::ThreadCounting = test_alloc::ThreadCounting;

// Structured damage on a real committed seed (one per non-ISOBMFF container),
// rather than arbitrary bytes: most rounds land inside a mostly-valid stream,
// which is where a best-effort demuxer's resynchronisation and bounds live.
// The assertions are the reader rule's hard floor: no panic, no hang, and a
// bounded peak allocation for the whole run.
//
// The recorded damage script is `[offset: u32][kind: u8][length: u16]`
// followed by replacement bytes; a short script still applies its first
// fields (a truncation needs only the offset).

const TS: usize = 0;
const PS: usize = 1;
const FLV: usize = 2;
const MKV: usize = 3;

const ALLOC_LIMIT: usize = 128 * 1024 * 1024;
const MAX_LEN: usize = 1 << 20;

fn seeds() -> &'static [Vec<u8>; 4] {
    static SEEDS: OnceLock<[Vec<u8>; 4]> = OnceLock::new();
    SEEDS.get_or_init(|| {
        [
            test_fixtures::read("ts/h264-aac.ts"),
            test_fixtures::read("ps/h264-ac3.ps"),
            test_fixtures::read("flv/av.flv"),
            test_fixtures::read("mkv/h264-aac.mkv"),
        ]
    })
}

fn be(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
}

fn mutate(seed: &[u8], script: &[u8]) -> Vec<u8> {
    let mut out = seed.to_vec();
    if script.len() < 4 {
        return out;
    }
    let at = usize::try_from(be(&script[..4])).unwrap_or(usize::MAX) % (out.len() + 1);
    let kind = script.get(4).copied().unwrap_or(0) % 5;
    let len = script
        .get(5..7)
        .map_or(16, |bytes| (usize::try_from(be(bytes)).unwrap_or(16) % 4096).max(1));
    match kind {
        0 => out.truncate(at),
        1 => {
            let end = at.saturating_add(len).min(out.len());
            out[at..end].fill(0);
        }
        2 => {
            let end = at.saturating_add(len).min(out.len());
            let bytes = script.get(4..).unwrap_or(&[0x5A]);
            for (index, slot) in out[at..end].iter_mut().enumerate() {
                *slot = bytes[index % bytes.len()];
            }
        }
        3 => {
            let end = at.saturating_add(len).min(out.len());
            out.drain(at..end);
        }
        _ => {
            let take = len.min(script.len().saturating_sub(7));
            let bytes = script.get(7..7 + take).unwrap_or(&[]);
            out.splice(at..at, bytes.iter().copied());
        }
    }
    out.truncate(out.len().min(MAX_LEN));
    out
}

fn drive_ts(bytes: &[u8]) {
    let mut demux = transmux::StreamingTsDemux::new();
    let _ = demux.feed(bytes);
    while demux.poll_event().is_some() {}
    let _ = demux.finish();
    while demux.poll_event().is_some() {}
}

fn drive_ps(bytes: &[u8]) {
    use broadcast_common::Stage;
    use broadcast_common::stage::Timestamp;
    let mut demux = transmux::StreamingPsDemux::new();
    for piece in bytes.chunks(4096) {
        let _ = demux.feed(piece, Timestamp::ZERO);
    }
    let _ = demux.finish();
    while demux.poll().is_some() {}
}

fn drive_flv(bytes: &[u8]) {
    let mut demux = transmux::StreamingFlvDemux::new();
    let _ = demux.feed(bytes);
    while demux.poll_event().is_some() {}
    let _ = demux.finish();
    while demux.poll_event().is_some() {}
}

fn drive_mkv(bytes: &[u8]) {
    use broadcast_common::Stage;
    use broadcast_common::stage::Timestamp;
    use transmux::matroska_demux::{MatroskaDemux, MatroskaDemuxConfig, MatroskaInput};
    let total = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut demux =
        MatroskaDemux::seekable(MatroskaDemuxConfig::default().with_total_len(Some(total)));
    for _ in 0..64 {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        let slice = bytes.get(start..end).unwrap_or(&[]).to_vec();
        if demux
            .feed(
                MatroskaInput::At {
                    offset,
                    bytes: slice.into(),
                },
                Timestamp::ZERO,
            )
            .is_err()
        {
            break;
        }
        while demux.poll().is_some() {}
    }
    let _ = demux.finish();
    while demux.poll().is_some() {}
}

fuzz_target!(|data: (u8, &[u8])| {
    let (selector, script) = data;
    let which = usize::from(selector) % 4;
    let seed = &seeds()[which];
    let bytes = mutate(seed, script);
    let (_, snapshot) = test_alloc::ThreadCounting::measure(|| match which {
        TS => drive_ts(&bytes),
        PS => drive_ps(&bytes),
        FLV => drive_flv(&bytes),
        MKV => drive_mkv(&bytes),
        _ => {}
    });
    assert!(
        snapshot.bytes <= ALLOC_LIMIT,
        "format {which}: allocated {} bytes, over {ALLOC_LIMIT}",
        snapshot.bytes
    );
});

//! A damaged TS must not pass the `Media::degraded.is_empty()` validity gate.
//!
//! Downstream report (zenith-tv, transmux 0.27.0): zeroing byte 0 (the sync
//! byte) of every 7th packet of a real 720p50 H.264 + AC-3 + 3x MP2 capture
//! made `TsDemux::unpackage` lose samples while `degraded` stayed empty,
//! because `TsResync` silently skipped the 188 bytes and the batch wrapper
//! also threw every `InputDegraded` event away. Fixtures load only through
//! `test_fixtures`.

use broadcast_common::Unpackage;
use transmux::TsDemux;
use transmux::ir::{DemuxEvent, InputDegradation};
use transmux::media::Media;
use transmux::ts_demux::StreamingTsDemux;

const PKT: usize = 188;
const ORF1: &str = "ts/orf1-dvbs2-raw-zenith-tv.ts";
/// The downstream repro's exact damage: packet indices 56392, 56399, ...,
/// 56791 (58 packets).
fn damaged_indices() -> Vec<usize> {
    (56392..56792).step_by(7).collect()
}

/// Packets of lead-in kept before the first damaged packet in a PR run.
const PR_LEAD_IN_PACKETS: usize = 2000;
/// Packets kept after the last damaged packet in a PR run.
const PR_TAIL_PACKETS: usize = 500;
/// Largest capture the no-false-positive sweep reads in a PR run;
/// `RUST_BROADCAST_SOAK` sweeps every capture.
const PR_SWEEP_MAX_BYTES: u64 = 6 * 1024 * 1024;

fn soak() -> bool {
    std::env::var_os("RUST_BROADCAST_SOAK").is_some()
}

/// The ORF1 capture and the packet index of its first byte: the whole file
/// under `RUST_BROADCAST_SOAK`, otherwise just the damaged region and its
/// lead-in (PAT/PMT repeat, so every window demuxes cleanly).
fn orf1_window(clean: Vec<u8>) -> (Vec<u8>, usize) {
    if soak() {
        return (clean, 0);
    }
    let idx = damaged_indices();
    let base = idx[0] - PR_LEAD_IN_PACKETS;
    let end = idx[idx.len() - 1] + 1 + PR_TAIL_PACKETS;
    (clean[base * PKT..end * PKT].to_vec(), base)
}

fn counts(m: &Media) -> Vec<usize> {
    m.tracks.iter().map(|t| t.samples.len()).collect()
}

fn sync_losses(m: &Media) -> Vec<(u64, u64)> {
    m.degraded
        .iter()
        .filter_map(|d| match d {
            InputDegradation::SyncLoss {
                offset,
                dropped_bytes,
            } => Some((*offset, *dropped_bytes)),
            _ => None,
        })
        .collect()
}

fn non_sync_loss(m: &Media) -> Vec<InputDegradation> {
    m.degraded
        .iter()
        .filter(|d| !matches!(d, InputDegradation::SyncLoss { .. }))
        .cloned()
        .collect()
}

fn demux(bytes: &[u8]) -> Media {
    TsDemux::new().unpackage(bytes).expect("demux")
}

#[test]
fn orf1_broken_sync_bytes_are_reported_and_counts_are_clean_minus_lost() {
    let Some(path) = test_fixtures::require(ORF1, "ts_sync_loss_degraded") else {
        return;
    };
    let (clean, base) = orf1_window(std::fs::read(path).unwrap());
    let clean_media = demux(&clean);
    assert!(
        clean_media.degraded.is_empty(),
        "clean capture: {:?}",
        clean_media.degraded
    );

    let idx: Vec<usize> = damaged_indices().into_iter().map(|i| i - base).collect();
    assert_eq!(idx.len(), 58);
    let mut damaged = clean.clone();
    for &i in &idx {
        damaged[i * PKT] = 0x00;
    }
    let media = demux(&damaged);

    // Samples really are lost ...
    assert_ne!(counts(&media), counts(&clean_media));
    // ... and exactly as many as if the 58 packets were simply absent.
    let removed: Vec<u8> = clean
        .chunks(PKT)
        .enumerate()
        .filter(|(i, _)| !idx.contains(i))
        .flat_map(|(_, c)| c.iter().copied())
        .collect();
    let removed_media = demux(&removed);
    assert_eq!(counts(&media), counts(&removed_media), "clean minus lost");

    // ... and every loss is reported once, pointing at the damaged packet.
    let expected: Vec<(u64, u64)> = idx
        .iter()
        .map(|&i| ((i * PKT) as u64, PKT as u64))
        .collect();
    assert_eq!(sync_losses(&media), expected);
    // Everything else (the continuity gaps the lost packets cause) is what the
    // stream with the packets removed reports.
    assert_eq!(non_sync_loss(&media), non_sync_loss(&removed_media));
    assert!(!media.degraded.is_empty());
}

#[test]
fn streaming_demux_reports_the_same_losses_at_any_chunking() {
    let Some(path) = test_fixtures::require(ORF1, "ts_sync_loss_degraded") else {
        return;
    };
    let mut damaged = std::fs::read(path).unwrap();
    let idx = damaged_indices();
    for &i in &idx {
        damaged[i * PKT] = 0x00;
    }
    // Only the damaged region plus lead-in: keeps the test fast.
    let damaged = &damaged[(56392 - 2000) * PKT..(56792 + 50) * PKT];
    let base = 56392 - 2000;
    for chunk in [4099usize, 188 * 33, 1_000_003] {
        let mut d = StreamingTsDemux::new();
        let mut losses = Vec::new();
        for c in damaged.chunks(chunk) {
            d.feed(c);
        }
        d.finish();
        while let Some(ev) = d.poll_event() {
            if let DemuxEvent::InputDegraded {
                kind:
                    InputDegradation::SyncLoss {
                        offset,
                        dropped_bytes,
                    },
                provenance,
                ..
            } = ev
            {
                losses.push((offset, dropped_bytes, provenance.packet_index.unwrap()));
            }
        }
        assert_eq!(losses.len(), 58, "chunk {chunk}");
        for (k, (&i, l)) in idx.iter().zip(&losses).enumerate() {
            assert_eq!(l.0, ((i - base) * PKT) as u64, "chunk {chunk} offset");
            assert_eq!(l.1, PKT as u64);
            // packets emitted before it: everything before, minus earlier drops
            assert_eq!(l.2, (i - base - k) as u64, "chunk {chunk} packet_index");
        }
    }
}

fn small() -> Vec<u8> {
    test_fixtures::read("ts/h264_aac.ts")
}

#[test]
fn garbage_between_packets_is_one_sync_loss_and_no_samples_lost() {
    let clean = small();
    let clean_media = demux(&clean);
    assert!(clean_media.degraded.is_empty());
    let at = 100 * PKT;
    let mut bytes = clean[..at].to_vec();
    bytes.extend_from_slice(&[0x11; 10]);
    bytes.extend_from_slice(&clean[at..]);
    let media = demux(&bytes);
    assert_eq!(counts(&media), counts(&clean_media));
    assert_eq!(
        media.degraded,
        vec![InputDegradation::SyncLoss {
            offset: at as u64,
            dropped_bytes: 10
        }]
    );
}

#[test]
fn truncated_trailing_packet_is_reported() {
    let clean = small();
    let mut bytes = clean.clone();
    bytes.extend_from_slice(&clean[..100]);
    let media = demux(&bytes);
    assert_eq!(
        media.degraded,
        vec![InputDegradation::TruncatedPacket {
            offset: clean.len() as u64,
            dropped_bytes: 100
        }]
    );
}

#[test]
fn never_resynchronised_tail_is_reported_at_finish() {
    let clean = small();
    let mut bytes = clean.clone();
    bytes.extend_from_slice(&[0x22; 400]);
    let media = demux(&bytes);
    assert_eq!(
        media.degraded,
        vec![InputDegradation::SyncLoss {
            offset: clean.len() as u64,
            dropped_bytes: 400
        }]
    );
}

#[test]
fn leading_junk_before_the_first_lock_is_not_degradation() {
    let clean = small();
    let mut bytes = vec![0x33; 77];
    bytes.extend_from_slice(&clean);
    let media = demux(&bytes);
    assert!(media.degraded.is_empty(), "{:?}", media.degraded);
}

#[test]
fn transport_error_indicator_reaches_media_degraded() {
    let mut bytes = small();
    // A packet carrying PES payload of the video PID well after the PSI.
    let i = 200 * PKT;
    bytes[i + 1] |= 0x80;
    let media = demux(&bytes);
    assert!(
        media.degraded.contains(&InputDegradation::TransportError),
        "{:?}",
        media.degraded
    );
}

#[test]
fn damage_never_panics() {
    let clean = small();
    for step in [3usize, 7, 11] {
        let mut b = clean.clone();
        for i in (0..b.len() / PKT).step_by(step) {
            b[i * PKT] = (i % 251) as u8;
            b[i * PKT + 1 + i % 100] ^= 0xFF;
        }
        let _ = demux(&b);
        let _ = demux(&b[..b.len() - 61]);
    }
}

fn all_ts() -> Vec<std::path::PathBuf> {
    let mut v = test_fixtures::private_files("ts", &["ts"]);
    let mut stack = vec![test_fixtures::root().join("ts")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "ts") {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

/// No false positives: report (never assert on) what every committed TS says,
/// but a capture must never report a sync loss unless it is in the allowlist
/// of captures that really contain broken sync.
#[test]
fn committed_fixtures_have_no_unexpected_sync_loss() {
    for p in all_ts() {
        if !soak() && std::fs::metadata(&p).unwrap().len() > PR_SWEEP_MAX_BYTES {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        let m = demux(&bytes);
        let s = m
            .degraded
            .iter()
            .filter(|d| {
                matches!(
                    d,
                    InputDegradation::SyncLoss { .. } | InputDegradation::TruncatedPacket { .. }
                )
            })
            .count();
        eprintln!(
            "{}: degraded={} (sync/truncated={s})",
            p.display(),
            m.degraded.len()
        );
        assert_eq!(s, 0, "{}: {:?}", p.display(), m.degraded);
    }
}

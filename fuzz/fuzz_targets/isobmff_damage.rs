#![no_main]

//! A real file, damaged at a deterministic place: no panic, bounded memory, and
//! an error only when nothing can be played.
//!
//! The input *is* the file image, so the seed corpus is the committed fixtures.
//! The damage site and kind are derived from the image's own bytes, which keeps
//! every corpus entry a valid, differently-damaged file on each mutation.

use broadcast_core::damage::DamageKind;
use isobmff::{Limits, Reader, ReaderConfig, SampleOrder, Source, Step};
use libfuzzer_sys::fuzz_target;

/// Mutation kinds the target applies.
const KINDS: u8 = 6;
/// Longest run a zeroed or randomised mutation covers.
const MAX_RUN: usize = 64;
/// Caps small enough that a hostile table cannot allocate its way out of the
/// fuzzer's budget.
fn fuzz_limits() -> Limits {
    Limits::default()
        .with_max_depth(16)
        .with_max_moov_bytes(1 << 20)
        .with_max_samples_per_track(1 << 16)
        .with_max_sample_bytes(1 << 22)
        .with_max_fragment_bytes(1 << 20)
        .with_max_metadata_item_bytes(1 << 20)
}

/// A hostile file cannot describe more steps than this.
const MAX_STEPS: usize = 1 << 20;

/// A cheap rolling hash, so the damage site depends on every input byte.
fn hash(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value
}

/// One damaged copy of `data`.
fn damage(data: &[u8]) -> Vec<u8> {
    let seed = hash(data);
    let mut out = data.to_vec();
    if out.len() < 12 {
        return out;
    }
    let kind = (seed % u64::from(KINDS)) as u8;
    let offset = usize::try_from(seed >> 8).unwrap_or(usize::MAX) % out.len();
    let len = 1 + usize::try_from(seed >> 20).unwrap_or(usize::MAX) % MAX_RUN;
    let end = offset.saturating_add(len).min(out.len());
    let value = u32::try_from(seed >> 32).unwrap_or(u32::MAX);
    match kind {
        0 => {
            out.truncate(offset.max(1));
        }
        1 => out[offset..end].fill(0),
        2 => {
            let mut noise = seed;
            for byte in &mut out[offset..end] {
                noise = noise.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                *byte = (noise >> 33) as u8;
            }
        }
        3 => {
            let head = offset.min(out.len().saturating_sub(4));
            out[head..head + 4].copy_from_slice(&value.to_be_bytes());
        }
        4 => {
            let head = offset.min(out.len().saturating_sub(4));
            let count = u32::from_be_bytes(
                out[head..head + 4]
                    .try_into()
                    .unwrap_or([0u8; core::mem::size_of::<u32>()]),
            );
            let changed = count.wrapping_add(value & 0xFFFF);
            out[head..head + 4].copy_from_slice(&changed.to_be_bytes());
        }
        _ => {
            let head = offset.min(out.len().saturating_sub(4));
            out[head..head + 4].copy_from_slice(b"free");
        }
    }
    out
}

/// Whether an error is one the lane's rule allows: nothing could be played.
fn allowed(error: &isobmff::Error) -> bool {
    match error {
        isobmff::Error::InvalidInput(text) => {
            text.contains("no moov") || text.contains("no track")
        }
        isobmff::Error::LimitExceeded { .. } => true,
        _ => false,
    }
}

/// Drive the reader over one image, asserting the rule and monotonic timing.
fn drive(data: &[u8]) {
    let config = ReaderConfig::default()
        .with_total_len(Some(data.len() as u64))
        .with_order(SampleOrder::DecodeTime)
        .with_source(Source::Seekable)
        .with_limits(fuzz_limits());
    let mut reader = Reader::new(config);
    let mut last: Option<(u32, u64)> = None;
    let mut tracks = 0usize;
    let mut samples = 0usize;
    let mut dropped = false;
    for _ in 0..MAX_STEPS {
        match reader.step() {
            Ok(Step::Need(range)) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                if let Err(error) = reader.supply(range.offset, data.get(start..end).unwrap_or(&[]))
                {
                    assert!(allowed(&error), "a supply error must be structural: {error:?}");
                    return;
                }
            }
            Ok(Step::Tracks(table)) => tracks += table.len(),
            Ok(Step::Sample(sample)) => {
                samples += 1;
                if let Some((track, time)) = last
                    && track == sample.track.value()
                {
                    assert!(
                        sample.decode_time >= time,
                        "decode times are monotonic on one track: {time} then {}",
                        sample.decode_time
                    );
                }
                last = Some((sample.track.value(), sample.decode_time));
            }
            Ok(Step::Damage(damage)) => {
                if matches!(
                    damage.kind(),
                    DamageKind::MediaUnitDropped | DamageKind::TrackDropped
                ) {
                    dropped = true;
                }
            }
            Ok(Step::Done) => break,
            Ok(_) => {}
            Err(error) => {
                assert!(
                    allowed(&error) || tracks == 0,
                    "a file with a track table must not be refused: {error:?} \
                     (tracks={tracks}, samples={samples}, dropped={dropped})"
                );
                return;
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    drive(&damage(data));
});

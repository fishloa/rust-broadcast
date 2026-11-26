//! One `mdat` claimed by many `moof`s must be copied once, not once per
//! fragment (audit 20 findings 13 / 57).
//!
//! `Mp4Demux` bounds each `moof`'s sample total against the input length,
//! which a per-fragment check cannot see through: hundreds of tiny `moof`s that
//! each point `tfhd.base_data_offset` at one `mdat` pass every one of those
//! checks, and the demuxer copies that payload again for each of them. The
//! bound that holds is the run's own total, so a fragment past it is refused
//! (reported as a degradation, with the samples before it kept).
#![cfg(feature = "std")]
use test_harness::collect::Collected as Media;

const MOOFS: usize = 1000;
const TRACK_ID: u32 = 1;
const SAMPLES_PER_MOOF: u32 = 8;
const SAMPLE_SIZE: u32 = 32;
const MDAT_PAYLOAD: usize = 256;

const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 0x000001;
const TRUN_DATA_OFFSET: u32 = 0x000001;
const TRUN_SAMPLE_SIZE: u32 = 0x000200;

fn box_bytes(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(payload);
    out
}

fn container(ty: &[u8; 4], kids: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for k in kids {
        payload.extend_from_slice(k);
    }
    box_bytes(ty, &payload)
}

fn mfhd() -> Vec<u8> {
    let mut p = vec![0u8; 4];
    p.extend_from_slice(&1u32.to_be_bytes());
    box_bytes(b"mfhd", &p)
}

fn tfhd_at(base: u64) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&TFHD_BASE_DATA_OFFSET_PRESENT.to_be_bytes());
    p.extend_from_slice(&TRACK_ID.to_be_bytes());
    p.extend_from_slice(&base.to_be_bytes());
    box_bytes(b"tfhd", &p)
}

fn trun_len() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&(TRUN_DATA_OFFSET | TRUN_SAMPLE_SIZE).to_be_bytes());
    p.extend_from_slice(&SAMPLES_PER_MOOF.to_be_bytes());
    p.extend_from_slice(&0i32.to_be_bytes());
    for _ in 0..SAMPLES_PER_MOOF {
        p.extend_from_slice(&SAMPLE_SIZE.to_be_bytes());
    }
    box_bytes(b"trun", &p)
}

fn moof_at(base: u64) -> Vec<u8> {
    container(
        b"moof",
        &[mfhd(), container(b"traf", &[tfhd_at(base), trun_len()])],
    )
}

/// Everything up to and including the top-level `moov`, so the file keeps a
/// real track table while the fragments are the ones this test writes.
fn prefix_through_moov(file: &[u8]) -> Vec<u8> {
    let mut offset = 0usize;
    while offset + 8 <= file.len() {
        let size = u32::from_be_bytes([
            file[offset],
            file[offset + 1],
            file[offset + 2],
            file[offset + 3],
        ]) as usize;
        if &file[offset + 4..offset + 8] == b"moov" {
            return file[..offset + size].to_vec();
        }
        offset += size;
    }
    panic!("no moov in the fixture");
}

fn hostile_file() -> Vec<u8> {
    let fixture = test_fixtures::path("mp4/cenc-layouts/clear-ffmpeg-default.mp4");
    let bytes = std::fs::read(&fixture).expect("read the clear fMP4 fixture");
    let mut file = prefix_through_moov(&bytes);

    let moof_len = moof_at(0).len();
    let mdat_payload = file.len() + moof_len + 8;

    file.extend_from_slice(&moof_at(mdat_payload as u64));
    file.extend_from_slice(&box_bytes(b"mdat", &vec![0xAA; MDAT_PAYLOAD]));
    for _ in 1..MOOFS {
        file.extend_from_slice(&moof_at(mdat_payload as u64));
    }
    // Resolves the queued fragments; the sample bytes they name stay in the
    // first `mdat`.
    file.extend_from_slice(&box_bytes(b"mdat", &[]));
    file
}

/// The old per-`moof` bound let this file copy `MOOFS x MDAT_PAYLOAD` bytes out
/// of a far smaller input. The run-wide budget stops it at the input length,
/// and every refused fragment is reported rather than dropped silently.
#[test]
fn many_moofs_over_one_mdat_are_copied_once() {
    let file = hostile_file();
    let media: Media = test_harness::collect::media_from_mp4(
        file.as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("the prefix and the first fragment are well formed");
    let copied: usize = media
        .tracks
        .iter()
        .flat_map(|t| t.samples.iter())
        .map(|s| s.data.len())
        .sum();
    assert!(copied > 0, "the first fragment's samples must survive");
    assert!(
        copied <= file.len(),
        "copied {copied} sample bytes out of a {}-byte input",
        file.len()
    );
    assert!(
        !media.degraded.is_empty(),
        "every fragment past the budget must be reported as degraded"
    );
}

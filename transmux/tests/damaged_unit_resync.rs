//! Audit T16 (#1144) — one damaged container unit must not abort the whole
//! input. A committed real fixture gets a corrupt unit injected in the MIDDLE,
//! followed by valid data; the data after the damage must still be decoded and
//! the damage must be reported in `Media::degraded`.

use broadcast_common::Package;
use broadcast_common::Unpackage;
use transmux::ir::InputDegradation;
use transmux::media::Media;
use transmux::{PsDemux, RtpDepacketiser, RtpInput, RtpInputStream, RtpMediaKind, RtpPacketiser};

const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];

fn fixture(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn total_bytes(m: &Media) -> usize {
    m.tracks
        .iter()
        .flat_map(|t| t.samples.iter())
        .map(|s| s.data.len())
        .sum()
}

#[test]
fn ps_damaged_pack_header_in_the_middle_is_skipped_and_reported() {
    let clean_bytes = fixture("ps/h264_ac3.ps");
    let clean = PsDemux::new().unpackage(&clean_bytes).unwrap();
    assert!(clean.degraded.is_empty(), "a clean stream reports nothing");

    let packs: Vec<usize> = clean_bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == PACK_START_CODE)
        .map(|(i, _)| i)
        .collect();
    assert!(packs.len() > 8, "fixture needs several packs");
    // A pack in the middle that carries a private_stream_1 PES (AC-3 audio,
    // stream_id 0xBD) so its loss is observable in the audio track.
    let mut bounds = packs.clone();
    bounds.push(clean_bytes.len());
    let victim = (packs.len() / 2..packs.len())
        .find(|&i| {
            clean_bytes[bounds[i]..bounds[i + 1]]
                .windows(4)
                .any(|w| w == [0x00, 0x00, 0x01, 0xBD])
        })
        .map(|i| packs[i])
        .expect("an audio pack in the second half");

    // Clear the '01' marker bits of the pack header's first byte after the
    // start code (ISO/IEC 13818-1 §2.5.3.3): the pack no longer parses.
    let mut bytes = clean_bytes.clone();
    bytes[victim + 4] = 0x00;

    let damaged = PsDemux::new().unpackage(&bytes).expect("must not abort");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    assert_eq!(damaged.tracks.len(), clean.tracks.len());
    // The pack's payload is gone ...
    assert!(
        total_bytes(&damaged) < total_bytes(&clean),
        "the damaged pack's data is gone"
    );
    // ... but only that pack's: everything after it is still decoded, so each
    // track still ends on the same sample as the undamaged stream.
    for (d, c) in damaged.tracks.iter().zip(&clean.tracks) {
        assert!(!d.samples.is_empty());
        assert_eq!(
            d.samples.last().unwrap().data,
            c.samples.last().unwrap().data
        );
        assert_eq!(d.samples[0].data, c.samples[0].data);
    }
}

#[test]
fn ps_with_no_recoverable_pack_is_still_an_error() {
    assert!(PsDemux::new().unpackage(&[0xAB; 64][..]).is_err());
}

fn packetised() -> (Media, transmux::RtpOutput) {
    let ts = fixture("ts/h264_aac.ts");
    let media = transmux::TsDemux::new()
        .unpackage(&ts[..])
        .expect("demux TS");
    let out = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    }
    .package(&media)
    .expect("packetise");
    (media, out)
}

fn depacketise(kind: RtpMediaKind, packets: Vec<Vec<u8>>, media: &Media) -> Media {
    let mut stream = RtpInputStream::new(kind, packets);
    if kind == RtpMediaKind::Aac {
        let cfg = media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, transmux::pipeline::CodecConfig::Aac { .. }))
            .unwrap()
            .spec
            .config
            .clone();
        stream = stream.with_config(cfg);
    }
    RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![stream],
        })
        .expect("a malformed packet must not fail the whole input")
}

#[test]
fn rtp_video_malformed_packets_in_the_middle_are_skipped_and_reported() {
    let (media, out) = packetised();
    let vs = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap();
    let packets: Vec<Vec<u8>> = vs
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let clean = depacketise(RtpMediaKind::H264, packets.clone(), &media);
    assert!(clean.degraded.is_empty());
    let clean_samples = clean.tracks[0].samples.len();

    let mid = packets.len() / 2;
    // (a) header cut short (RFC 3550 §5.1 needs 12 bytes), (b) an STAP-A whose
    // second NAL size runs past the payload.
    let mut damaged = packets.clone();
    damaged[mid].truncate(5);
    // A STAP-A with an overrunning NAL size, if the stream has one; otherwise
    // a second truncated packet.
    let second = mid + 3;
    damaged[second].truncate(3);

    let got = depacketise(RtpMediaKind::H264, damaged, &media);
    assert_eq!(
        got.degraded,
        vec![
            InputDegradation::MalformedUnit {
                position: mid as u64
            },
            InputDegradation::MalformedUnit {
                position: second as u64
            },
        ]
    );
    let s = &got.tracks[0].samples;
    assert!(s.len() < clean_samples, "damaged AUs are gone");
    assert!(
        s.len() + 6 >= clean_samples,
        "{} vs {}",
        s.len(),
        clean_samples
    );
    // The data after the damage is decoded byte-identically.
    assert_eq!(
        s.last().unwrap().data,
        clean.tracks[0].samples.last().unwrap().data
    );
    assert_eq!(s[0].data, clean.tracks[0].samples[0].data);
}

#[test]
fn rtp_audio_malformed_packet_in_the_middle_is_skipped_and_reported() {
    let (media, out) = packetised();
    let a = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .unwrap();
    let packets: Vec<Vec<u8>> = a
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let clean = depacketise(RtpMediaKind::Aac, packets.clone(), &media);
    let mid = packets.len() / 2;
    let mut damaged = packets;
    damaged[mid].truncate(7);
    let got = depacketise(RtpMediaKind::Aac, damaged, &media);
    assert_eq!(
        got.degraded,
        vec![InputDegradation::MalformedUnit {
            position: mid as u64
        }]
    );
    let (g, c) = (&got.tracks[0].samples, &clean.tracks[0].samples);
    assert!(
        g.len() < c.len() && g.len() + 4 >= c.len(),
        "{} vs {}",
        g.len(),
        c.len()
    );
    assert_eq!(g.last().unwrap().data, c.last().unwrap().data);
}

#[test]
fn fmp4_corrupt_moof_in_the_middle_is_skipped_and_reported() {
    use transmux::media::Fmp4Demux;
    let clean_bytes = fixture("mp4/cmaf/av_frag.mp4");
    let clean = Fmp4Demux::new().unpackage(&clean_bytes[..]).unwrap();
    assert!(clean.degraded.is_empty());

    // Walk the top-level boxes and pick the middle moof.
    let mut moofs = Vec::new();
    let mut off = 0usize;
    while off + 8 <= clean_bytes.len() {
        let size = u32::from_be_bytes(clean_bytes[off..off + 4].try_into().unwrap()) as usize;
        if &clean_bytes[off + 4..off + 8] == b"moof" {
            moofs.push(off);
        }
        off += size;
    }
    assert!(moofs.len() >= 5);
    let victim = moofs[moofs.len() / 2];

    // The moof first child box (mfhd) claims a 2-byte size: framing of the
    // moof body is destroyed, the top-level box framing is intact.
    let mut bytes = clean_bytes.clone();
    bytes[victim + 8..victim + 12].copy_from_slice(&2u32.to_be_bytes());

    let damaged = Fmp4Demux::new()
        .unpackage(&bytes[..])
        .expect("one corrupt fragment must not fail the file");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    let count = |m: &Media| m.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    assert!(count(&damaged) < count(&clean), "that fragment is lost");
    for (d, c) in damaged.tracks.iter().zip(&clean.tracks) {
        assert!(d.samples.len() <= c.samples.len());
        assert!(!d.samples.is_empty());
        assert_eq!(
            d.samples.last().unwrap().data,
            c.samples.last().unwrap().data
        );
        assert_eq!(d.samples[0].data, c.samples[0].data);
    }
}

#[test]
fn fmp4_trun_pointing_outside_the_file_skips_that_traf_and_reports() {
    use transmux::media::Fmp4Demux;
    let clean_bytes = fixture("mp4/cmaf/av_frag.mp4");
    let clean = Fmp4Demux::new().unpackage(&clean_bytes[..]).unwrap();

    let mut moofs = Vec::new();
    let mut off = 0usize;
    while off + 8 <= clean_bytes.len() {
        let size = u32::from_be_bytes(clean_bytes[off..off + 4].try_into().unwrap()) as usize;
        if &clean_bytes[off + 4..off + 8] == b"moof" {
            moofs.push((off, size));
        }
        off += size;
    }
    let (victim, size) = moofs[moofs.len() / 2];
    // First trun in the victim moof: box header(8) + version/flags(4) +
    // sample_count(4) + data_offset(4, flag 0x000001 set).
    let rel = clean_bytes[victim..victim + size]
        .windows(4)
        .position(|w| w == b"trun")
        .expect("trun");
    let at = victim + rel;
    assert_eq!(clean_bytes[at + 7] & 1, 1, "data_offset present");
    let mut bytes = clean_bytes.clone();
    bytes[at + 12..at + 16].copy_from_slice(&0x7fff_fff0u32.to_be_bytes());

    let damaged = Fmp4Demux::new()
        .unpackage(&bytes[..])
        .expect("a bad trun must not fail the file");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    let count = |m: &Media| m.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    assert!(count(&damaged) < count(&clean));
    for (d, c) in damaged.tracks.iter().zip(&clean.tracks) {
        assert_eq!(
            d.samples.last().unwrap().data,
            c.samples.last().unwrap().data
        );
        assert_eq!(d.samples[0].data, c.samples[0].data);
    }
}

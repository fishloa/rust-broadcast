//! Deterministic allocation-count evidence for the transmux optimization sweep
//! (issues #1079/#1080/#1081, audit r04-O* / r05-O*). A counting
//! `#[global_allocator]` (thread-local counters, see `alloc_measurement.rs` for
//! why) measures the calling thread only. Counts, never timings.

use test_alloc::ThreadCounting;

#[global_allocator]
static GLOBAL: ThreadCounting = ThreadCounting::new();

/// Run `f`, returning `(result, allocation_count, allocated_bytes)` for this thread.
fn measure<R>(f: impl FnOnce() -> R) -> (R, usize, usize) {
    let (r, snap) = ThreadCounting::measure(f);
    (r, snap.allocs, snap.bytes)
}

/// Run `f`, returning `(result, total_allocated_bytes)` for this thread — the
/// copy-count proxy for "how many times were the payload bytes duplicated".
fn bytes_allocated<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let (r, _, b) = measure(f);
    (r, b)
}

mod ps_demux_copies {
    use super::bytes_allocated;
    use broadcast_common::Unpackage;
    use transmux::PsDemux;

    fn load(rel: &str) -> Option<Vec<u8>> {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures")
            .join(rel);
        std::fs::read(p).ok()
    }

    /// r04-O6: the PS demuxer copied every video byte three times (ES concat, the
    /// per-AU `to_vec`, the Annex-B -> length-prefixed re-encode). Total bytes
    /// allocated while demuxing, as a multiple of the input size, is the
    /// deterministic copy-count proxy.
    #[test]
    fn ps_demux_copies_of_the_payload() {
        for (rel, budget_x100) in [
            ("ps/ffmpeg-h264-noaud.ps", H264_BUDGET_X100),
            ("ps/ffmpeg-mpeg2video-mp2.ps", MPEG2_BUDGET_X100),
        ] {
            let Some(bytes) = load(rel) else {
                eprintln!("SKIPPED {rel}");
                continue;
            };
            let (media, allocated) =
                bytes_allocated(|| PsDemux::new().unpackage(bytes.as_slice()).unwrap());
            drop(media);
            let x100 = allocated * 100 / bytes.len();
            eprintln!("ps demux {rel}: {allocated} bytes allocated = {x100}/100 x input");
            assert!(x100 <= budget_x100, "{rel}: {x100}/100 x input");
        }
    }

    const H264_BUDGET_X100: usize = 658; // 664 before r04-O6, 652 after (midway)
    const MPEG2_BUDGET_X100: usize = 337; // 370 before r04-O6, 305 after (midway)
}

mod ts_demux_copies {
    use super::bytes_allocated;
    use broadcast_common::Unpackage;
    use transmux::TsDemux;

    /// r04-O10: each completed PES payload was copied (`pes.payload.to_vec()`)
    /// before being probed/pushed, though a live track only reads it. Total
    /// bytes allocated while demuxing, as a multiple of the input size.
    #[test]
    fn ts_demux_copies_of_the_payload() {
        let p =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
        let Ok(bytes) = std::fs::read(p) else {
            eprintln!("SKIPPED ts/h264_aac.ts");
            return;
        };
        let (media, allocated) =
            bytes_allocated(|| TsDemux::new().unpackage(bytes.as_slice()).unwrap());
        drop(media);
        let x100 = allocated * 100 / bytes.len();
        eprintln!("ts demux h264_aac.ts: {allocated} bytes allocated = {x100}/100 x input");
        assert!(x100 <= TS_BUDGET_X100, "{x100}/100 x input");
    }

    const TS_BUDGET_X100: usize = 976; // 1008 before r04-O10, 944 after (midway)
}

mod rtmp_read_chunks {
    use super::bytes_allocated;
    use transmux::rtmp::{Message, read_chunks, write_chunks};

    const BODY_LEN: usize = 200_000;
    const CHUNK_SIZE: usize = 128;

    /// r04-O7: `read_chunks` cloned the whole per-csid context (including the
    /// in-progress `partial` buffer) for every chunk: O(n^2) bytes for one large
    /// message. Total bytes allocated, as a multiple of the message size.
    #[test]
    fn read_chunks_does_not_clone_the_partial_message_per_chunk() {
        let msg = Message {
            csid: 6,
            message_type_id: 9,
            message_stream_id: 1,
            timestamp: 40,
            body: (0..BODY_LEN).map(|i| (i % 251) as u8).collect(),
        };
        let wire = write_chunks(std::slice::from_ref(&msg), CHUNK_SIZE).unwrap();
        let (out, allocated) = bytes_allocated(|| read_chunks(&wire).unwrap());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].body, msg.body);
        let x = allocated / BODY_LEN;
        eprintln!("read_chunks: {allocated} bytes allocated = {x} x message size");
        assert!(x <= READ_BUDGET_X, "{x} x message size");
    }

    const READ_BUDGET_X: usize = 50; // 2343 before r04-O7, 2 after (still bites O(n^2))
}

mod rtp_stream_copies {
    use super::bytes_allocated;
    use broadcast_common::{Package, Unpackage};
    use transmux::pipeline::CodecConfig;
    use transmux::rtp::RtpMediaKind;
    use transmux::{RtpPacketiser, RtpStreamDepacketiser, RtpStreamTrack, TsDemux};

    /// r04-O7: every in-order RTP packet was copied twice on the way into the AU
    /// buffer (`SeqState::admit`, then `push_one`). Bytes allocated by the push
    /// loop, as a multiple of the packet bytes pushed.
    #[test]
    fn rtp_push_copies_each_packet_once() {
        let p =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
        let Ok(data) = std::fs::read(p) else {
            eprintln!("SKIPPED ts/h264_aac.ts");
            return;
        };
        let media = TsDemux::new().unpackage(data.as_slice()).unwrap();
        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .unwrap();
        let video_only = media
            .clone()
            .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .unwrap();
        let mut pk = RtpPacketiser {
            mtu: 1400,
            ssrc: 0x1234_5678,
            ..RtpPacketiser::default()
        };
        let out = pk.package(&video_only).unwrap();
        let stream = out
            .streams
            .iter()
            .find(|s| s.kind == RtpMediaKind::H264)
            .unwrap();
        let packets: Vec<Vec<u8>> = stream
            .packets
            .iter()
            .map(|p| p.as_contiguous().to_vec())
            .collect();
        let total: usize = packets.iter().map(Vec::len).sum();
        let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            video.spec.config.clone(),
            90_000,
        )]);
        let (samples, allocated) = bytes_allocated(|| {
            let mut n = 0;
            for pkt in &packets {
                n += d.push(1, pkt).unwrap().len();
            }
            n
        });
        assert!(samples > 0);
        let x100 = allocated * 100 / total;
        eprintln!("rtp push: {allocated} bytes allocated = {x100}/100 x packet bytes");
        assert!(x100 <= RTP_BUDGET_X100, "{x100}/100 x packet bytes");
    }

    const RTP_BUDGET_X100: usize = 697; // 747 before r04-O7, 647 after (midway)
}

#[cfg(feature = "cenc")]
mod cenc_decrypt_copies {
    use super::bytes_allocated;
    use transmux::CencDecryptor;

    fn load() -> Option<Vec<u8>> {
        let p =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/mp4/cenc.mp4");
        std::fs::read(p).ok()
    }

    /// r05-O2: `from_fmp4` copied the whole file and `demux` copied every sample
    /// again. Bytes allocated by harvest + demux, as a multiple of the file size.
    #[test]
    fn harvest_and_demux_do_not_copy_the_file_and_every_sample() {
        let Some(file) = load() else {
            eprintln!("SKIPPED mp4/cenc.mp4");
            return;
        };
        let (n, allocated) = bytes_allocated(|| {
            let dec = CencDecryptor::from_fmp4(&file).unwrap();
            dec.demux().unwrap().tracks.len()
        });
        assert!(n > 0);
        let x100 = allocated * 100 / file.len();
        eprintln!("cenc harvest+demux: {allocated} bytes allocated = {x100}/100 x file");
        assert!(x100 <= CENC_BUDGET_X100, "{x100}/100 x file");

        // Sharing the buffer: nothing of the file or its samples is copied.
        let shared = bytes::Bytes::from(file.clone());
        let (n, allocated) = bytes_allocated(|| {
            let dec = CencDecryptor::from_fmp4_bytes(shared).unwrap();
            dec.demux().unwrap().tracks.len()
        });
        assert!(n > 0);
        let x100 = allocated * 100 / file.len();
        eprintln!("cenc harvest+demux (shared): {allocated} bytes = {x100}/100 x file");
        assert!(x100 <= CENC_SHARED_BUDGET_X100, "{x100}/100 x file");
    }

    const CENC_BUDGET_X100: usize = 167; // 214 before r05-O2, 121 after (midway)
    const CENC_SHARED_BUDGET_X100: usize = 60; // 21 after (from_fmp4_bytes is new; old code copied >= 100)
}

mod muxer_buffering {
    use super::bytes_allocated;
    use broadcast_common::{Package, Unpackage};
    use transmux::{MkvMux, ProgressiveMux, TsDemux, TsMux};

    /// r05-O4: the muxers buffered their whole output through 2-4 full copies.
    /// Bytes allocated by `package`, as a multiple of the produced output size.
    #[test]
    fn muxers_copy_the_output_few_times() {
        let p =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
        let Ok(data) = std::fs::read(p) else {
            eprintln!("SKIPPED ts/h264_aac.ts");
            return;
        };
        let media = TsDemux::new().unpackage(data.as_slice()).unwrap();
        let cases: [(&str, Vec<u8>, usize); 3] = {
            let (prog, a1) = bytes_allocated(|| ProgressiveMux::new(true).package(&media).unwrap());
            let (mkv, a2) = bytes_allocated(|| MkvMux::new().package(&media).unwrap());
            let (ts, a3) = bytes_allocated(|| TsMux::new().package(&media).unwrap());
            [("progressive", prog, a1), ("mkv", mkv, a2), ("ts", ts, a3)]
        };
        for (name, out, allocated) in cases {
            let x100 = allocated * 100 / out.len();
            eprintln!("mux {name}: {allocated} bytes allocated = {x100}/100 x output");
            let budget = match name {
                "progressive" => PROGRESSIVE_BUDGET_X100,
                "mkv" => MKV_BUDGET_X100,
                _ => TS_BUDGET_X100,
            };
            assert!(x100 <= budget, "{name}: {x100}/100 x output");
        }
    }

    const PROGRESSIVE_BUDGET_X100: usize = 316; // 412 before r05-O4, 220 after (midway)
    const MKV_BUDGET_X100: usize = 719; // 1145 before r05-O4, 293 after (midway)
    const TS_BUDGET_X100: usize = 655; // 770 before r05-O4, 540 after (midway)
}

mod segmenter_cuts {
    use super::measure;
    use transmux::{
        AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, Sample,
        Segmenter, TrackSpec,
    };

    fn spec() -> TrackSpec {
        let record = AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 66,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: vec![AvcSps(vec![
                0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24,
            ])],
            pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        };
        TrackSpec::new(
            1,
            90_000,
            CodecConfig::Avc {
                config: AVCConfigurationBox::new(record),
                width: 16,
                height: 16,
            },
        )
    }

    /// Push `cuts` one-sample GOPs (each keyframe closes the previous segment) and
    /// return `(segments_ready, allocs)` over the push loop only.
    fn run(cuts: usize) -> (usize, usize) {
        let mut seg = Segmenter::new(vec![spec()], 1000, 0.5).unwrap();
        let samples: Vec<Sample> = (0..cuts)
            .map(|i| {
                let dts = (i as i64) * 90_000;
                Sample::new(vec![0u8; 4], Some(dts), Some(dts), Some(90_000), true)
            })
            .collect();
        let (n, allocs, _) = measure(|| {
            for s in samples {
                seg.push(1, s).unwrap();
            }
            seg.take_ready().len()
        });
        (n, allocs)
    }

    /// r05-O5: the dead init-change detection rebuilt the whole `moov` on every
    /// cut (109.1 allocations per cut before, 18.1 after); the budget fails if an
    /// init-segment build comes back.
    #[test]
    fn per_cut_allocations_exclude_an_init_segment_build() {
        let (n1, a1) = run(8);
        let (n2, a2) = run(16);
        assert!(n1 >= 6 && n2 >= 14, "segments cut: {n1}, {n2}");
        let per_cut = (a2 - a1) as f64 / (n2 - n1) as f64;
        eprintln!("segmenter allocations per cut: {per_cut:.1}");
        assert!(per_cut <= PER_CUT_BUDGET, "per cut {per_cut}");
    }

    const PER_CUT_BUDGET: f64 = 60.0;
}

mod hvcc_prealloc {
    use super::measure;
    use broadcast_common::Parse;
    use transmux::HEVCDecoderConfigurationRecord;

    /// A 22-byte hvcC header (`lengthSizeMinusOne = 3`) declaring 255 arrays whose
    /// first array declares 65535 NAL units, followed by no bytes at all.
    fn hostile() -> Vec<u8> {
        let mut b = vec![0u8; 22];
        b[0] = 1;
        b[21] = 0x03;
        b.push(0xFF); // numOfArrays
        b.push(0x20); // array_completeness / NAL_unit_type
        b.extend_from_slice(&[0xFF, 0xFF]); // numNalus
        b
    }

    /// r04-O4: `Vec::with_capacity(num_nalus)` / `(num_arrays)` straight from the
    /// wire reserved ~1.5 MiB before a single NAL was read (1,581,000 bytes before,
    /// 32 after). Bounded by body length.
    #[test]
    fn declared_counts_do_not_drive_preallocation() {
        let bytes = hostile();
        let (r, allocs, bytes_alloc) = measure(|| HEVCDecoderConfigurationRecord::parse(&bytes));
        assert!(r.is_err());
        eprintln!("hvcC hostile counts: {allocs} allocs, {bytes_alloc} bytes");
        assert!(bytes_alloc <= HVCC_BYTES_BUDGET, "{bytes_alloc} bytes");
    }

    const HVCC_BYTES_BUDGET: usize = 4096;
}

mod heaac_signaling_allocs {
    use super::measure;
    use broadcast_common::Parse;
    use transmux::AudioSpecificConfig;

    /// r04-O2: `heaac_signaling()` (and `rfc6381()` through it) re-serialized the
    /// ASC into a fresh `Vec` on every call: 1 allocation per call before, 0 after.
    #[test]
    fn heaac_signaling_and_rfc6381_do_not_allocate_per_call() {
        // AAC-LC, 44.1 kHz, stereo.
        let asc = AudioSpecificConfig::parse(&[0x12, 0x10]).unwrap();
        let (sig, allocs, _) = measure(|| asc.heaac_signaling());
        assert_eq!(sig.effective_aot, 2);
        eprintln!("heaac_signaling: {allocs} allocs");
        assert_eq!(allocs, 0);
        let (s, allocs, _) = measure(|| asc.rfc6381());
        assert_eq!(s, "mp4a.40.2");
        // The only allocation is the returned `String` itself.
        assert!(allocs <= 1, "rfc6381 allocs: {allocs}");
    }
}

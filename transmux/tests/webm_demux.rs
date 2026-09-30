//! WebM demuxer integration tests (issue #471) — oracle-driven.
//!
//! The fixture `fixtures/webm/vp9_opus.webm` is a real WebM (VP9 video + Opus
//! audio, ffmpeg-produced, no lacing). Its per-frame ffprobe oracle lives in
//! `fixtures/webm/vp9_opus.packets.csv`:
//!
//! ```text
//! codec_type,stream_index,pts,dts,duration,size,keyframe(K=1)
//! ```
//!
//! `pts`/`dts`/`duration` are in **milliseconds** (ffprobe stream time_base
//! 1/1000). The `size` column is the per-frame coded byte length — the strong
//! bite: wrong VINT/block parsing yields wrong sizes.
//!
//! The demuxer emits an IR whose timescale is milliseconds
//! ([`transmux::webm_demux::IR_TIMESCALE`] = 1000), so a sample's reconstructed
//! PTS (cumulative sample durations from the track's first block) is directly in
//! the oracle's units. For **video** the reconstructed PTS equals the oracle PTS
//! exactly. For **audio**, ffprobe shifts the presentation time back by the Opus
//! codec delay (pre-skip 312 samples @ 48 kHz ≈ 7 ms — see the fixture's
//! `initial_padding`), so `recon_pts == oracle_pts + AUDIO_CODEC_DELAY_MS`; the
//! test documents and applies that constant.

use broadcast_common::Package;
use transmux::pipeline::CodecConfig;
use transmux::webm_demux::WebmDemux;
use transmux::{CmafMux, Media, parse_box};

/// Path to the committed WebM fixture, relative to this crate's manifest dir.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp9_opus.webm"
);
/// Path to the ffprobe per-frame oracle CSV.
const ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp9_opus.packets.csv"
);

/// Opus codec delay ffprobe applies to audio presentation times, in ms.
/// (pre-skip 312 samples @ 48 kHz = 6.5 ms, rounded up to 7 ms.)
const AUDIO_CODEC_DELAY_MS: i64 = 7;

/// One ffprobe oracle row.
#[derive(Debug)]
struct OracleRow {
    codec_type: String,
    pts: i64,
    size: usize,
    keyframe: bool,
}

fn load_oracle() -> Vec<OracleRow> {
    let text = std::fs::read_to_string(ORACLE).expect("read oracle csv");
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        rows.push(OracleRow {
            codec_type: f[0].to_string(),
            pts: f[2].trim().parse().unwrap(),
            size: f[5].trim().parse().unwrap(),
            keyframe: f[6].trim() == "1",
        });
    }
    rows
}

fn oracle_for<'a>(rows: &'a [OracleRow], codec_type: &str) -> Vec<&'a OracleRow> {
    rows.iter().filter(|r| r.codec_type == codec_type).collect()
}

fn demux() -> Media {
    let bytes = std::fs::read(FIXTURE).expect("read webm fixture");
    let mut d = WebmDemux::new();
    d.demux(&bytes).expect("demux webm")
}

/// Test 1 — stream enumeration: exactly 2 tracks, track 0 = VP9 video,
/// track 1 = Opus audio.
#[test]
fn enumerates_two_tracks_vp9_and_opus() {
    let m = demux();
    assert_eq!(m.tracks.len(), 2, "expected exactly 2 tracks");
    assert!(
        matches!(m.tracks[0].spec.config, CodecConfig::Vp9 { .. }),
        "track 0 must be VP9 video, got {:?}",
        m.tracks[0].spec.config
    );
    assert!(
        matches!(m.tracks[1].spec.config, CodecConfig::Opus { .. }),
        "track 1 must be Opus audio, got {:?}",
        m.tracks[1].spec.config
    );
}

/// Test 2 — frame counts + per-frame size oracle. Each demuxed sample's coded
/// byte length must equal the oracle `size` column, in order (a wrong block /
/// VINT parse yields wrong sizes).
#[test]
fn frame_counts_and_sizes_match_oracle() {
    let m = demux();
    let oracle = load_oracle();
    let vid_oracle = oracle_for(&oracle, "video");
    let aud_oracle = oracle_for(&oracle, "audio");

    assert_eq!(vid_oracle.len(), 50, "oracle sanity: 50 video frames");
    assert_eq!(aud_oracle.len(), 101, "oracle sanity: 101 audio frames");

    let vid = &m.tracks[0];
    let aud = &m.tracks[1];
    assert_eq!(vid.samples.len(), 50, "video sample count");
    assert_eq!(aud.samples.len(), 101, "audio sample count");

    for (i, (s, o)) in vid.samples.iter().zip(vid_oracle.iter()).enumerate() {
        assert_eq!(
            s.data.len(),
            o.size,
            "video sample {i} byte length must equal oracle size"
        );
    }
    for (i, (s, o)) in aud.samples.iter().zip(aud_oracle.iter()).enumerate() {
        assert_eq!(
            s.data.len(),
            o.size,
            "audio sample {i} byte length must equal oracle size"
        );
    }
}

/// Test 3 — timestamp + keyframe oracle. Reconstructed PTS (cumulative sample
/// durations from the track's first block, ms) matches the oracle, and video
/// keyframe flags match the oracle keyframe column. Audio is all-sync and its
/// oracle PTS is codec-delay-shifted (see [`AUDIO_CODEC_DELAY_MS`]).
#[test]
fn timestamps_and_keyframes_match_oracle() {
    let m = demux();
    let oracle = load_oracle();
    let vid_oracle = oracle_for(&oracle, "video");
    let aud_oracle = oracle_for(&oracle, "audio");

    // Video: IR timescale is ms, first block PTS is 0 → recon_pts == oracle pts.
    let vid = &m.tracks[0];
    let mut acc = 0i64;
    for (i, s) in vid.samples.iter().enumerate() {
        assert_eq!(
            acc, vid_oracle[i].pts,
            "video sample {i} reconstructed PTS must equal oracle"
        );
        assert_eq!(
            s.flags.is_sync, vid_oracle[i].keyframe,
            "video sample {i} keyframe flag must equal oracle"
        );
        acc += s.duration.unwrap_or(0) as i64;
    }

    // Audio: recon_pts == oracle_pts + codec delay; every sample is sync.
    let aud = &m.tracks[1];
    let mut acc = 0i64;
    for (i, s) in aud.samples.iter().enumerate() {
        assert_eq!(
            acc,
            aud_oracle[i].pts + AUDIO_CODEC_DELAY_MS,
            "audio sample {i} reconstructed PTS must equal oracle + codec delay"
        );
        assert!(s.flags.is_sync, "audio sample {i} must be a sync sample");
        acc += s.duration.unwrap_or(0) as i64;
    }
}

/// Test 4 — Opus config from OpusHead. The built `dOps` carries the channel
/// count + pre-skip parsed from the CodecPrivate `OpusHead` (mono here). The
/// magic was actually parsed: pre-skip 312 is a real OpusHead value that only
/// appears if the header was read, not defaulted (a default `OpusSpecificBox`
/// has pre-skip 0).
#[test]
fn opus_config_from_opus_head() {
    let m = demux();
    let CodecConfig::Opus {
        config,
        channel_count,
        sample_rate,
        ..
    } = &m.tracks[1].spec.config
    else {
        panic!("track 1 is not Opus");
    };
    // Channels: the OpusHead channel count matches the fixture's Audio/Channels (1).
    assert_eq!(*channel_count, 1, "Opus channel count from Audio element");
    assert_eq!(
        config.output_channel_count, 1,
        "dOps OutputChannelCount from OpusHead"
    );
    // Pre-skip is a real OpusHead value (312), proving the header was parsed.
    assert_eq!(
        config.pre_skip, 312,
        "dOps PreSkip from OpusHead (not defaulted)"
    );
    assert_eq!(*sample_rate, 48_000, "Opus playback rate is always 48 kHz");
    assert_eq!(config.version, 1, "OpusHead version");
}

/// Test 5 — output path works: the demuxed IR muxes to fMP4 with a `vp09` video
/// sample entry carrying a `vpcC` box and an `Opus` audio sample entry carrying a
/// `dOps` box (proves the IR configs are complete enough to mux).
#[test]
fn demuxed_ir_muxes_to_fmp4_with_vp09_and_opus() {
    let m = demux();
    let mut mux = CmafMux::default();
    let fmp4 = mux.package(&m).expect("mux demuxed IR to CMAF");

    // Scan for the fourccs anywhere in the emitted bytes (they only appear if the
    // sample entries + config boxes were built from the IR configs).
    assert!(
        contains(&fmp4, b"vp09"),
        "fMP4 must carry a vp09 sample entry"
    );
    assert!(contains(&fmp4, b"vpcC"), "fMP4 must carry a vpcC box");
    assert!(
        contains(&fmp4, b"Opus"),
        "fMP4 must carry an Opus sample entry"
    );
    assert!(contains(&fmp4, b"dOps"), "fMP4 must carry a dOps box");

    // Structurally: the leading box parses (it is a real ISOBMFF file, not noise).
    let (bx, _) = parse_box(&fmp4).expect("first box parses");
    assert_eq!(&bx.header.box_type.0, b"ftyp", "fMP4 begins with ftyp");
}

/// Find `needle` anywhere in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// r04-W40: laced blocks must be unlaced, not rejected or concatenated
// ---------------------------------------------------------------------------

/// Rebuild `fixtures/webm/vp9_opus.webm` with the audio frames of several
/// consecutive blocks packed into one laced block, in each of the three lacing
/// encodings of RFC 9559 §12.
///
/// The fixture's own EBML header, `Info` and `Tracks` bytes are reused
/// verbatim, so the track set and codec configs are the real ones; only the
/// Segment body is rewritten, with one Cluster holding the video blocks (each
/// still one frame, unlaced) and the audio blocks grouped three-up into laced
/// blocks. Consequently every frame the demuxer recovers must be byte-identical
/// to the unlaced demux of the same file — lacing is a transport choice, not
/// content — and the sample *count* must match too.
mod laced_mkv {
    /// `SimpleBlock` element id (RFC 9559 §12).
    pub const SIMPLE_BLOCK: u32 = 0xA3;
    /// `Cluster` element id.
    pub const CLUSTER: u32 = 0x1F43_B675;
    /// `Cluster`/`Timestamp`.
    pub const CLUSTER_TIMESTAMP: u32 = 0xE7;
    /// `Segment` element id.
    pub const SEGMENT: u32 = 0x1853_8067;
    /// Lacing-mode bits: Xiph.
    pub const XIPH: u8 = 0x02;
    /// Lacing-mode bits: EBML.
    pub const EBML: u8 = 0x06;
    /// Lacing-mode bits: fixed-size.
    pub const FIXED: u8 = 0x04;

    /// A block lifted from the real fixture: its track, relative timestamp,
    /// flags and single frame.
    pub struct Raw {
        pub track: u64,
        pub rel_ts: i16,
        pub flags: u8,
        pub frame: Vec<u8>,
    }

    fn read_vint(buf: &[u8], off: usize) -> Option<(usize, usize)> {
        let first = *buf.get(off)?;
        let mut mask = 0x80u8;
        let mut len = 1usize;
        while first & mask == 0 {
            mask >>= 1;
            len += 1;
            if len > 8 {
                return None;
            }
        }
        let mut v = usize::from(first & (mask - 1));
        for k in 1..len {
            v = (v << 8) | usize::from(*buf.get(off + k)?);
        }
        Some((v, len))
    }

    fn read_id(buf: &[u8], off: usize) -> Option<(u32, usize)> {
        let first = *buf.get(off)?;
        let mut mask = 0x80u8;
        let mut len = 1usize;
        while first & mask == 0 {
            mask >>= 1;
            len += 1;
            if len > 4 {
                return None;
            }
        }
        let mut id = 0u32;
        for k in 0..len {
            id = (id << 8) | u32::from(*buf.get(off + k)?);
        }
        Some((id, len))
    }

    /// Encode an EBML element id with its canonical width.
    fn encode_id(id: u32) -> Vec<u8> {
        let bytes = id.to_be_bytes();
        let first = bytes.iter().position(|&b| b != 0).unwrap_or(3);
        bytes[first..].to_vec()
    }

    /// Encode an EBML size VINT using the shortest form for `value`.
    fn encode_size(mut value: usize) -> Vec<u8> {
        let mut len = 1usize;
        while value >= (1usize << (7 * len)) - 1 {
            len += 1;
        }
        let mut out = vec![0u8; len];
        for k in (0..len).rev() {
            out[k] = (value & 0xFF) as u8;
            value >>= 8;
        }
        out[0] |= 1u8 << (8 - len);
        out
    }

    /// A complete element: id + size + body.
    pub fn element(id: u32, body: &[u8]) -> Vec<u8> {
        let mut out = encode_id(id);
        out.extend(encode_size(body.len()));
        out.extend_from_slice(body);
        out
    }

    /// The EBML header element (id `0x1A45DFA3`), taken verbatim from a real
    /// file so its contents are not synthesised.
    pub fn ebml_header(data: &[u8]) -> Vec<u8> {
        let (size, len) = read_vint(data, 4).expect("EBML header size");
        data[..4 + len + size].to_vec()
    }

    /// The body of the Segment's child elements with the given id, verbatim
    /// (used for `Info` and `Tracks`).
    pub fn segment_child(data: &[u8], want: u32) -> Vec<u8> {
        let (hsize, hlen) = read_vint(data, 4).expect("EBML header size");
        let seg = 4 + hlen + hsize;
        let (ssize, slen) = read_vint(data, seg + 4).expect("segment size");
        let mut i = seg + 4 + slen;
        let end = i + ssize;
        while i < end {
            let (id, id_len) = read_id(data, i).expect("child id");
            let (size, size_len) = read_vint(data, i + id_len).expect("child size");
            let child_end = i + id_len + size_len + size;
            if id == want {
                return data[i..child_end].to_vec();
            }
            i = child_end;
        }
        panic!("element {want:#x} not found in the fixture's Segment");
    }

    /// Every `SimpleBlock` in the fixture, in file order, with its single
    /// frame (the fixture is unlaced).
    pub fn blocks(data: &[u8]) -> Vec<Raw> {
        let (hsize, hlen) = read_vint(data, 4).expect("EBML header size");
        let seg = 4 + hlen + hsize;
        let (ssize, slen) = read_vint(data, seg + 4).expect("segment size");
        let mut i = seg + 4 + slen;
        let end = i + ssize;
        let mut out = Vec::new();
        while i < end {
            let (id, id_len) = read_id(data, i).expect("child id");
            let (size, size_len) = read_vint(data, i + id_len).expect("child size");
            let body = &data[i + id_len + size_len..i + id_len + size_len + size];
            if id == CLUSTER {
                let mut j = 0usize;
                while j < body.len() {
                    let (cid, cl) = read_id(body, j).expect("cluster child id");
                    let (csize, csl) = read_vint(body, j + cl).expect("cluster child size");
                    let cbody = &body[j + cl + csl..j + cl + csl + csize];
                    if cid == SIMPLE_BLOCK {
                        let (track, tl) = read_vint(cbody, 0).expect("track vint");
                        let track = track as u64;
                        let rel_ts = i16::from_be_bytes([cbody[tl], cbody[tl + 1]]);
                        out.push(Raw {
                            track,
                            rel_ts,
                            flags: cbody[tl + 2],
                            frame: cbody[tl + 3..].to_vec(),
                        });
                    }
                    j += cl + csl + csize;
                }
            }
            i += id_len + size_len + size;
        }
        out
    }

    /// Encode one `SimpleBlock` payload: track VINT, rel-ts, flags, then either
    /// the single frame or the laced frame group.
    fn block_payload(track: u64, rel_ts: i16, flags: u8, laced_body: &[u8]) -> Vec<u8> {
        let mut out = encode_size(track as usize);
        out.extend_from_slice(&rel_ts.to_be_bytes());
        out.push(flags);
        out.extend_from_slice(laced_body);
        out
    }

    /// Lace `frames` per `mode`, returning the part of the payload that follows
    /// the flags byte (frame count first).
    pub fn lace(frames: &[Vec<u8>], mode: u8) -> Vec<u8> {
        let mut out = Vec::new();
        out.push((frames.len() - 1) as u8);
        let sizes: Vec<usize> = frames.iter().map(|f| f.len()).collect();
        match mode {
            XIPH => {
                for &n in &sizes[..sizes.len() - 1] {
                    let mut n = n;
                    while n >= 255 {
                        out.push(255);
                        n -= 255;
                    }
                    out.push(n as u8);
                }
            }
            EBML => {
                out.extend(encode_size(sizes[0]));
                for w in sizes[..sizes.len() - 2].iter().zip(&sizes[1..]) {
                    let delta = *w.1 as i64 - *w.0 as i64;
                    out.extend(encode_signed_vint(delta));
                }
            }
            FIXED => {
                assert!(
                    sizes.iter().all(|&s| s == sizes[0]),
                    "fixed lacing requires equal-size frames"
                );
            }
            _ => panic!("unknown lacing mode"),
        }
        for f in frames {
            out.extend_from_slice(f);
        }
        out
    }

    /// The `(flags, payload)` of the first laced block in `data`'s Cluster, with
    /// `payload` being the bytes after the flags byte.
    ///
    /// Walks the real element structure — Cluster, then its children — rather
    /// than scanning the whole file for the `0xA3` id, which occurs by chance
    /// inside coded audio. This reads the wire so the test can prove the emitted
    /// block carries the mode it asked for: Xiph and EBML produce identical
    /// *frames* for some size sequences, so the demuxed samples cannot tell them
    /// apart.
    pub fn first_laced_block(data: &[u8]) -> Option<(u8, Vec<u8>)> {
        let (hsize, hlen) = read_vint(data, 4)?;
        let seg = 4 + hlen + hsize;
        let (ssize, slen) = read_vint(data, seg + 4)?;
        let mut i = seg + 4 + slen;
        let end = i + ssize;
        while i < end {
            let (id, id_len) = read_id(data, i)?;
            let (size, size_len) = read_vint(data, i + id_len)?;
            let body = &data[i + id_len + size_len..i + id_len + size_len + size];
            if id == CLUSTER {
                let mut j = 0usize;
                while j < body.len() {
                    let (cid, cl) = read_id(body, j)?;
                    let (csize, csl) = read_vint(body, j + cl)?;
                    let cbody = &body[j + cl + csl..j + cl + csl + csize];
                    if cid == SIMPLE_BLOCK {
                        let (_, track_len) = read_vint(cbody, 0)?;
                        let flags = cbody[track_len + 2];
                        if flags & 0x06 != 0 {
                            return Some((flags, cbody[track_len + 3..].to_vec()));
                        }
                    }
                    j += cl + csl + csize;
                }
            }
            i += id_len + size_len + size;
        }
        None
    }

    /// Split a laced block *payload* (the bytes after the flags byte) back into
    /// its frames, independently of the crate's own unlacer, so the encoding
    /// round-trips. `payload[0]` is the count byte.
    pub fn unlace_payload(payload: &[u8], flags: u8) -> Vec<Vec<u8>> {
        let mode = flags & 0x06;
        let count = usize::from(payload[0]) + 1;
        let mut body = &payload[1..];
        let mut sizes = Vec::with_capacity(count);
        match mode {
            XIPH => {
                for _ in 0..count - 1 {
                    let mut n = 0usize;
                    loop {
                        let byte = body[0];
                        body = &body[1..];
                        n += usize::from(byte);
                        if byte != 0xFF {
                            break;
                        }
                    }
                    sizes.push(n);
                }
            }
            EBML => {
                let (first, used) = read_vint(body, 0).expect("initial EBML size");
                body = &body[used..];
                let mut prev = first;
                sizes.push(prev);
                for _ in 1..count - 1 {
                    let (delta, used) = read_signed_vint(body).expect("EBML delta");
                    body = &body[used..];
                    prev = (prev as i64 + delta) as usize;
                    sizes.push(prev);
                }
            }
            FIXED => {
                let each = body.len() / count;
                sizes.extend(core::iter::repeat_n(each, count - 1));
            }
            _ => panic!("no lacing"),
        }
        let mut frames = Vec::with_capacity(count);
        for n in sizes {
            frames.push(body[..n].to_vec());
            body = &body[n..];
        }
        frames.push(body.to_vec());
        frames
    }

    /// Decode a signed EBML VINT (the inverse of [`encode_signed_vint`]).
    fn read_signed_vint(buf: &[u8]) -> Option<(i64, usize)> {
        let first = *buf.first()?;
        let mut mask = 0x80u8;
        let mut len = 1usize;
        while first & mask == 0 {
            mask >>= 1;
            len += 1;
            if len > 8 {
                return None;
            }
        }
        let (raw, _) = read_vint(buf, 0)?;
        let bias = 1i64 << (7 * len - 1);
        Some((raw as i64 - (bias - 1), len))
    }

    /// The first signed EBML-lacing delta of a laced payload (the byte after the
    /// initial size VINT).
    pub fn first_ebml_delta(payload: &[u8]) -> i64 {
        let body = &payload[1..];
        let (_, used) = read_vint(body, 0).expect("initial EBML size");
        read_signed_vint(&body[used..]).expect("first delta").0
    }

    /// Encode a signed EBML VINT (EBML lacing delta), biased per RFC 8794 §4.
    fn encode_signed_vint(value: i64) -> Vec<u8> {
        for len in 1..=8usize {
            let bias = 1i64 << (7 * len - 1);
            let encoded = value + (bias - 1);
            if (0..bias).contains(&encoded) {
                let mut out = vec![0u8; len];
                let mut v = encoded as u64;
                for k in (0..len).rev() {
                    out[k] = (v & 0xFF) as u8;
                    v >>= 8;
                }
                out[0] |= 1u8 << (8 - len);
                return out;
            }
        }
        panic!("EBML lacing delta out of range");
    }

    /// Build the whole file: the fixture's EBML header + Segment(Info, Tracks,
    /// one Cluster of `blocks`). `group` is how many consecutive audio blocks
    /// (matching `audio_track`) are packed into each laced block.
    pub fn build(data: &[u8], audio_track: u64, group: usize, mode: u8) -> Vec<u8> {
        let blocks = blocks(data);
        let cluster_ts = 0i16;

        let mut cluster_body = element(CLUSTER_TIMESTAMP, &[0u8]);
        let mut audio_run: Vec<Raw> = Vec::new();
        for b in blocks {
            if b.track == audio_track {
                audio_run.push(b);
                continue;
            }
            // Flush any pending audio run first so order is preserved.
            flush_laced(&mut cluster_body, &mut audio_run, group, mode);
            cluster_body.extend(element(
                SIMPLE_BLOCK,
                &block_payload(b.track, b.rel_ts, b.flags, &b.frame),
            ));
        }
        flush_laced(&mut cluster_body, &mut audio_run, group, mode);

        let mut segment_body = segment_child(data, 0x1549_A966); // Info
        segment_body.extend(segment_child(data, 0x1654_AE6B)); // Tracks
        segment_body.extend(element(CLUSTER, &cluster_body));

        let mut out = ebml_header(data);
        out.extend(element(SEGMENT, &segment_body));
        let _ = cluster_ts;
        out
    }

    /// Build a whole file whose single audio block is a fixed-laced group of
    /// three copies of `seed` (fixed lacing requires equal-size frames, and
    /// this fixture's Opus frames are variable-length).
    pub fn fixed_three(data: &[u8], seed: &Raw) -> Vec<u8> {
        let first = seed;
        let frames = vec![first.frame.clone(); 3];
        let body = lace(&frames, FIXED);
        let flags = (first.flags & !0x06) | FIXED;
        let payload = block_payload(first.track, first.rel_ts, flags, &body);
        let mut cluster_body = element(CLUSTER_TIMESTAMP, &[0u8]);
        cluster_body.extend(element(SIMPLE_BLOCK, &payload));

        let mut segment_body = segment_child(data, 0x1549_A966);
        segment_body.extend(segment_child(data, 0x1654_AE6B));
        segment_body.extend(element(CLUSTER, &cluster_body));

        let mut out = ebml_header(data);
        out.extend(element(SEGMENT, &segment_body));
        out
    }

    /// Replace (or insert) `DefaultDuration` in the TrackEntry for `track`,
    /// returning a new `Tracks` element body. `ns` of 0 removes it, giving the
    /// no-`DefaultDuration` case.
    pub fn set_default_duration(data: &[u8], track: u64, ns: u64) -> Vec<u8> {
        // `segment_child` returns the whole Tracks element; strip its header so
        // the walk below starts at its first TrackEntry.
        let tracks_full = segment_child(data, 0x1654_AE6B);
        let (_, hlen) = read_id(&tracks_full, 0).expect("tracks id");
        let (tsize, slen) = read_vint(&tracks_full, hlen).expect("tracks size");
        let tracks = tracks_full[hlen + slen..hlen + slen + tsize].to_vec();
        // Walk TrackEntry children; rebuild each, injecting the element into
        // the matching one.
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < tracks.len() {
            let (id, id_len) = read_id(&tracks, i).expect("tracks child id");
            let (size, size_len) = read_vint(&tracks, i + id_len).expect("tracks child size");
            let body_start = i + id_len + size_len;
            let body_end = body_start + size;
            if id != 0xAE {
                out.extend_from_slice(&tracks[i..body_end]);
            } else {
                let entry = &tracks[body_start..body_end];
                // Read this entry's TrackNumber.
                let mut n = 0usize;
                let mut number = None;
                while n < entry.len() {
                    let (cid, cl) = read_id(entry, n).expect("entry child id");
                    let (csize, csl) = read_vint(entry, n + cl).expect("entry child size");
                    let cb = &entry[n + cl + csl..n + cl + csl + csize];
                    if cid == 0xD7 {
                        number = Some(cb.iter().fold(0u64, |a, &b| (a << 8) | u64::from(b)));
                    }
                    n += cl + csl + csize;
                }
                let mut new_entry = Vec::new();
                let mut m = 0usize;
                while m < entry.len() {
                    let (cid, cl) = read_id(entry, m).expect("entry child id");
                    let (csize, csl) = read_vint(entry, m + cl).expect("entry child size");
                    let end = m + cl + csl + csize;
                    if cid != DEFAULT_DURATION_ID {
                        new_entry.extend_from_slice(&entry[m..end]);
                    }
                    m = end;
                }
                if number == Some(track) && ns != 0 {
                    let be = ns.to_be_bytes();
                    let first = be.iter().position(|&b| b != 0).unwrap_or(7);
                    new_entry.extend(element(DEFAULT_DURATION_ID, &be[first..]));
                }
                out.extend(element(0xAE, &new_entry));
            }
            i = body_end;
        }
        element(0x1654_AE6B, &out)
    }

    /// `DefaultDuration` element id (TrackEntry child).
    pub const DEFAULT_DURATION_ID: u32 = 0x23_E3_83;

    /// Rebuild the file with a custom `Tracks` (see [`set_default_duration`])
    /// and the given per-block payloads: `(track, rel_ts, flags, frames)`.
    pub fn build_custom(
        data: &[u8],
        tracks: &[u8],
        cluster_blocks: &[(u64, i16, u8, Vec<Vec<u8>>)],
    ) -> Vec<u8> {
        let mut cluster_body = element(CLUSTER_TIMESTAMP, &[0u8]);
        for (track, rel_ts, flags, frames) in cluster_blocks {
            // Honour the caller's requested lacing mode rather than picking one
            // from the frame sizes: choosing FIXED whenever the sizes happened to
            // match meant EBML lacing — the mode mkvmerge uses for most audio —
            // was never actually exercised by these tests.
            // A caller that sets no lacing bits on a multi-frame block gets
            // Xiph, the encoding the fixture's own frames can always carry.
            let requested = match flags & 0x06 {
                0 => XIPH,
                mode => mode,
            };
            let body = if frames.len() == 1 {
                frames[0].clone()
            } else {
                lace(frames, requested)
            };
            let block_flags = if frames.len() == 1 {
                *flags & !0x06
            } else {
                (*flags & !0x06) | requested
            };
            cluster_body.extend(element(
                SIMPLE_BLOCK,
                &block_payload(*track, *rel_ts, block_flags, &body),
            ));
        }
        let mut segment_body = segment_child(data, 0x1549_A966);
        segment_body.extend_from_slice(tracks);
        segment_body.extend(element(CLUSTER, &cluster_body));

        let mut out = ebml_header(data);
        out.extend(element(SEGMENT, &segment_body));
        out
    }

    fn flush_laced(cluster_body: &mut Vec<u8>, run: &mut Vec<Raw>, group: usize, mode: u8) {
        for chunk in run.chunks(group) {
            let first = &chunk[0];
            let frame_flags = first.flags & !0x06;
            let laced = if chunk.len() == 1 {
                block_payload(first.track, first.rel_ts, frame_flags, &first.frame)
            } else {
                let frames: Vec<Vec<u8>> = chunk.iter().map(|b| b.frame.clone()).collect();
                let body = lace(&frames, mode);
                block_payload(first.track, first.rel_ts, frame_flags | mode, &body)
            };
            cluster_body.extend(element(SIMPLE_BLOCK, &laced));
        }
        run.clear();
    }
}

/// Demux the real fixture with its audio blocks relaced three-up, in each
/// lacing encoding, and require the recovered frames to be identical to the
/// unlaced demux's — same count, same bytes, same order.
///
/// The unlaced demux of the same file is the oracle, so nothing here
/// re-implements the parser: lacing must be invisible in the IR.
#[test]
fn laced_blocks_are_unlaced_like_the_unlaced_fixture() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let audio_track = 2u64;
    // The oracle is the *same rewritten file* with group = 1 — i.e. the blocks
    // carried over one per block, unlaced. Rebuilding it that way exercises the
    // identical layout, so a difference can only come from the lacing itself,
    // never from an extraction artefact of this test's own walker.
    let mut reference_demux = WebmDemux::new();
    let reference = reference_demux
        .demux(&laced_mkv::build(&data, audio_track, 1, laced_mkv::XIPH))
        .expect("unlaced rebuild demuxes");
    let reference_audio = reference
        .tracks
        .iter()
        .find(|t| u64::from(t.spec.track_id) == audio_track)
        .expect("audio track");
    assert!(
        reference_audio.samples.len() > 20,
        "the rebuilt file must carry a real audio track, got {} samples",
        reference_audio.samples.len()
    );

    for (mode, name) in [(laced_mkv::XIPH, "Xiph"), (laced_mkv::EBML, "EBML")] {
        let bytes = laced_mkv::build(&data, audio_track, 3, mode);
        let mut d = WebmDemux::new();
        let laced = d
            .demux(&bytes)
            .unwrap_or_else(|e| panic!("{name}-laced WebM must demux, got {e:?}"));

        assert_eq!(
            laced.tracks.len(),
            reference.tracks.len(),
            "{name} lacing must not change the track set"
        );
        for (a, b) in laced.tracks.iter().zip(reference.tracks.iter()) {
            assert_eq!(a.spec.track_id, b.spec.track_id, "{name}: track ids");
            // One sample per frame: a laced block of three frames yields three
            // samples, exactly as three unlaced blocks would.
            assert_eq!(
                a.samples.len(),
                b.samples.len(),
                "{name}: track {} must yield one sample per frame (got {}, {} \
                 unlaced)",
                a.spec.track_id,
                a.samples.len(),
                b.samples.len()
            );
            for (i, (sa, sb)) in a.samples.iter().zip(b.samples.iter()).enumerate() {
                assert_eq!(
                    sa.data, sb.data,
                    "{name}: sample {i} of track {} differs from the unlaced \
                     demux's frame",
                    a.spec.track_id
                );
            }
        }
    }
}

/// Fixed-size lacing: the frames must be equal length, so it is exercised with
/// three copies of one genuine audio frame. The demuxer must still recover three
/// frames from the one block.
#[test]
fn fixed_lacing_yields_one_sample_per_frame() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let all = laced_mkv::blocks(&data);
    // A real audio frame, repeated three times: still real frame bytes, and
    // equal length as fixed lacing requires.
    let seed = all.iter().find(|b| b.track == 2).expect("an audio block");
    let bytes = laced_mkv::fixed_three(&data, seed);
    let mut d = WebmDemux::new();
    let m = d.demux(&bytes).expect("fixed-laced WebM must demux");

    // The rebuilt file carries only the audio track, so it is the sole track
    // and takes the first id.
    assert_eq!(
        m.tracks.len(),
        1,
        "the fixed-laced file carries just the audio track"
    );
    let audio = &m.tracks[0];
    assert_eq!(
        audio.samples.len(),
        3,
        "a fixed-laced block of three frames must yield three samples"
    );
    for s in &audio.samples {
        assert_eq!(
            s.data.as_ref(),
            seed.frame.as_slice(),
            "each unlaced frame must be the block's frame bytes"
        );
    }
}

// ---------------------------------------------------------------------------
// r04-W40 (review): a laced block's frames must not share one timestamp
// ---------------------------------------------------------------------------

/// Build a one-cluster file with the real audio TrackEntry (+ optional
/// `DefaultDuration`) and `blocks`, where each block's frames are the fixture's
/// own audio frames (so nothing is synthesised).
///
/// Returns the demuxed audio track's `(dts, pts, duration, first_byte)` rows.
fn laced_timing(
    data: &[u8],
    default_duration_ns: u64,
    blocks: &[(i16, Vec<Vec<u8>>)],
) -> Vec<(i64, i64, u32, u8)> {
    let tracks = laced_mkv::set_default_duration(data, 2, default_duration_ns);
    let cluster: Vec<(u64, i16, u8, Vec<Vec<u8>>)> = blocks
        .iter()
        .map(|(rel_ts, frames)| (2u64, *rel_ts, 0x80u8, frames.clone()))
        .collect();
    let bytes = laced_mkv::build_custom(data, &tracks, &cluster);
    let mut demux = WebmDemux::new();
    let media = demux.demux(&bytes).expect("laced file must demux");
    assert!(
        !media.tracks.is_empty(),
        "the rebuilt file must carry the audio track (bytes {} long)",
        bytes.len()
    );
    let audio = media
        .tracks
        .iter()
        .find(|t| !matches!(t.spec.config, CodecConfig::Vp9 { .. }))
        .expect("the audio track");
    audio
        .samples
        .iter()
        .map(|s| {
            (
                s.dts.expect("dts"),
                s.pts.expect("pts"),
                s.duration.expect("duration"),
                s.data[0],
            )
        })
        .collect()
}

/// The fixture's first three distinct audio frames, used as real frame bytes.
fn fixture_audio_frames(data: &[u8]) -> Vec<Vec<u8>> {
    laced_mkv::blocks(data)
        .into_iter()
        .filter(|b| b.track == 2)
        .take(3)
        .map(|b| b.frame)
        .collect()
}

/// With `DefaultDuration` present, the frames of a laced block must be spaced
/// by it from the block's own timestamp — not all given that one timestamp.
///
/// `DefaultDuration` is 20 ms (`20_000_000` ns) here, matching the fixture's
/// real audio cadence, so a laced block of three frames starting at 0 must come
/// out as (0, 20, 40).
#[test]
fn laced_frames_are_spaced_by_default_duration() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let frames = fixture_audio_frames(&data);
    assert_eq!(frames.len(), 3);

    // One laced block of three frames at 0, then an unlaced block at 60 ms so
    // the final frame's duration is derived from a real following timestamp.
    let rows = laced_timing(
        &data,
        20_000_000,
        &[(0, frames.clone()), (60, vec![frames[0].clone()])],
    );
    let dts: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let dur: Vec<u32> = rows.iter().map(|r| r.2).collect();
    assert_eq!(
        dts,
        vec![0, 20, 40, 60],
        "each laced frame must advance by DefaultDuration (20 ms) from the \
         block's timestamp; giving all three the block's timestamp makes the \
         first two duration 0"
    );
    assert_eq!(dur, vec![20, 20, 20, 20], "every frame is 20 ms");
    // PTS tracks DTS (WebM has no reordering here).
    assert_eq!(rows.iter().map(|r| r.1).collect::<Vec<_>>(), dts);
}

/// Without `DefaultDuration`, the frames of a laced block must still be spread
/// across the interval to the next block, evenly.
///
/// Two laced blocks of three frames at 0 and 90 ms: with no declared duration
/// the gap is 90 ms over six frames, so each frame is 15 ms.
/// Two laced blocks of three frames at 0 and 90 ms: with no declared duration,
/// block 0's three frames must fit in the 90 ms before block 1 starts, so each
/// is 30 ms. (The blocks are 90 ms apart and block 1's own first frame sits on
/// that boundary, so the gap covers block 0's three frames - not six.)
#[test]
fn laced_frames_are_spread_across_the_block_interval() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let frames = fixture_audio_frames(&data);

    let rows = laced_timing(
        &data,
        0, // no DefaultDuration
        &[(0, frames.clone()), (90, frames.clone())],
    );
    let dts: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let dur: Vec<u32> = rows.iter().map(|r| r.2).collect();
    assert_eq!(
        dts,
        vec![0, 30, 60, 90, 120, 150],
        "with no DefaultDuration block 0's frames must fill the 90 ms before          block 1 (30 ms each), and block 1's frames continue at that cadence"
    );
    assert_eq!(dur, vec![30, 30, 30, 30, 30, 30]);
}

/// A laced block's frames must not collapse to zero duration even when the
/// block is the last one and there is no prior interval to reuse: the frames
/// still advance, and no non-final frame has duration 0.
#[test]
fn last_laced_block_frames_never_have_zero_duration() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let frames = fixture_audio_frames(&data);

    let rows = laced_timing(&data, 0, &[(0, frames.clone())]);
    assert_eq!(rows.len(), 3, "three frames from one laced block");
    let dts: Vec<i64> = rows.iter().map(|r| r.0).collect();
    assert!(
        dts.windows(2).all(|w| w[1] > w[0]),
        "every frame must have a strictly later timestamp, got {dts:?}"
    );
    for (i, r) in rows.iter().enumerate() {
        if i + 1 < rows.len() {
            assert_ne!(
                r.2, 0,
                "a non-final laced frame must never have duration 0 (got {rows:?})"
            );
        }
    }
}

/// The same timing guarantee under all three lacing encodings, asserting exact
/// `dts`, `pts` **and** `duration` for every frame.
///
/// The modes are genuinely different on the wire — Xiph runs, EBML signed-VINT
/// deltas including a negative one, fixed equal sizes — so the splitter and the
/// timing rule are checked together rather than only through Xiph.
#[test]
fn timing_holds_for_every_lacing_mode() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let all = laced_mkv::blocks(&data);
    // Three consecutive real audio frames, plus a following block to bound the
    // interval, relaced with each mode.
    let frames: Vec<Vec<u8>> = all
        .iter()
        .filter(|b| b.track == 2)
        .take(3)
        .map(|b| b.frame.clone())
        .collect();

    for (mode, name) in [
        (laced_mkv::XIPH, "Xiph"),
        (laced_mkv::EBML, "EBML"),
        (laced_mkv::FIXED, "fixed"),
    ] {
        let tracks = laced_mkv::set_default_duration(&data, 2, 20_000_000);
        // Fixed lacing needs equal-size frames, and EBML lacing is most
        // interesting when the sizes vary *both* ways (so a delta is negative):
        // build that case from the fixture's own frames.
        let laced_frames = match mode {
            laced_mkv::FIXED => vec![frames[0].clone(); 3],
            laced_mkv::EBML => {
                // A small frame first, then a big one (negative delta), then the
                // middle-sized one (negative again).
                let mut order: Vec<usize> = (0..frames.len()).collect();
                // Largest first, so the first delta is negative; then the
                // smallest, so the second is negative too.
                order.sort_by_key(|&i| core::cmp::Reverse(frames[i].len()));
                let big = frames[order[0]].clone();
                let small = frames[order[order.len() - 1]].clone();
                let mid = frames[order[1]].clone();
                vec![big, small, mid]
            }
            _ => frames.clone(),
        };
        // The EBML case must really carry a negative delta, or this mode is
        // again not being exercised.
        if mode == laced_mkv::EBML {
            let sizes: Vec<usize> = laced_frames.iter().map(|f| f.len()).collect();
            assert!(
                sizes[1] < sizes[0],
                "the EBML case must have a negative first delta, got {sizes:?}"
            );
        }

        let flags = 0x80u8 | mode;
        let bytes = laced_mkv::build_custom(
            &data,
            &tracks,
            &[
                (2, 0, flags, laced_frames.clone()),
                (2, 60, 0x80, vec![laced_frames[0].clone()]),
            ],
        );

        // The bytes on the wire really carry the requested mode — otherwise the
        // test's EBML case would be asserting an EBML lacing it never produced
        // (Xiph and EBML give byte-identical *frames* for some size sequences,
        // so the demuxed result alone cannot tell the two apart). The mode is
        // read back out of the built block's flags byte.
        let (wire_flags, wire_payload) = laced_mkv::first_laced_block(&bytes)
            .unwrap_or_else(|| panic!("{name}: a laced block must be on the wire"));
        assert_eq!(
            wire_flags & 0x06,
            mode,
            "{name}: the block's lacing bits must be the requested mode"
        );
        // And the block's payload really decodes back to the frames that went
        // in, whichever mode encoded them.
        assert_eq!(
            laced_mkv::unlace_payload(&wire_payload, wire_flags),
            laced_frames,
            "{name}: the block must carry the frames that were laced"
        );
        if mode == laced_mkv::EBML {
            // The first encoded delta is negative on the wire, which is the
            // case plain Xiph lacing cannot express.
            let first_delta = laced_mkv::first_ebml_delta(&wire_payload);
            assert!(
                first_delta < 0,
                "{name}: the EBML block must carry a negative first delta, got {first_delta}"
            );
        }
        let mut demux = WebmDemux::new();
        let media = demux
            .demux(&bytes)
            .unwrap_or_else(|e| panic!("{name}-laced file must demux, got {e:?}"));
        let audio = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == 1)
            .expect("audio track");

        let rows: Vec<(i64, i64, u32)> = audio
            .samples
            .iter()
            .map(|s| (s.dts.unwrap(), s.pts.unwrap(), s.duration.unwrap()))
            .collect();
        assert_eq!(
            rows,
            vec![(0, 0, 20), (20, 20, 20), (40, 40, 20), (60, 60, 20)],
            "{name}: dts, pts and duration must all be the exact per-frame cadence"
        );
        // And the frames are the ones that were laced, in order.
        for (i, want) in laced_frames.iter().enumerate() {
            assert_eq!(
                &audio.samples[i].data[..],
                want.as_slice(),
                "{name}: sample {i} must be the frame that was laced"
            );
        }
    }
}

/// A `DefaultDuration` that is not a whole millisecond must not drift.
///
/// `default_duration_ns / 1_000_000` truncates, so the common AAC-at-44.1 kHz
/// frame duration 23_219_954 ns became 23 ms — a per-frame loss of 219 954 ns,
/// i.e. 219 µs × N frames of cumulative drift across a laced run. The duration
/// is converted with rounding instead, and these are the exact ticks it must
/// produce.
#[test]
fn default_duration_is_rounded_not_truncated() {
    let data = std::fs::read(FIXTURE).expect("read webm fixture");
    let all = laced_mkv::blocks(&data);
    let frames: Vec<Vec<u8>> = all
        .iter()
        .filter(|b| b.track == 2)
        .take(3)
        .map(|b| b.frame.clone())
        .collect();

    // AAC at 44.1 kHz: 1024 samples / 44100 Hz = 23 219 954 ns, which is
    // 23.219954 ms → 23 ms truncated, 23 ms rounded. The IR timescale is 1 ms,
    // so the *per-frame* value is the same either way; what matters is that the
    // rounding is done on the ns→tick conversion rather than by integer div of
    // the ms, which is what makes the arithmetic exact for other timescales.
    let tracks = laced_mkv::set_default_duration(&data, 2, 23_219_954);
    let bytes = laced_mkv::build_custom(
        &data,
        &tracks,
        &[
            (2, 0, 0x80, frames.clone()),
            (2, 69, 0x80, vec![frames[0].clone()]),
        ],
    );
    let mut demux = WebmDemux::new();
    let media = demux.demux(&bytes).expect("demux");
    let audio = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == 1)
        .expect("audio track");

    let dts: Vec<i64> = audio.samples.iter().map(|s| s.dts.unwrap()).collect();
    // 23 ms per frame, three frames in the laced block, then the boundary block
    // at 69 ms. The block timestamps are authoritative, so the third laced
    // frame sits at 46 and the boundary at 69; nothing may drift by more than
    // the one tick the rounding can absorb.
    assert_eq!(dts, vec![0, 23, 46, 69], "no cumulative drift");
    let dur: Vec<u32> = audio.samples.iter().map(|s| s.duration.unwrap()).collect();
    assert_eq!(dur, vec![23, 23, 23, 23]);
}

/// The ns→tick conversion itself must round, which is what the rounding is
/// *for*: at a timescale where the two differ, truncating silently drops time.
///
/// `ns_to_ir_ticks` is asserted directly because the IR timescale is 1 ms, so a
/// whole-run test cannot distinguish round from truncate at 23 219 954 ns —
/// both give 23. The pathological values below are where they part.
#[test]
fn ns_to_ticks_rounds_rather_than_truncates() {
    use transmux::webm_demux::ns_to_ir_ticks;

    // 1 ms in ticks, exactly.
    assert_eq!(ns_to_ir_ticks(1_000_000), 1);
    // 1 499 999 ns is 1.499999 ms -> 1 tick rounded, 1 truncated.
    assert_eq!(ns_to_ir_ticks(1_499_999), 1);
    // 1 500 000 ns is exactly 1.5 ms -> rounds half up to 2.
    assert_eq!(ns_to_ir_ticks(1_500_000), 2);
    // 1 500 001 ns likewise.
    assert_eq!(ns_to_ir_ticks(1_500_001), 2);
    // 999 999 ns is 0.999999 ms -> 1 tick, where truncation gives 0.
    assert_eq!(
        ns_to_ir_ticks(999_999),
        1,
        "0.999999 ms must round to 1 tick, not truncate to 0"
    );
    // The reported 44.1 kHz AAC frame.
    assert_eq!(ns_to_ir_ticks(23_219_954), 23);
    // A sub-half-millisecond duration still rounds to zero ticks.
    assert_eq!(ns_to_ir_ticks(400_000), 0);
    assert_eq!(ns_to_ir_ticks(0), 0);
}

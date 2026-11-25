//! Minimal WebM/EBML file builder for tests that need a synthetic track set
//! or a hand-built block payload.
//!
//! Element ids and framing follow RFC 9559 §27 / RFC 8794 §4; the built files
//! carry only what the demuxer reads (`EBML` header, `Segment`, `Tracks`,
//! `Cluster`, `Timestamp`, `SimpleBlock`). Real-codec fixtures live in
//! `fixtures/mkv/`; this builder is for cases no real capture carries (an
//! unsupported CodecID, a hostile lace, a codec header no fixture uses).

/// `EBML` header element id.
pub const EBML_HEADER: u32 = 0x1A45_DFA3;
/// `Segment` element id.
pub const SEGMENT: u32 = 0x1853_8067;
/// `Tracks` master element id.
pub const TRACKS: u32 = 0x1654_AE6B;
/// `TrackEntry` element id.
pub const TRACK_ENTRY: u32 = 0xAE;
/// `TrackNumber` element id.
pub const TRACK_NUMBER: u32 = 0xD7;
/// `TrackType` element id.
pub const TRACK_TYPE: u32 = 0x83;
/// `CodecID` element id.
pub const CODEC_ID: u32 = 0x86;
/// `CodecPrivate` element id.
pub const CODEC_PRIVATE: u32 = 0x63A2;
/// `Cluster` master element id.
pub const CLUSTER: u32 = 0x1F43_B675;
/// `Cluster`/`Timestamp` element id.
pub const CLUSTER_TIMESTAMP: u32 = 0xE7;
/// `SimpleBlock` element id.
pub const SIMPLE_BLOCK: u32 = 0xA3;

/// Encode an EBML element id with its canonical width (no leading zero bytes).
pub fn encode_id(id: u32) -> Vec<u8> {
    let bytes = id.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(3);
    bytes[first..].to_vec()
}

/// Encode an EBML size VINT with the shortest width that fits `value`.
pub fn encode_size(mut value: usize) -> Vec<u8> {
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

/// Encode `value` as an EBML size VINT of exactly `width` bytes.
pub fn encode_size_width(value: u64, width: usize) -> Vec<u8> {
    let marker: u8 = 1 << (8 - width);
    let mut out = vec![0u8; width];
    let mut v = value;
    for i in (0..width).rev() {
        out[i] = (v & 0xFF) as u8;
        v >>= 8;
    }
    out[0] |= marker;
    out
}

/// A complete element: id + size + body.
pub fn element(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = encode_id(id);
    out.extend(encode_size(body.len()));
    out.extend_from_slice(body);
    out
}

/// A minimal big-endian `uint` body (RFC 8794 §4.1): no leading zero bytes,
/// at least one byte.
pub fn uint_body(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    bytes[first..].to_vec()
}

/// One `TrackEntry` naming its track number, type and codec.
pub fn track_entry(number: u64, track_type: u64, codec_id: &[u8], codec_private: &[u8]) -> Vec<u8> {
    let mut body = element(TRACK_NUMBER, &uint_body(number));
    body.extend(element(TRACK_TYPE, &uint_body(track_type)));
    body.extend(element(CODEC_ID, codec_id));
    body.extend(element(CODEC_PRIVATE, codec_private));
    element(TRACK_ENTRY, &body)
}

/// A one-cluster WebM: `EBML` header + `Segment { Tracks, Cluster {
/// Timestamp, SimpleBlock* } }`, with one sync block per `(track, frame)`.
pub fn file(tracks: &[Vec<u8>], blocks: &[(u64, &[u8])]) -> Vec<u8> {
    let mut tracks_body = Vec::new();
    for entry in tracks {
        tracks_body.extend_from_slice(entry);
    }
    let mut cluster_body = element(CLUSTER_TIMESTAMP, &[0u8]);
    for (track, frame) in blocks {
        let mut payload = encode_size(*track as usize);
        payload.extend_from_slice(&0i16.to_be_bytes());
        payload.push(0x80);
        payload.extend_from_slice(frame);
        cluster_body.extend(element(SIMPLE_BLOCK, &payload));
    }
    let mut segment_body = element(TRACKS, &tracks_body);
    segment_body.extend(element(CLUSTER, &cluster_body));
    let mut out = element(EBML_HEADER, &[]);
    out.extend(element(SEGMENT, &segment_body));
    out
}

/// Read an EBML element id at `p` (the width is the leading-zero count + 1).
pub fn read_id(buf: &[u8], p: usize) -> (u32, usize) {
    let first = buf[p];
    let len = first.leading_zeros() as usize + 1;
    let mut id = 0u32;
    for &b in &buf[p..p + len] {
        id = (id << 8) | b as u32;
    }
    (id, len)
}

/// Read an EBML size VINT's value and width at `p`.
pub fn read_size(buf: &[u8], p: usize) -> (u64, usize) {
    let first = buf[p];
    let len = first.leading_zeros() as usize + 1;
    let mask: u8 = if len < 8 { 0xFF >> len } else { 0 };
    let mut value = (first & mask) as u64;
    for &b in &buf[p + 1..p + len] {
        value = (value << 8) | b as u64;
    }
    (value, len)
}

pub mod synth_mkv {
    pub const EBML_HEADER: u32 = 0x1A45_DFA3;
    pub const SEGMENT: u32 = 0x1853_8067;
    pub const TRACKS: u32 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u32 = 0xAE;
    pub const TRACK_NUMBER: u32 = 0xD7;
    pub const TRACK_TYPE: u32 = 0x83;
    pub const CODEC_ID: u32 = 0x86;
    pub const CODEC_PRIVATE: u32 = 0x63A2;
    pub const VIDEO: u32 = 0xE0;
    pub const AUDIO: u32 = 0xE1;
    pub const PIXEL_WIDTH: u32 = 0xB0;
    pub const PIXEL_HEIGHT: u32 = 0xBA;
    pub const CHANNELS: u32 = 0x9F;
    pub const SAMPLING_FREQUENCY: u32 = 0xB5;
    pub const DEFAULT_DURATION: u32 = 0x23E383;
    pub const CLUSTER: u32 = 0x1F43B675;
    pub const CLUSTER_TIMESTAMP: u32 = 0xE7;
    pub const SIMPLE_BLOCK: u32 = 0xA3;
    pub const BLOCK_GROUP: u32 = 0xA0;
    pub const BLOCK: u32 = 0xA1;
    pub const REFERENCE_BLOCK: u32 = 0xFB;
    pub const BLOCK_DURATION: u32 = 0x9B;
    pub const LACING_FIXED: u8 = 0x04;
    pub const KEYFRAME: u8 = 0x80;

    pub fn encode_size(mut value: usize) -> Vec<u8> {
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

    pub fn element(id: u32, body: &[u8]) -> Vec<u8> {
        let bytes = id.to_be_bytes();
        let first = bytes.iter().position(|&b| b != 0).unwrap_or(3);
        let mut out = bytes[first..].to_vec();
        out.extend(encode_size(body.len()));
        out.extend_from_slice(body);
        out
    }

    pub fn uint(id: u32, value: u64) -> Vec<u8> {
        let be = value.to_be_bytes();
        let first = be.iter().position(|&b| b != 0).unwrap_or(7);
        element(id, &be[first..])
    }

    pub fn float(id: u32, value: f32) -> Vec<u8> {
        element(id, &value.to_bits().to_be_bytes())
    }

    pub fn block_payload(track: u64, rel_ts: i16, flags: u8, body: &[u8]) -> Vec<u8> {
        let mut out = encode_size(track as usize);
        out.extend_from_slice(&rel_ts.to_be_bytes());
        out.push(flags);
        out.extend_from_slice(body);
        out
    }

    pub fn simple_block(track: u64, rel_ts: i16, flags: u8, body: &[u8]) -> Vec<u8> {
        element(SIMPLE_BLOCK, &block_payload(track, rel_ts, flags, body))
    }

    pub fn block(track: u64, rel_ts: i16, flags: u8, body: &[u8]) -> Vec<u8> {
        element(BLOCK, &block_payload(track, rel_ts, flags, body))
    }

    pub fn track_entry(
        number: u64,
        track_type: u64,
        codec_id: &[u8],
        codec_private: &[u8],
        extra: &[u8],
    ) -> Vec<u8> {
        let mut body = uint(TRACK_NUMBER, number);
        body.extend(uint(TRACK_TYPE, track_type));
        body.extend(element(CODEC_ID, codec_id));
        if !codec_private.is_empty() {
            body.extend(element(CODEC_PRIVATE, codec_private));
        }
        body.extend_from_slice(extra);
        element(TRACK_ENTRY, &body)
    }

    pub fn tracks(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        for e in entries {
            body.extend_from_slice(e);
        }
        element(TRACKS, &body)
    }

    /// EBML header + one Segment holding `tracks` and `cluster` (already a
    /// complete Cluster element, see [`cluster`]).
    pub fn file(tracks_elem: &[u8], cluster: &[u8]) -> Vec<u8> {
        let mut segment = tracks_elem.to_vec();
        segment.extend_from_slice(cluster);
        let mut out = element(EBML_HEADER, &[]);
        out.extend(element(SEGMENT, &segment));
        out
    }

    pub fn vp8_video_track() -> Vec<u8> {
        let mut video = uint(PIXEL_WIDTH, 320);
        video.extend(uint(PIXEL_HEIGHT, 240));
        tracks(&[track_entry(1, 1, b"V_VP8", &[], &element(VIDEO, &video))])
    }

    pub fn opus_head() -> Vec<u8> {
        let mut cp = Vec::from(*b"OpusHead");
        cp.push(1);
        cp.push(2);
        cp.extend_from_slice(&[0, 0]);
        cp.extend_from_slice(&48_000u32.to_le_bytes());
        cp.extend_from_slice(&[0, 0]);
        cp.push(0);
        assert_eq!(cp.len(), 19);
        cp
    }

    pub fn opus_audio_track() -> Vec<u8> {
        tracks(&[track_entry(1, 2, b"A_OPUS", &opus_head(), &[])])
    }

    pub fn vorbis_audio_track(id_header_len: usize) -> Vec<u8> {
        let mut id = vec![0x01u8];
        id.extend_from_slice(b"vorbis");
        id.extend_from_slice(&[0, 0, 0, 0]);
        id.push(2);
        id.extend_from_slice(&44_100u32.to_le_bytes());
        assert_eq!(id.len(), 16);
        let id = &id[..id_header_len];
        let mut cp = vec![2u8, id_header_len as u8, 0];
        cp.extend_from_slice(id);
        tracks(&[track_entry(1, 2, b"A_VORBIS", &cp, &[])])
    }

    pub fn cluster(timestamp: u64, children: &[Vec<u8>]) -> Vec<u8> {
        let mut body = uint(CLUSTER_TIMESTAMP, timestamp);
        for c in children {
            body.extend_from_slice(c);
        }
        element(CLUSTER, &body)
    }
}

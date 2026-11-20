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

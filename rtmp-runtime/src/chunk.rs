//! RTMP chunk stream — basic header, message header, extended timestamp
//! (Adobe RTMP 1.0 §5.3).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §3 (RTMP Chunk Stream) for the wire
//! layout: chunk format (§5.3.1), basic header (§5.3.1.1), the four message
//! header `fmt` variants (§5.3.1.2), and extended timestamp (§5.3.1.3).
//!
//! Not yet implemented — see #738 Task N (chunk stream).

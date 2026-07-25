//! RTMP ingest server session state machine — `connect` → `createStream` →
//! `publish` (Adobe RTMP 1.0 §7.2, `NetConnection`/`NetStream` commands).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §7 (RTMP Command Messages) for the
//! command sequence this session drives.
//!
//! Not yet implemented — see #738 Task N (server).

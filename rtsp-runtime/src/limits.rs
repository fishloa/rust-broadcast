//! Size caps shared by the sans-IO cores and the `tokio` codecs.

/// Longest header block (start line + headers, up to and including the blank
/// line) accepted, enforced by `framing::head_end` on the bytes buffered before
/// the terminator. A longer head, terminated or not, is rejected.
pub(crate) const MAX_HEAD_BYTES: usize = 64 * 1024;

/// Largest complete message (headers + body) the server codec will wait for.
/// 2 MiB covers a large `ANNOUNCE` SDP body with headroom.
#[cfg(feature = "tokio")]
pub(crate) const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

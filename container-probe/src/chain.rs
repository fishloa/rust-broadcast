//! The frame-length chain walker shared by the ADTS and MP3 probers
//! (audit r01-O5: it used to be written twice, byte for byte, in `adts.rs`
//! and `mp3.rs`).
//!
//! An elementary stream is recognised structurally: a header at offset `i`
//! declares a frame length, the next header must sit exactly that far on, and
//! so on. The walker finds the longest such chain anywhere in the buffer.

/// Result of [`longest_chain`]: `(chain length, truncated, anchor, first
/// frame length)`.
///
/// `truncated` is `true` when the best chain ended by running off the end of
/// the buffer (a frame body past the end, or a next header not fully visible)
/// rather than hitting an invalid header.
pub(crate) type Chain = (usize, bool, usize, usize);

/// The longest chain of frames linked by their own length fields, scanning
/// from every candidate position `>= start`.
///
/// * `frame_len(data, i)` returns the frame length of a valid header at `i`.
/// * `header_len` is the number of bytes a header needs to be judged.
/// * `strong` short-circuits the search at that many frames (bounding the
///   work to a bounded number of link steps).
pub(crate) fn longest_chain(
    data: &[u8],
    start: usize,
    header_len: usize,
    strong: usize,
    frame_len: impl Fn(&[u8], usize) -> Option<usize>,
) -> Chain {
    let n = data.len();
    let mut best = 0usize;
    let mut best_truncated = false;
    let mut best_anchor = 0usize;
    let mut best_frame_len = 0usize;
    let mut i = start;
    while i < n {
        if let Some(first_len) = frame_len(data, i) {
            let mut p = i;
            let mut run = 0usize;
            let mut truncated = false;
            loop {
                match frame_len(data, p) {
                    Some(l) => {
                        run += 1;
                        if run >= strong {
                            return (run, false, i, first_len); // strong reached; stop early
                        }
                        if p + l > n {
                            // The frame's body extends past the buffer: a
                            // genuine truncation (the stream was cut
                            // mid-frame).
                            truncated = true;
                            break;
                        }
                        p += l;
                    }
                    None => {
                        // No valid header at `p`. If a full header could not
                        // have been read here (the region ended within it, or
                        // exactly at it), the chain ran out of data rather
                        // than being ruled out — the next frame's header is
                        // simply not fully visible yet.
                        if p + header_len > n {
                            truncated = true;
                        }
                        break;
                    }
                }
            }
            if run > best || (run == best && truncated) {
                best = run;
                best_truncated = truncated;
                best_anchor = i;
                best_frame_len = first_len;
            }
        }
        i += 1;
    }
    (best, best_truncated, best_anchor, best_frame_len)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every position is a frame header of this fixed length.
    fn fixed(frame_len: usize) -> impl Fn(&[u8], usize) -> Option<usize> {
        move |_, _| Some(frame_len)
    }

    /// A header only at positions whose byte is `0xAA`.
    fn marked(data: &[u8], i: usize) -> Option<usize> {
        data.get(i).is_some_and(|&b| b == 0xAA).then_some(4)
    }

    /// `while i < n` must not visit `i == n`, where no header exists.
    #[test]
    fn an_empty_region_has_no_chain() {
        assert_eq!(longest_chain(&[], 0, 0, 8, fixed(1)), (0, false, 0, 0));
    }

    /// A frame body extending past the end is truncated and is not counted as
    /// a second frame.
    #[test]
    fn a_frame_body_past_the_end_is_truncated_once() {
        let data = [0u8; 10];
        assert_eq!(longest_chain(&data, 0, 0, 8, fixed(11)), (1, true, 9, 11));
    }

    /// A frame body ending exactly at the end is fully present: it is not a
    /// truncation, and the walk stops at that boundary.
    #[test]
    fn a_frame_body_ending_at_the_end_is_complete() {
        let data = [0u8; 10];
        let got = longest_chain(&data, 0, 0, 8, |_, i| (i == 0).then_some(10));
        assert_eq!(got, (1, false, 0, 10));
    }

    /// A next header ending exactly at the region end is visible, so the chain
    /// is not reported as truncated merely for sitting on the boundary.
    #[test]
    fn a_header_ending_at_the_region_end_is_visible() {
        let data = [0u8; 10];
        let got = longest_chain(&data, 0, 4, 8, |_, i| (i == 0).then_some(6));
        assert_eq!(got, (1, false, 0, 6));
    }

    /// Equal-length chains: the earlier anchor is kept when neither is
    /// truncated.
    #[test]
    fn the_first_equal_chain_keeps_its_anchor() {
        let mut data = [0u8; 24];
        for i in [0usize, 4, 12, 16] {
            data[i] = 0xAA;
        }
        assert_eq!(longest_chain(&data, 0, 4, 8, marked), (2, false, 0, 4));
    }

    /// A later equal-length chain that ran off the end replaces the earlier
    /// one: it could still grow, so its anchor is the useful answer.
    #[test]
    fn a_later_truncated_equal_chain_wins_the_tie() {
        let mut data = [0u8; 20];
        for i in [0usize, 4, 12, 16] {
            data[i] = 0xAA;
        }
        assert_eq!(longest_chain(&data, 0, 4, 8, marked), (2, true, 12, 4));
    }

    /// Reaching `strong` exactly returns immediately with the first chain.
    #[test]
    fn reaching_the_strong_threshold_returns_the_first_chain() {
        let mut data = [0u8; 12];
        for i in [0usize, 4, 8] {
            data[i] = 0xAA;
        }
        assert_eq!(longest_chain(&data, 0, 4, 3, marked), (3, false, 0, 4));
    }
}

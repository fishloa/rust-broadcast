//! #2944: the sample-byte allowance is what turns an overlapping chunk layout
//! from quadratic memory into a rejection, so the multiplier's value is pinned
//! literally rather than read back through the derived budget alone.

use isobmff::frag_offsets::{MAX_SAMPLE_BYTES_PER_INPUT_LENGTH, sample_bytes_budget};

#[test]
fn one_input_length_of_sample_bytes_is_the_allowance() {
    assert_eq!(MAX_SAMPLE_BYTES_PER_INPUT_LENGTH, 1);
    assert_eq!(sample_bytes_budget(1000), 1000);
    assert_eq!(sample_bytes_budget(0), 0);
}

//! Constant-time byte equality shared by every secret comparison in the workspace.

use subtle::ConstantTimeEq;

/// Byte-equality that does not stop at the first differing byte, via
/// [`subtle::ConstantTimeEq`]. Only the length check short-circuits: lengths
/// are not the secret being protected.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_naive_equality() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer-string"));
        assert!(constant_time_eq(b"", b""));
    }
}

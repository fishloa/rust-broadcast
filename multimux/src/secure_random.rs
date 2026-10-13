//! OS-CSPRNG helpers for protocol identifiers and credentials that must be
//! unpredictable: ICE `ufrag`/`pwd` (RFC 8445 §5.4), WHIP/WHEP resource ids
//! and `ETag`s (RFC 9725 §8), RTP SSRCs (RFC 3550 §8.1 asks for a random
//! choice). Issue #1142: these were drawn from `RandomState` SipHash output,
//! which is a keyed hash seeded once per thread and not a CSPRNG; every call
//! here reads `getrandom` directly.

/// 62-symbol alphanumeric alphabet used for tokens.
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Largest multiple of the alphabet length that fits in a byte; bytes at or
/// above it are rejected so every symbol is equally likely (no modulo bias).
const REJECT_FROM: u8 = (256 / ALPHABET.len() * ALPHABET.len()) as u8;

fn fill(buf: &mut [u8]) {
    // The OS random source being unavailable is unrecoverable for a secure
    // service: refuse to mint a guessable identifier.
    getrandom::getrandom(buf).expect("OS random source unavailable");
}

/// A uniformly random alphanumeric token of `len` characters.
pub(crate) fn token(len: usize) -> String {
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        fill(&mut buf);
        for &b in &buf {
            if b < REJECT_FROM && out.len() < len {
                out.push(ALPHABET[usize::from(b) % ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// A random non-zero 32-bit SSRC (the low bit is forced so the value is
/// never 0).
pub(crate) fn ssrc() -> u32 {
    let mut b = [0u8; 4];
    fill(&mut b);
    u32::from_be_bytes(b) | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_has_requested_length_and_alphabet() {
        for len in [0, 1, 8, 24, 32, 200] {
            let t = token(len);
            assert_eq!(t.len(), len);
            assert!(t.bytes().all(|b| ALPHABET.contains(&b)));
        }
    }

    #[test]
    fn tokens_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            assert!(seen.insert(token(24)), "a 24-char token repeated");
        }
    }

    #[test]
    fn ssrc_is_nonzero_and_varies() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let s = ssrc();
            assert_ne!(s, 0);
            seen.insert(s);
        }
        assert!(seen.len() > 190);
    }
}

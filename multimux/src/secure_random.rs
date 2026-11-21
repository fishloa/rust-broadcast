//! OS-CSPRNG helpers for protocol identifiers and credentials that must be
//! unpredictable: ICE `ufrag`/`pwd` (RFC 8445 §5.4), WHIP/WHEP resource ids
//! and `ETag`s (RFC 9725 §8), RTP SSRCs (RFC 3550 §8.1 asks for a random
//! choice). Issue #1142: these were drawn from `RandomState` SipHash output,
//! which is a keyed hash seeded once per thread and not a CSPRNG; every call
//! here reads `getrandom` directly.

#[cfg(any(feature = "whip", feature = "whep"))]
/// 62-symbol alphanumeric alphabet used for tokens.
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

#[cfg(any(feature = "whip", feature = "whep"))]
/// Largest multiple of the alphabet length that fits in a byte; bytes at or
/// above it are rejected so every symbol is equally likely (no modulo bias).
const REJECT_FROM: u8 = (256 / ALPHABET.len() * ALPHABET.len()) as u8;

fn fill(buf: &mut [u8]) {
    // The OS random source being unavailable is unrecoverable for a secure
    // service: refuse to mint a guessable identifier.
    getrandom::fill(buf).expect("OS random source unavailable");
}

/// 64 bits of OS randomness, for a seed that only has to be unpredictable
/// enough to keep two peers from pacing a retry in lockstep (the backoff
/// jitter). Unlike [`fill`]'s callers this is not a security boundary, so an
/// unavailable source degrades to `0` (which the jitter's own mixing turns
/// into a non-degenerate sequence) rather than taking the process down.
pub(crate) fn seed_entropy() -> u64 {
    let mut buf = [0u8; 8];
    getrandom::fill(&mut buf).map_or(0, |()| u64::from_le_bytes(buf))
}

/// 32 bytes of OS randomness: the secret an SRT caller session derives its identifiers and
/// stream key from.
pub(crate) fn entropy_seed() -> [u8; broadcast_client::srt::ENTROPY_SEED_LEN] {
    let mut buf = [0u8; broadcast_client::srt::ENTROPY_SEED_LEN];
    fill(&mut buf);
    buf
}

#[cfg(any(feature = "whip", feature = "whep"))]
/// The alphabet symbol for a random byte, or `None` when the byte is at or
/// above [`REJECT_FROM`] and would make some symbols more likely than others.
fn token_symbol(b: u8) -> Option<char> {
    (b < REJECT_FROM).then(|| ALPHABET[usize::from(b) % ALPHABET.len()] as char)
}

#[cfg(any(feature = "whip", feature = "whep"))]
/// A uniformly random alphanumeric token of `len` characters.
pub(crate) fn token(len: usize) -> String {
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        fill(&mut buf);
        for &b in &buf {
            if out.len() < len
                && let Some(c) = token_symbol(b)
            {
                out.push(c);
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

    /// A token whose rejection logic collapses spins forever; run the sample
    /// on its own thread so the test thread can report the failure instead of
    /// parking the whole suite.
    #[cfg(any(feature = "whip", feature = "whep"))]
    fn within_deadline<F: FnOnce() + Send + 'static>(body: F) {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
            let _ = done.send(result);
        });
        match finished.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(Ok(())) => {}
            Ok(Err(payload)) => std::panic::resume_unwind(payload),
            Err(_) => panic!("the token sample must terminate"),
        }
    }

    #[cfg(any(feature = "whip", feature = "whep"))]
    #[test]
    fn token_has_requested_length_and_alphabet() {
        within_deadline(|| {
            for len in [0, 1, 8, 24, 32, 200] {
                let t = token(len);
                assert_eq!(t.len(), len);
                assert!(t.bytes().all(|b| ALPHABET.contains(&b)));
            }
        });
    }

    #[cfg(any(feature = "whip", feature = "whep"))]
    #[test]
    fn tokens_do_not_repeat() {
        within_deadline(|| {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..2000 {
                assert!(seen.insert(token(24)), "a 24-char token repeated");
            }
        });
    }

    #[cfg(any(feature = "whip", feature = "whep"))]
    #[test]
    fn token_rejection_threshold_rejects_the_biased_byte() {
        assert_eq!(REJECT_FROM, 248, "largest multiple of 62 at or below 256");
        assert_eq!(REJECT_FROM as usize % ALPHABET.len(), 0);
        assert_eq!(token_symbol(0), Some('A'));
        assert_eq!(token_symbol(61), Some('9'), "the last alphabet symbol");
        assert_eq!(
            token_symbol(247),
            Some(ALPHABET[247 % ALPHABET.len()] as char)
        );
        assert_eq!(token_symbol(248), None, "248 would bias the alphabet");
        assert_eq!(token_symbol(255), None);
    }

    #[cfg(any(feature = "whip", feature = "whep"))]
    #[test]
    fn token_uses_every_symbol_of_the_alphabet() {
        within_deadline(|| {
            let sample = token(10_000);
            let mut counts = [0usize; 62];
            for b in sample.bytes() {
                let index = ALPHABET
                    .iter()
                    .position(|&a| a == b)
                    .expect("token byte is an alphabet symbol");
                counts[index] += 1;
            }
            let mean = sample.len() / ALPHABET.len();
            for (index, &count) in counts.iter().enumerate() {
                assert!(
                    count > 0,
                    "symbol {} never occurred",
                    ALPHABET[index] as char
                );
                assert!(
                    count <= 2 * mean,
                    "symbol {} over-represented: {count} vs mean {mean}",
                    ALPHABET[index] as char
                );
            }
        });
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
        for _ in 0..256 {
            assert_eq!(
                ssrc() & 1,
                1,
                "the low bit is forced so a random draw never yields an even SSRC"
            );
        }
    }

    #[test]
    fn entropy_seeds_are_random_and_never_a_constant() {
        let a = entropy_seed();
        let b = entropy_seed();
        assert_ne!(a, b, "two SRT sessions must not derive the same secret");
        for seed in [a, b] {
            assert_ne!(seed, [0u8; broadcast_client::srt::ENTROPY_SEED_LEN]);
            assert_ne!(seed, [1u8; broadcast_client::srt::ENTROPY_SEED_LEN]);
        }
        assert_eq!(a.len(), broadcast_client::srt::ENTROPY_SEED_LEN);
    }

    #[test]
    fn seed_entropy_varies_and_is_not_a_degenerate_constant() {
        let mut varied = false;
        for _ in 0..3 {
            let (a, b) = (seed_entropy(), seed_entropy());
            assert_ne!(a, 0);
            assert_ne!(a, 1);
            assert_ne!(b, 0);
            assert_ne!(b, 1);
            varied |= a != b;
        }
        assert!(varied, "three entropy pairs all repeated");
    }
}

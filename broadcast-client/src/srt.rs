//! SRT caller configuration ([`crate::Config::srt`]).

use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::timeouts::Timeouts;
use core::time::Duration;

use zeroize::Zeroize;

/// Length of the entropy seed in bytes.
pub const ENTROPY_SEED_LEN: usize = 32;

/// Default [`SrtCallerConfig::latency`] (120 ms, libsrt's default), which suits a LAN or a
/// well-provisioned WAN. The negotiated value is the larger of this and the peer's.
pub const DEFAULT_LATENCY: Duration = Duration::from_millis(120);
/// Default [`SrtCallerConfig::timeouts`]' handshake bound: libsrt's 3 s connect
/// timeout.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
/// Default [`SrtCallerConfig::timeouts`]' peer-idle bound: libsrt's 5 s peer
/// idle timeout.
pub const DEFAULT_PEER_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// AES key length of an encrypted SRT connection (the stream key the caller generates).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtKeyLength {
    /// AES-128, a 16-byte key.
    Aes128,
    /// AES-192, a 24-byte key.
    Aes192,
    /// AES-256, a 32-byte key.
    Aes256,
}

impl SrtKeyLength {
    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Aes128 => "aes-128",
            Self::Aes192 => "aes-192",
            Self::Aes256 => "aes-256",
        }
    }

    pub(crate) fn bytes(self) -> usize {
        match self {
            Self::Aes128 => 16,
            Self::Aes192 => 24,
            Self::Aes256 => 32,
        }
    }
}

broadcast_common::impl_spec_display!(SrtKeyLength);

/// Passphrase encryption of an SRT connection. The passphrase is a secret: it is wiped on drop and
/// never shown by `Debug`.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub struct SrtEncryption {
    passphrase: Vec<u8>,
    /// Length of the stream key the session generates and wraps with a key derived from the
    /// passphrase.
    pub key_length: SrtKeyLength,
}

impl SrtEncryption {
    /// Encrypt with `passphrase` and a key of `key_length`.
    #[must_use]
    pub fn new(passphrase: impl Into<String>, key_length: SrtKeyLength) -> Self {
        Self {
            passphrase: passphrase.into().into_bytes(),
            key_length,
        }
    }

    pub(crate) fn passphrase(&self) -> &[u8] {
        &self.passphrase
    }
}

impl Drop for SrtEncryption {
    fn drop(&mut self) {
        self.passphrase.zeroize();
    }
}

impl core::fmt::Debug for SrtEncryption {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SrtEncryption")
            .field("passphrase", &format_args!("<redacted>"))
            .field("key_length", &self.key_length)
            .finish()
    }
}

/// Secret randomness the session derives each SRT connection's identifiers and stream key from.
/// The session reads no randomness source: the host fills this from its CSPRNG, once per session.
#[derive(Clone, PartialEq, Eq)]
pub struct SrtEntropySeed([u8; ENTROPY_SEED_LEN]);

impl SrtEntropySeed {
    /// Wrap `bytes`, which must come from a cryptographically secure random generator.
    #[must_use]
    pub fn new(bytes: [u8; ENTROPY_SEED_LEN]) -> Self {
        Self(bytes)
    }

    pub(crate) fn bytes(&self) -> &[u8; ENTROPY_SEED_LEN] {
        &self.0
    }
}

impl Drop for SrtEntropySeed {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl core::fmt::Debug for SrtEntropySeed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SrtEntropySeed(<redacted>)")
    }
}

/// SRT caller settings ([`crate::Config::srt`]): what the sans-IO caller sends
/// and how long it waits, not the tokio adapter's socket settings (those are
/// `srt_runtime::io::SrtSocketConfig`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtCallerConfig {
    /// TSBPD latency this side asks for, sent as whole milliseconds on the wire (the connection
    /// uses the larger of this and the peer's); any value is accepted, a smaller one trades loss
    /// recovery for delay.
    pub latency: Duration,
    /// Stream id sent in the handshake (an access-control string such as `#!::r=name,m=request`).
    pub stream_id: Option<String>,
    /// Passphrase encryption; `None` is a clear connection.
    pub encryption: Option<SrtEncryption>,
    /// Required for an `srt://` session: see [`SrtEntropySeed`].
    pub entropy_seed: Option<SrtEntropySeed>,
    /// The bounds the caller reads its clock against: `handshake` is the
    /// longest the handshake may take before the attempt fails and `read_idle`
    /// the longest silence from the peer (not even a keepalive) before the
    /// connection is dropped. `connect` and `write` are unused (the session
    /// performs no I/O of its own). Defaults: 3 s handshake, 5 s peer idle.
    pub timeouts: Timeouts,
}

impl Default for SrtCallerConfig {
    fn default() -> Self {
        Self {
            latency: DEFAULT_LATENCY,
            stream_id: None,
            encryption: None,
            entropy_seed: None,
            timeouts: Timeouts::default()
                .with_handshake(DEFAULT_HANDSHAKE_TIMEOUT)
                .with_read_idle(DEFAULT_PEER_IDLE_TIMEOUT),
        }
    }
}

impl SrtCallerConfig {
    /// Replace the handshake and peer-idle bounds with `timeouts`' (`handshake`
    /// and `read_idle`; `connect`/`write` are not used).
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        let invalid = |field| Err(crate::Error::InvalidConfig { field });
        if self.timeouts.handshake.is_zero() {
            return invalid("srt.timeouts.handshake");
        }
        if self.timeouts.read_idle.is_zero() {
            return invalid("srt.timeouts.read_idle");
        }
        if self
            .encryption
            .as_ref()
            .is_some_and(|e| e.passphrase.is_empty())
        {
            return invalid("srt.encryption.passphrase");
        }
        Ok(())
    }
}

#[cfg(test)]
mod timeouts_tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn the_default_bounds_are_three_seconds_of_handshake_and_five_of_peer_idle() {
        let config = SrtCallerConfig::default();
        assert_eq!(config.timeouts.handshake, Duration::from_secs(3));
        assert_eq!(config.timeouts.read_idle, Duration::from_secs(5));
        assert_eq!(config.timeouts.handshake, DEFAULT_HANDSHAKE_TIMEOUT);
        assert_eq!(config.timeouts.read_idle, DEFAULT_PEER_IDLE_TIMEOUT);
    }

    #[test]
    fn with_timeouts_stores_the_one_timeouts_value_it_is_given() {
        let timeouts = Timeouts::default()
            .with_handshake(Duration::from_millis(1500))
            .with_read_idle(Duration::from_millis(7000));
        let config = SrtCallerConfig::default().with_timeouts(timeouts);
        assert_eq!(config.timeouts, timeouts);
        assert_eq!(config.timeouts.handshake, Duration::from_millis(1500));
        assert_eq!(config.timeouts.read_idle, Duration::from_millis(7000));
    }

    #[test]
    fn a_zero_bound_is_rejected_rather_than_widened() {
        let config = SrtCallerConfig::default()
            .with_timeouts(Timeouts::default().with_handshake(Duration::ZERO));
        assert!(matches!(
            config.validate(),
            Err(crate::Error::InvalidConfig {
                field: "srt.timeouts.handshake"
            })
        ));
        let config = SrtCallerConfig::default()
            .with_timeouts(Timeouts::default().with_read_idle(Duration::ZERO));
        assert!(matches!(
            config.validate(),
            Err(crate::Error::InvalidConfig {
                field: "srt.timeouts.read_idle"
            })
        ));
    }
}

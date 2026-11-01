//! SRT caller configuration ([`crate::Config::srt`]).

use alloc::string::String;
use alloc::vec::Vec;

use zeroize::Zeroize;

/// Length of the entropy seed in bytes.
pub const ENTROPY_SEED_LEN: usize = 32;

/// Default [`SrtConfig::latency_ms`]: the SRT receiver latency floor (120 ms), which suits a LAN or
/// a well-provisioned WAN. The negotiated value is the larger of this and the peer's.
pub const DEFAULT_LATENCY_MS: u16 = 120;
/// Default [`SrtConfig::handshake_timeout_ms`]: libsrt's 3 s connect timeout.
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 3_000;
/// Default [`SrtConfig::peer_idle_timeout_ms`]: libsrt's 5 s peer idle timeout.
pub const DEFAULT_PEER_IDLE_TIMEOUT_MS: u64 = 5_000;

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

/// SRT caller settings.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtConfig {
    /// TSBPD latency this side asks for in milliseconds (the connection uses the larger of this
    /// and the peer's). At least 120: the receiver's floor.
    pub latency_ms: u16,
    /// Stream id sent in the handshake (an access-control string such as `#!::r=name,m=request`).
    pub stream_id: Option<String>,
    /// Passphrase encryption; `None` is a clear connection.
    pub encryption: Option<SrtEncryption>,
    /// Required for an `srt://` session: see [`SrtEntropySeed`].
    pub entropy_seed: Option<SrtEntropySeed>,
    /// Longest the handshake may take before the attempt fails.
    pub handshake_timeout_ms: u64,
    /// Longest silence from the peer (not even a keepalive) before the connection is dropped.
    pub peer_idle_timeout_ms: u64,
}

impl Default for SrtConfig {
    fn default() -> Self {
        Self {
            latency_ms: DEFAULT_LATENCY_MS,
            stream_id: None,
            encryption: None,
            entropy_seed: None,
            handshake_timeout_ms: DEFAULT_HANDSHAKE_TIMEOUT_MS,
            peer_idle_timeout_ms: DEFAULT_PEER_IDLE_TIMEOUT_MS,
        }
    }
}

impl SrtConfig {
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        let invalid = |field| Err(crate::Error::InvalidConfig { field });
        if self.latency_ms < DEFAULT_LATENCY_MS {
            return invalid("srt.latency_ms");
        }
        if self.handshake_timeout_ms == 0 {
            return invalid("srt.handshake_timeout_ms");
        }
        if self.peer_idle_timeout_ms == 0 {
            return invalid("srt.peer_idle_timeout_ms");
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

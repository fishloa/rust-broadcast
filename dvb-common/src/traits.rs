//! Canonical `Parse` and `Serialize` traits for the DVB crate family.
//!
//! Each implementer picks its own error type via `type Error`, so
//! domain-specific error variants stay visible to the caller.

/// Parse a DVB structure from raw bytes. Borrowing allowed via `<'a>`; the
/// concrete error type is chosen per implementer.
pub trait Parse<'a>: Sized {
    /// The error type this implementer returns. Typically the enclosing
    /// crate's `Error` enum.
    type Error;

    /// Parse `bytes` as `Self`. Returns `Err(Self::Error)` on any protocol
    /// violation or buffer underrun.
    fn parse(bytes: &'a [u8]) -> Result<Self, Self::Error>;
}

/// Serialize a DVB structure back to bytes. Split from [`Parse`] so owned
/// and borrowed variants of the same type can implement `Serialize`
/// without carrying a lifetime.
pub trait Serialize {
    /// The error type this implementer returns (usually the same as the
    /// corresponding [`Parse`] impl, but need not be).
    type Error;

    /// Number of bytes `serialize_into` will write.
    fn serialized_len(&self) -> usize;

    /// Write the serialised form into `buf`. Returns the number of bytes
    /// written (always equal to `serialized_len()`).
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Convenience: allocate a `Vec` and serialise into it. Panics only if
    /// `serialize_into` misreports `serialized_len()` — a contract every
    /// implementer is responsible for upholding.
    fn to_bytes(&self) -> Vec<u8>
    where
        Self::Error: core::fmt::Debug,
    {
        let mut v = vec![0u8; self.serialized_len()];
        self.serialize_into(&mut v)
            .expect("serialize_into must succeed when buffer is exactly serialized_len()");
        v
    }
}

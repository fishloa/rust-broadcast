//! T2-MI–specific traits. `Parse` / `Serialize` are provided by `dvb_common`
//! and imported directly at call sites.

/// Sealed trait — no external implementors. Keeps `PayloadDef` extensible
/// without breaking downstream implementors.
pub(crate) mod sealed {
    pub trait Sealed {}
}

/// Implemented by every typed T2-MI payload; drives
/// [`crate::payload::AnyPayload`] dispatch.
///
/// `PACKET_TYPE` is the wire `packet_type` byte from the T2-MI header that
/// this type parses. `NAME` is a diagnostic label in SCREAMING_SNAKE convention
/// without any `_payload` suffix.
pub trait PayloadDef<'a>:
    sealed::Sealed + dvb_common::Parse<'a, Error = crate::error::Error>
{
    /// Wire `packet_type` byte (TS 102 773 Table 1) this type accepts.
    const PACKET_TYPE: u8;
    /// Diagnostic name. Convention (workspace-wide): SCREAMING_SNAKE,
    /// suffix-free — `BBFRAME`, `L1_CURRENT`, `FEF_NULL`
    /// (no `_payload` suffix).
    const NAME: &'static str;
}

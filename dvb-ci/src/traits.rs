//! `dvb-ci`-specific dispatch trait. `Parse` / `Serialize` come from
//! `broadcast_common` and are imported at the call sites.
//!
//! Mirrors dvb-si's `DescriptorDef` and scte35-splice's `CommandDef`: each typed
//! APDU object declares its 3-byte `apdu_tag` and a SCREAMING_SNAKE diagnostic
//! `NAME`, and the `declare_apdus!` dispatch macro pins the tag in the dispatch
//! list to the trait const via a drift test, so the list can never silently
//! drift from the implemented set.
//!
//! The `Parse` impl on each object parses the **whole** APDU including its
//! `apdu_tag` + `length_field` header, and the `Serialize` impl writes the whole
//! APDU back — so dispatch can route on the header and round-trip is symmetric.

use crate::tag::ApduTag;
use broadcast_common::Parse;

/// Implemented by every typed APDU object; drives [`crate::AnyApdu`] dispatch.
pub trait ApduDef<'a>: Parse<'a, Error = crate::error::Error> {
    /// The object's 3-byte `apdu_tag` (EN 50221 Table 58).
    const TAG: ApduTag;
    /// Diagnostic name, SCREAMING_SNAKE, suffix-free: `CA_PMT`, `PROFILE_ENQ`,
    /// `APPLICATION_INFO`, `DATE_TIME`.
    const NAME: &'static str;
}

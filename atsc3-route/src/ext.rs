//! ROUTE-specific LCT header extension **constants** — HET values for A/331
//! Annex A §A.3.7 (`EXT_ROUTE_PRESENTATION_TIME`) and §A.3.8 (`EXT_TOL`),
//! transcribed at `atsc3/docs/a331-route.md` §2.
//!
//! The typed decoders (`ExtRoutePresentationTime`, `ExtTol`) that previously
//! lived here were **removed** because no publicly-available ATSC 3.0 ROUTE
//! capture contains either extension (14,000+ real packets from three
//! independent sources scanned, zero hits). This crate's fixture discipline
//! requires every implemented type to be exercised by a byte-exact round-trip
//! against a real capture — implementing from spec alone, with only inline
//! unit tests, is below the bar.
//!
//! The HET constants remain so a caller walking an [`rmt_flute::LctHeader`]'s
//! extension chain can match on them and extract the raw bytes.  When a real
//! capture containing these extensions surfaces, the typed decoders can be
//! re-added and fixture-tested in the same commit.

// ---------------------------------------------------------------------------
// EXT_ROUTE_PRESENTATION_TIME — §A.3.7.1, Figure A.3.5 (HET = 66)
// ---------------------------------------------------------------------------

/// HET for `EXT_ROUTE_PRESENTATION_TIME` (§A.3.7.1). Variable-length
/// extension (HET < 128, carries `HEL`).
pub const HET_EXT_ROUTE_PRESENTATION_TIME: u8 = 66;

// ---------------------------------------------------------------------------
// EXT_TOL — Transport Object Length (§A.3.8.1, Figures A.3.6/A.3.7)
// ---------------------------------------------------------------------------

/// HET for the 24-bit form of `EXT_TOL` (§A.3.8.1, Figure A.3.6). Fixed-length
/// extension (one 32-bit word, no `HEL`).
pub const HET_EXT_TOL_24: u8 = 194;
/// HET for the 48-bit form of `EXT_TOL` (§A.3.8.1, Figure A.3.7).
/// Variable-length extension (`HEL` = 2, two 32-bit words).
pub const HET_EXT_TOL_48: u8 = 67;

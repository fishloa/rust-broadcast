//! Pure time arithmetic: UTC clock, `xs:dateTime`, UTCTiming, availability,
//! live edge and PeriodStart. Nothing in this module reads a clock; every
//! function takes the instants it needs as arguments.

pub mod datetime;

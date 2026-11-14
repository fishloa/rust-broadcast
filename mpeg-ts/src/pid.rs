//! MPEG-TS Packet Identifier (PID) types and well-known PID constants.
//!
//! Specified by ITU-T H.222.0 §2.4 (= ISO/IEC 13818-1). DVB-specific
//! well-known PID assignments are also listed in ETSI EN 300 468 §5.1.3 Table 1.

/// A 13-bit MPEG-TS Packet Identifier.
///
/// Thin newtype over the wire `u16`, always in `0x0000..=0x1FFF` (the
/// transport header field width, ISO/IEC 13818-1 §2.4.3.2). Construction never
/// masks a caller-supplied value: [`Pid::try_new`] rejects an out-of-range
/// one, [`Pid::from_wire_masked`] is the explicit parse-site constructor, and
/// [`Pid::from_const`] is the compile-time constructor for constants.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Pid(u16);

impl Pid {
    /// Mask covering the 13 valid PID bits.
    pub const MASK: u16 = 0x1FFF;

    /// Construct a constant `Pid`, **failing compilation** (a `const` panic)
    /// when `value` exceeds [`Pid::MASK`].
    ///
    /// This is the constructor for well-known PIDs and table constants: a
    /// mistyped literal is a build error, never a silently masked different
    /// PID. Runtime values use [`Pid::try_new`].
    ///
    /// ```
    /// use mpeg_ts::pid::Pid;
    /// const PMT: Pid = Pid::from_const(0x0100);
    /// assert_eq!(PMT.value(), 0x0100);
    /// ```
    #[must_use]
    pub const fn from_const(value: u16) -> Self {
        assert!(
            value <= Self::MASK,
            "PID constant exceeds the 13-bit range 0x0000..=0x1FFF"
        );
        Self(value)
    }

    /// Construct a `Pid` from the low 13 bits of `value`, **deliberately**
    /// discarding the rest: the explicit name for extracting the 13-bit PID
    /// field out of a wider wire word (e.g. `reserved(3)` + `PID(13)`), where
    /// masking is the specified behaviour. Use this only at parse sites, so a
    /// caller-supplied value is never masked by accident; use
    /// [`Pid::try_new`] for those and [`Pid::from_const`] for constants.
    ///
    /// ```
    /// use mpeg_ts::pid::Pid;
    /// // reserved(3) = 0b111 followed by PID 0x0100.
    /// assert_eq!(Pid::from_wire_masked(0xE100).value(), 0x0100);
    /// assert_eq!(Pid::try_new(0xE100).unwrap_err().value, 0xE100);
    /// ```
    #[must_use]
    pub const fn from_wire_masked(value: u16) -> Self {
        Self(value & Self::MASK)
    }

    /// Construct a `Pid`, rejecting any value above [`Pid::MASK`] (`0x1FFF`).
    ///
    /// Use this at API boundaries that take a caller-supplied value.
    ///
    /// # Errors
    /// [`PidOutOfRange`] when `value > 0x1FFF`.
    pub const fn try_new(value: u16) -> Result<Self, PidOutOfRange> {
        if value > Self::MASK {
            Err(PidOutOfRange { value })
        } else {
            Ok(Self(value))
        }
    }

    /// The raw 13-bit value.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

/// A value that does not fit the 13-bit PID field (ISO/IEC 13818-1 §2.4.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PidOutOfRange {
    /// The rejected value.
    pub value: u16,
}

impl core::fmt::Display for PidOutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "PID 0x{:04X} exceeds the 13-bit range 0x0000..=0x1FFF",
            self.value
        )
    }
}

impl core::error::Error for PidOutOfRange {}

impl TryFrom<u16> for Pid {
    type Error = PidOutOfRange;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<Pid> for u16 {
    fn from(pid: Pid) -> Self {
        pid.0
    }
}

impl core::fmt::Debug for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Pid(0x{:04X})", self.0)
    }
}

impl core::fmt::UpperHex for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}

impl core::fmt::LowerHex for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

impl core::fmt::Display for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:04X}", self.0)
    }
}

/// Well-known PIDs. The transport stream MUST carry the corresponding tables
/// on these PIDs.
///
/// Each constant is a [`crate::pid::Pid`]; call `.value()` to obtain the raw
/// `u16`, or use `Pid::try_from(u16)` / `u16::from(Pid)` for conversions.
pub mod well_known {
    use super::Pid;

    /// Program Association Table (MPEG-2).
    pub const PAT: Pid = Pid::from_const(0x0000);
    /// Conditional Access Table (MPEG-2).
    pub const CAT: Pid = Pid::from_const(0x0001);
    /// Transport Stream Description Table (MPEG-2).
    pub const TSDT: Pid = Pid::from_const(0x0002);
    /// IPMP Control Information Table (MPEG-2).
    pub const IPMP_CIT: Pid = Pid::from_const(0x0003);
    /// Network Information Table (DVB).
    pub const NIT: Pid = Pid::from_const(0x0010);
    /// Service Description Table + Bouquet Association Table (DVB).
    pub const SDT_BAT: Pid = Pid::from_const(0x0011);
    /// Event Information Table (DVB).
    pub const EIT: Pid = Pid::from_const(0x0012);
    /// Running Status Table (DVB).
    pub const RST: Pid = Pid::from_const(0x0013);
    /// Time and Date + Time Offset + Stuffing (DVB).
    pub const TDT_TOT: Pid = Pid::from_const(0x0014);
    /// Network synchronisation.
    pub const NETWORK_SYNC: Pid = Pid::from_const(0x0015);
    /// Resolution Notification Table.
    pub const RNT: Pid = Pid::from_const(0x0016);
    /// Satellite Access Table (EN 300 468 Table 1).
    pub const SAT: Pid = Pid::from_const(0x001B);
    /// Link-local inband signalling (EN 300 468 Table 1); only for use between
    /// devices in a controlled environment, never on a broadcast signal.
    pub const INBAND_SIGNALLING: Pid = Pid::from_const(0x001C);
    /// Measurement (reserved).
    pub const MEASUREMENT: Pid = Pid::from_const(0x001D);
    /// Discontinuity Information Table.
    pub const DIT: Pid = Pid::from_const(0x001E);
    /// Selection Information Table.
    pub const SIT: Pid = Pid::from_const(0x001F);

    /// ATSC PSIP base PID.
    pub const ATSC_PSIP: Pid = Pid::from_const(0x1FFB);

    /// Null-packet padding PID. Payload is ignored.
    pub const NULL: Pid = Pid::from_const(0x1FFF);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_const_is_the_compile_time_constructor() {
        const PMT: Pid = Pid::from_const(0x0100);
        assert_eq!(PMT.value(), 0x0100);
        assert_eq!(Pid::from_const(0).value(), 0);
        assert_eq!(Pid::from_const(Pid::MASK).value(), 0x1FFF);
    }

    #[test]
    fn try_new_and_from_wire_masked_differ_only_in_policy() {
        assert_eq!(Pid::try_new(0x0000), Ok(well_known::PAT));
        assert_eq!(Pid::try_new(Pid::MASK), Ok(well_known::NULL));
        assert!(Pid::try_new(0x2000).is_err());
        assert_eq!(Pid::from_wire_masked(0xE100).value(), 0x0100);
    }

    #[test]
    fn inband_signalling_sits_between_sat_and_measurement() {
        assert_eq!(well_known::INBAND_SIGNALLING.value(), 0x001C);
        assert_eq!(
            well_known::INBAND_SIGNALLING.value(),
            well_known::SAT.value() + 1
        );
        assert_eq!(
            well_known::MEASUREMENT.value(),
            well_known::INBAND_SIGNALLING.value() + 1
        );
    }
}

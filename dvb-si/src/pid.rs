//! Reserved DVB/MPEG-2 PIDs.
//!
//! Values are fixed by ETSI EN 300 468 §5.1.3 Table 1 and ISO/IEC 13818-1.

/// A 13-bit MPEG-TS Packet Identifier.
///
/// Thin newtype over the wire `u16`. Values are masked to 13 bits on
/// construction (`0x0000..=0x1FFF`), matching the transport header field
/// width (ISO/IEC 13818-1 §2.4.3.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Pid(u16);

impl Pid {
    /// Mask covering the 13 valid PID bits.
    pub const MASK: u16 = 0x1FFF;

    /// Construct a `Pid`, masking to the low 13 bits.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value & Self::MASK)
    }

    /// The raw 13-bit value.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl From<u16> for Pid {
    fn from(value: u16) -> Self {
        Self::new(value)
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

impl core::fmt::Display for Pid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:04X}", self.0)
    }
}

/// Well-known PIDs. The transport stream MUST carry the corresponding tables
/// on these PIDs.
///
/// **API 2.0 type change:** constants are now [`crate::pid::Pid`] instead of `u16`.
/// Call `.value()` to obtain the raw `u16`, or use `Pid::from(u16)` /
/// `u16::from(Pid)` for conversions.
pub mod well_known {
    use super::Pid;

    /// Program Association Table (MPEG-2).
    pub const PAT: Pid = Pid::new(0x0000);
    /// Conditional Access Table (MPEG-2).
    pub const CAT: Pid = Pid::new(0x0001);
    /// Transport Stream Description Table (MPEG-2).
    pub const TSDT: Pid = Pid::new(0x0002);
    /// IPMP Control Information Table (MPEG-2).
    pub const IPMP_CIT: Pid = Pid::new(0x0003);
    /// Network Information Table (DVB).
    pub const NIT: Pid = Pid::new(0x0010);
    /// Service Description Table + Bouquet Association Table (DVB).
    pub const SDT_BAT: Pid = Pid::new(0x0011);
    /// Event Information Table (DVB).
    pub const EIT: Pid = Pid::new(0x0012);
    /// Running Status Table (DVB).
    pub const RST: Pid = Pid::new(0x0013);
    /// Time and Date + Time Offset + Stuffing (DVB).
    pub const TDT_TOT: Pid = Pid::new(0x0014);
    /// Network synchronisation.
    pub const NETWORK_SYNC: Pid = Pid::new(0x0015);
    /// Resolution Notification Table.
    pub const RNT: Pid = Pid::new(0x0016);
    /// Satellite Access Table (EN 300 468 Table 1).
    pub const SAT: Pid = Pid::new(0x001B);
    /// Link-local inband signalling (reserved).
    pub const INBAND_SIGNALLING: Pid = Pid::new(0x001C);
    /// Measurement (reserved).
    pub const MEASUREMENT: Pid = Pid::new(0x001D);
    /// Discontinuity Information Table.
    pub const DIT: Pid = Pid::new(0x001E);
    /// Selection Information Table.
    pub const SIT: Pid = Pid::new(0x001F);

    /// ATSC PSIP base PID.
    pub const ATSC_PSIP: Pid = Pid::new(0x1FFB);

    /// Null-packet padding PID. Payload is ignored.
    pub const NULL: Pid = Pid::new(0x1FFF);
}

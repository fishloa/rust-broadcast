//! Reserved DVB/MPEG-2 PIDs.
//!
//! Values are fixed by ETSI EN 300 468 §5.1.3 Table 1 and ISO/IEC 13818-1.

/// Well-known PIDs. The transport stream MUST carry the corresponding tables
/// on these PIDs.
pub mod well_known {
    /// Program Association Table (MPEG-2).
    pub const PAT: u16 = 0x0000;
    /// Conditional Access Table (MPEG-2).
    pub const CAT: u16 = 0x0001;
    /// Transport Stream Description Table (MPEG-2).
    pub const TSDT: u16 = 0x0002;
    /// IPMP Control Information Table (MPEG-2).
    pub const IPMP_CIT: u16 = 0x0003;
    /// Network Information Table (DVB).
    pub const NIT: u16 = 0x0010;
    /// Service Description Table + Bouquet Association Table (DVB).
    pub const SDT_BAT: u16 = 0x0011;
    /// Event Information Table (DVB).
    pub const EIT: u16 = 0x0012;
    /// Running Status Table (DVB).
    pub const RST: u16 = 0x0013;
    /// Time and Date + Time Offset + Stuffing (DVB).
    pub const TDT_TOT: u16 = 0x0014;
    /// Network synchronisation.
    pub const NETWORK_SYNC: u16 = 0x0015;
    /// Resolution Notification Table.
    pub const RNT: u16 = 0x0016;
    /// Inband signalling (reserved).
    pub const INBAND_SIGNALLING: u16 = 0x001C;
    /// Measurement (reserved).
    pub const MEASUREMENT: u16 = 0x001D;
    /// Discontinuity Information Table.
    pub const DIT: u16 = 0x001E;
    /// Selection Information Table.
    pub const SIT: u16 = 0x001F;

    /// ATSC PSIP base PID.
    pub const ATSC_PSIP: u16 = 0x1FFB;

    /// Null-packet padding PID. Payload is ignored.
    pub const NULL: u16 = 0x1FFF;
}

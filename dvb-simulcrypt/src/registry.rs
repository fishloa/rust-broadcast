//! Interface scoping + the message_type / parameter_type / error_status value
//! registries for the two implemented SimulCrypt interfaces.
//!
//! ETSI TS 103 197 V1.5.1 — Table 2 (protocol_version, §4.4.1 p. 27),
//! Table 3 (message_type, §4.4.1 pp. 27-28), Table 5/6 (ECMG⇔SCS parameter /
//! error, §5.2/§5.6) and Table 7/8 (EMMG/PDG⇔MUX parameter / error,
//! §6.2.2/§6.2.6).
//!
//! The `message_type` numeric space is **interface-scoped**: the same 16-bit
//! value means different things on different interfaces (and on the wire the
//! interface is fixed by which TCP connection the message arrived on — this
//! crate takes it as the [`Interface`] hint to [`crate::SimulcryptMessage::parse_on`]).

/// The SimulCrypt connection-oriented interface a message belongs to.
///
/// Only the two head-end CA interfaces are implemented; the remaining
/// TS 103 197 interfaces (C(P)SIG⇔(P)SIG, EIS⇔SCS, (P)SIG⇔MUX, ACG⇔EIS,
/// SIMCOMP⇔MUXCONFIG) share the same framing but are not modelled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Interface {
    /// ECMG ⇔ SCS (TS 103 197 clause 5).
    EcmgScs,
    /// EMMG/PDG ⇔ MUX (TS 103 197 clause 6).
    EmmgPdgMux,
}

impl Interface {
    /// The mandated `protocol_version` byte for this interface (Table 2).
    ///
    /// Both implemented interfaces use `0x03` (the `0x05` DVB-H variant of
    /// annex N is not modelled).
    pub const PROTOCOL_VERSION: u8 = 0x03;

    /// `protocol_version` for this interface (Table 2, §4.4.1 p. 27).
    #[must_use]
    pub const fn protocol_version(self) -> u8 {
        match self {
            Self::EcmgScs | Self::EmmgPdgMux => Self::PROTOCOL_VERSION,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::EcmgScs => "ECMG<->SCS",
            Self::EmmgPdgMux => "EMMG/PDG<->MUX",
        }
    }
}
dvb_common::impl_spec_display!(Interface);

// ===========================================================================
// message_type — ECMG ⇔ SCS (Table 3 subset, §5)
// ===========================================================================

/// ECMG⇔SCS `message_type` values (TS 103 197 Table 3, §4.4.1 pp. 27-28).
///
/// `Reserved(u16)` is the catch-all for any value outside the registry
/// (DVB-reserved or user-defined); it preserves the raw 16-bit value so
/// `Display`/serialize stay lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EcmgScsMessageType {
    /// `0x0001` channel_setup.
    ChannelSetup,
    /// `0x0002` channel_test.
    ChannelTest,
    /// `0x0003` channel_status.
    ChannelStatus,
    /// `0x0004` channel_close.
    ChannelClose,
    /// `0x0005` channel_error.
    ChannelError,
    /// `0x0101` stream_setup.
    StreamSetup,
    /// `0x0102` stream_test.
    StreamTest,
    /// `0x0103` stream_status.
    StreamStatus,
    /// `0x0104` stream_close_request.
    StreamCloseRequest,
    /// `0x0105` stream_close_response.
    StreamCloseResponse,
    /// `0x0106` stream_error.
    StreamError,
    /// `0x0201` CW_provision.
    CwProvision,
    /// `0x0202` ECM_response.
    EcmResponse,
    /// Any value not in the ECMG⇔SCS registry (DVB-reserved / user-defined).
    Reserved(u16),
}

impl EcmgScsMessageType {
    /// Decode a 16-bit `message_type` for the ECMG⇔SCS interface.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x0001 => Self::ChannelSetup,
            0x0002 => Self::ChannelTest,
            0x0003 => Self::ChannelStatus,
            0x0004 => Self::ChannelClose,
            0x0005 => Self::ChannelError,
            0x0101 => Self::StreamSetup,
            0x0102 => Self::StreamTest,
            0x0103 => Self::StreamStatus,
            0x0104 => Self::StreamCloseRequest,
            0x0105 => Self::StreamCloseResponse,
            0x0106 => Self::StreamError,
            0x0201 => Self::CwProvision,
            0x0202 => Self::EcmResponse,
            other => Self::Reserved(other),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::ChannelSetup => 0x0001,
            Self::ChannelTest => 0x0002,
            Self::ChannelStatus => 0x0003,
            Self::ChannelClose => 0x0004,
            Self::ChannelError => 0x0005,
            Self::StreamSetup => 0x0101,
            Self::StreamTest => 0x0102,
            Self::StreamStatus => 0x0103,
            Self::StreamCloseRequest => 0x0104,
            Self::StreamCloseResponse => 0x0105,
            Self::StreamError => 0x0106,
            Self::CwProvision => 0x0201,
            Self::EcmResponse => 0x0202,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ChannelSetup => "channel_setup",
            Self::ChannelTest => "channel_test",
            Self::ChannelStatus => "channel_status",
            Self::ChannelClose => "channel_close",
            Self::ChannelError => "channel_error",
            Self::StreamSetup => "stream_setup",
            Self::StreamTest => "stream_test",
            Self::StreamStatus => "stream_status",
            Self::StreamCloseRequest => "stream_close_request",
            Self::StreamCloseResponse => "stream_close_response",
            Self::StreamError => "stream_error",
            Self::CwProvision => "CW_provision",
            Self::EcmResponse => "ECM_response",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EcmgScsMessageType, Reserved);

// ===========================================================================
// message_type — EMMG/PDG ⇔ MUX (Table 3 subset, §6)
// ===========================================================================

/// EMMG/PDG⇔MUX `message_type` values (TS 103 197 Table 3, §4.4.1 pp. 27-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EmmgMuxMessageType {
    /// `0x0011` channel_setup.
    ChannelSetup,
    /// `0x0012` channel_test.
    ChannelTest,
    /// `0x0013` channel_status.
    ChannelStatus,
    /// `0x0014` channel_close.
    ChannelClose,
    /// `0x0015` channel_error.
    ChannelError,
    /// `0x0111` stream_setup.
    StreamSetup,
    /// `0x0112` stream_test.
    StreamTest,
    /// `0x0113` stream_status.
    StreamStatus,
    /// `0x0114` stream_close_request.
    StreamCloseRequest,
    /// `0x0115` stream_close_response.
    StreamCloseResponse,
    /// `0x0116` stream_error.
    StreamError,
    /// `0x0117` stream_BW_request.
    StreamBwRequest,
    /// `0x0118` stream_BW_allocation.
    StreamBwAllocation,
    /// `0x0211` data_provision.
    DataProvision,
    /// Any value not in the EMMG/PDG⇔MUX registry (DVB-reserved / user-defined).
    Reserved(u16),
}

impl EmmgMuxMessageType {
    /// Decode a 16-bit `message_type` for the EMMG/PDG⇔MUX interface.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x0011 => Self::ChannelSetup,
            0x0012 => Self::ChannelTest,
            0x0013 => Self::ChannelStatus,
            0x0014 => Self::ChannelClose,
            0x0015 => Self::ChannelError,
            0x0111 => Self::StreamSetup,
            0x0112 => Self::StreamTest,
            0x0113 => Self::StreamStatus,
            0x0114 => Self::StreamCloseRequest,
            0x0115 => Self::StreamCloseResponse,
            0x0116 => Self::StreamError,
            0x0117 => Self::StreamBwRequest,
            0x0118 => Self::StreamBwAllocation,
            0x0211 => Self::DataProvision,
            other => Self::Reserved(other),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::ChannelSetup => 0x0011,
            Self::ChannelTest => 0x0012,
            Self::ChannelStatus => 0x0013,
            Self::ChannelClose => 0x0014,
            Self::ChannelError => 0x0015,
            Self::StreamSetup => 0x0111,
            Self::StreamTest => 0x0112,
            Self::StreamStatus => 0x0113,
            Self::StreamCloseRequest => 0x0114,
            Self::StreamCloseResponse => 0x0115,
            Self::StreamError => 0x0116,
            Self::StreamBwRequest => 0x0117,
            Self::StreamBwAllocation => 0x0118,
            Self::DataProvision => 0x0211,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ChannelSetup => "channel_setup",
            Self::ChannelTest => "channel_test",
            Self::ChannelStatus => "channel_status",
            Self::ChannelClose => "channel_close",
            Self::ChannelError => "channel_error",
            Self::StreamSetup => "stream_setup",
            Self::StreamTest => "stream_test",
            Self::StreamStatus => "stream_status",
            Self::StreamCloseRequest => "stream_close_request",
            Self::StreamCloseResponse => "stream_close_response",
            Self::StreamError => "stream_error",
            Self::StreamBwRequest => "stream_BW_request",
            Self::StreamBwAllocation => "stream_BW_allocation",
            Self::DataProvision => "data_provision",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EmmgMuxMessageType, Reserved);

/// Interface-tagged `message_type`: decode a raw value once the [`Interface`]
/// is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MessageType {
    /// An ECMG⇔SCS message_type.
    EcmgScs(EcmgScsMessageType),
    /// An EMMG/PDG⇔MUX message_type.
    EmmgPdgMux(EmmgMuxMessageType),
}

impl MessageType {
    /// Decode a raw 16-bit `message_type` for the given interface.
    #[must_use]
    pub const fn from_u16(iface: Interface, v: u16) -> Self {
        match iface {
            Interface::EcmgScs => Self::EcmgScs(EcmgScsMessageType::from_u16(v)),
            Interface::EmmgPdgMux => Self::EmmgPdgMux(EmmgMuxMessageType::from_u16(v)),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::EcmgScs(m) => m.to_u16(),
            Self::EmmgPdgMux(m) => m.to_u16(),
        }
    }

    /// The interface this message_type was decoded against.
    #[must_use]
    pub const fn interface(self) -> Interface {
        match self {
            Self::EcmgScs(_) => Interface::EcmgScs,
            Self::EmmgPdgMux(_) => Interface::EmmgPdgMux,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::EcmgScs(m) => m.name(),
            Self::EmmgPdgMux(m) => m.name(),
        }
    }
}
dvb_common::impl_spec_display!(MessageType);

// ===========================================================================
// parameter_type — ECMG ⇔ SCS (Table 5, §5.2 p. 31)
// ===========================================================================

/// ECMG⇔SCS `parameter_type` values (TS 103 197 Table 5, §5.2 p. 31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EcmgScsParameterType {
    /// `0x0001` Super_CAS_id (uimsbf, 4 bytes).
    SuperCasId,
    /// `0x0002` section_TSpkt_flag (uimsbf, 1 byte).
    SectionTspktFlag,
    /// `0x0003` delay_start (tcimsbf/ms, 2 bytes).
    DelayStart,
    /// `0x0004` delay_stop (tcimsbf/ms, 2 bytes).
    DelayStop,
    /// `0x0005` transition_delay_start (tcimsbf/ms, 2 bytes).
    TransitionDelayStart,
    /// `0x0006` transition_delay_stop (tcimsbf/ms, 2 bytes).
    TransitionDelayStop,
    /// `0x0007` ECM_rep_period (uimsbf/ms, 2 bytes).
    EcmRepPeriod,
    /// `0x0008` max_streams (uimsbf, 2 bytes).
    MaxStreams,
    /// `0x0009` min_CP_duration (uimsbf/n×100ms, 2 bytes).
    MinCpDuration,
    /// `0x000A` lead_CW (uimsbf, 1 byte).
    LeadCw,
    /// `0x000B` CW_per_msg (uimsbf, 1 byte).
    CwPerMsg,
    /// `0x000C` max_comp_time (uimsbf/ms, 2 bytes).
    MaxCompTime,
    /// `0x000D` access_criteria (user defined, variable).
    AccessCriteria,
    /// `0x000E` ECM_channel_id (uimsbf, 2 bytes).
    EcmChannelId,
    /// `0x000F` ECM_stream_id (uimsbf, 2 bytes).
    EcmStreamId,
    /// `0x0010` nominal_CP_duration (uimsbf/n×100ms, 2 bytes).
    NominalCpDuration,
    /// `0x0011` access_criteria_transfer_mode (Boolean, 1 byte).
    AccessCriteriaTransferMode,
    /// `0x0012` CP_number (uimsbf, 2 bytes).
    CpNumber,
    /// `0x0013` CP_duration (uimsbf/n×100ms, 2 bytes).
    CpDuration,
    /// `0x0014` CP_CW_combination (CP uimsbf 2B + CW variable; compound, opaque CW).
    CpCwCombination,
    /// `0x0015` ECM_datagram (user defined, variable; opaque).
    EcmDatagram,
    /// `0x0016` AC_delay_start (tcimsbf/ms, 2 bytes).
    AcDelayStart,
    /// `0x0017` AC_delay_stop (tcimsbf/ms, 2 bytes).
    AcDelayStop,
    /// `0x0018` CW_encryption (user defined, variable; opaque).
    CwEncryption,
    /// `0x0019` ECM_id (uimsbf, 2 bytes).
    EcmId,
    /// `0x7000` error_status (2 bytes; see [`EcmgErrorStatus`]).
    ErrorStatus,
    /// `0x7001` error_information (user defined, variable).
    ErrorInformation,
    /// Any value not in the ECMG⇔SCS registry (DVB-reserved / user-defined).
    Reserved(u16),
}

impl EcmgScsParameterType {
    /// Decode a 16-bit `parameter_type` for the ECMG⇔SCS interface.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x0001 => Self::SuperCasId,
            0x0002 => Self::SectionTspktFlag,
            0x0003 => Self::DelayStart,
            0x0004 => Self::DelayStop,
            0x0005 => Self::TransitionDelayStart,
            0x0006 => Self::TransitionDelayStop,
            0x0007 => Self::EcmRepPeriod,
            0x0008 => Self::MaxStreams,
            0x0009 => Self::MinCpDuration,
            0x000A => Self::LeadCw,
            0x000B => Self::CwPerMsg,
            0x000C => Self::MaxCompTime,
            0x000D => Self::AccessCriteria,
            0x000E => Self::EcmChannelId,
            0x000F => Self::EcmStreamId,
            0x0010 => Self::NominalCpDuration,
            0x0011 => Self::AccessCriteriaTransferMode,
            0x0012 => Self::CpNumber,
            0x0013 => Self::CpDuration,
            0x0014 => Self::CpCwCombination,
            0x0015 => Self::EcmDatagram,
            0x0016 => Self::AcDelayStart,
            0x0017 => Self::AcDelayStop,
            0x0018 => Self::CwEncryption,
            0x0019 => Self::EcmId,
            0x7000 => Self::ErrorStatus,
            0x7001 => Self::ErrorInformation,
            other => Self::Reserved(other),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::SuperCasId => 0x0001,
            Self::SectionTspktFlag => 0x0002,
            Self::DelayStart => 0x0003,
            Self::DelayStop => 0x0004,
            Self::TransitionDelayStart => 0x0005,
            Self::TransitionDelayStop => 0x0006,
            Self::EcmRepPeriod => 0x0007,
            Self::MaxStreams => 0x0008,
            Self::MinCpDuration => 0x0009,
            Self::LeadCw => 0x000A,
            Self::CwPerMsg => 0x000B,
            Self::MaxCompTime => 0x000C,
            Self::AccessCriteria => 0x000D,
            Self::EcmChannelId => 0x000E,
            Self::EcmStreamId => 0x000F,
            Self::NominalCpDuration => 0x0010,
            Self::AccessCriteriaTransferMode => 0x0011,
            Self::CpNumber => 0x0012,
            Self::CpDuration => 0x0013,
            Self::CpCwCombination => 0x0014,
            Self::EcmDatagram => 0x0015,
            Self::AcDelayStart => 0x0016,
            Self::AcDelayStop => 0x0017,
            Self::CwEncryption => 0x0018,
            Self::EcmId => 0x0019,
            Self::ErrorStatus => 0x7000,
            Self::ErrorInformation => 0x7001,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention (the spec token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SuperCasId => "Super_CAS_id",
            Self::SectionTspktFlag => "section_TSpkt_flag",
            Self::DelayStart => "delay_start",
            Self::DelayStop => "delay_stop",
            Self::TransitionDelayStart => "transition_delay_start",
            Self::TransitionDelayStop => "transition_delay_stop",
            Self::EcmRepPeriod => "ECM_rep_period",
            Self::MaxStreams => "max_streams",
            Self::MinCpDuration => "min_CP_duration",
            Self::LeadCw => "lead_CW",
            Self::CwPerMsg => "CW_per_msg",
            Self::MaxCompTime => "max_comp_time",
            Self::AccessCriteria => "access_criteria",
            Self::EcmChannelId => "ECM_channel_id",
            Self::EcmStreamId => "ECM_stream_id",
            Self::NominalCpDuration => "nominal_CP_duration",
            Self::AccessCriteriaTransferMode => "access_criteria_transfer_mode",
            Self::CpNumber => "CP_number",
            Self::CpDuration => "CP_duration",
            Self::CpCwCombination => "CP_CW_combination",
            Self::EcmDatagram => "ECM_datagram",
            Self::AcDelayStart => "AC_delay_start",
            Self::AcDelayStop => "AC_delay_stop",
            Self::CwEncryption => "CW_encryption",
            Self::EcmId => "ECM_id",
            Self::ErrorStatus => "error_status",
            Self::ErrorInformation => "error_information",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EcmgScsParameterType, Reserved);

// ===========================================================================
// parameter_type — EMMG/PDG ⇔ MUX (Table 7, §6.2.2 p. 42)
// ===========================================================================

/// EMMG/PDG⇔MUX `parameter_type` values (TS 103 197 Table 7, §6.2.2 p. 42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EmmgMuxParameterType {
    /// `0x0001` client_id (uimsbf, 4 bytes).
    ClientId,
    /// `0x0002` section_TSpkt_flag (uimsbf, 1 byte).
    SectionTspktFlag,
    /// `0x0003` data_channel_id (uimsbf, 2 bytes).
    DataChannelId,
    /// `0x0004` data_stream_id (uimsbf, 2 bytes).
    DataStreamId,
    /// `0x0005` datagram (user defined, variable; opaque).
    Datagram,
    /// `0x0006` bandwidth (uimsbf/kbit/s, 2 bytes).
    Bandwidth,
    /// `0x0007` data_type (uimsbf, 1 byte; see [`DataType`]).
    DataType,
    /// `0x0008` data_id (uimsbf, 2 bytes).
    DataId,
    /// `0x7000` error_status (2 bytes; see [`EmmgErrorStatus`]).
    ErrorStatus,
    /// `0x7001` error_information (user defined, variable).
    ErrorInformation,
    /// Any value not in the EMMG/PDG⇔MUX registry (DVB-reserved / user-defined).
    Reserved(u16),
}

impl EmmgMuxParameterType {
    /// Decode a 16-bit `parameter_type` for the EMMG/PDG⇔MUX interface.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x0001 => Self::ClientId,
            0x0002 => Self::SectionTspktFlag,
            0x0003 => Self::DataChannelId,
            0x0004 => Self::DataStreamId,
            0x0005 => Self::Datagram,
            0x0006 => Self::Bandwidth,
            0x0007 => Self::DataType,
            0x0008 => Self::DataId,
            0x7000 => Self::ErrorStatus,
            0x7001 => Self::ErrorInformation,
            other => Self::Reserved(other),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::ClientId => 0x0001,
            Self::SectionTspktFlag => 0x0002,
            Self::DataChannelId => 0x0003,
            Self::DataStreamId => 0x0004,
            Self::Datagram => 0x0005,
            Self::Bandwidth => 0x0006,
            Self::DataType => 0x0007,
            Self::DataId => 0x0008,
            Self::ErrorStatus => 0x7000,
            Self::ErrorInformation => 0x7001,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention (the spec token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ClientId => "client_id",
            Self::SectionTspktFlag => "section_TSpkt_flag",
            Self::DataChannelId => "data_channel_id",
            Self::DataStreamId => "data_stream_id",
            Self::Datagram => "datagram",
            Self::Bandwidth => "bandwidth",
            Self::DataType => "data_type",
            Self::DataId => "data_id",
            Self::ErrorStatus => "error_status",
            Self::ErrorInformation => "error_information",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EmmgMuxParameterType, Reserved);

/// Interface-tagged `parameter_type`: decode a raw value once the [`Interface`]
/// is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ParameterType {
    /// An ECMG⇔SCS parameter_type.
    EcmgScs(EcmgScsParameterType),
    /// An EMMG/PDG⇔MUX parameter_type.
    EmmgPdgMux(EmmgMuxParameterType),
}

impl ParameterType {
    /// Decode a raw 16-bit `parameter_type` for the given interface.
    #[must_use]
    pub const fn from_u16(iface: Interface, v: u16) -> Self {
        match iface {
            Interface::EcmgScs => Self::EcmgScs(EcmgScsParameterType::from_u16(v)),
            Interface::EmmgPdgMux => Self::EmmgPdgMux(EmmgMuxParameterType::from_u16(v)),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::EcmgScs(p) => p.to_u16(),
            Self::EmmgPdgMux(p) => p.to_u16(),
        }
    }

    /// The interface this parameter_type was decoded against.
    #[must_use]
    pub const fn interface(self) -> Interface {
        match self {
            Self::EcmgScs(_) => Interface::EcmgScs,
            Self::EmmgPdgMux(_) => Interface::EmmgPdgMux,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::EcmgScs(p) => p.name(),
            Self::EmmgPdgMux(p) => p.name(),
        }
    }
}
dvb_common::impl_spec_display!(ParameterType);

// ===========================================================================
// EMMG/PDG value sub-tables (§6.2.3)
// ===========================================================================

/// `data_type` values (TS 103 197 §6.2.3 p. 42) — what a `datagram` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DataType {
    /// `0x00` EMM.
    Emm,
    /// `0x01` private data.
    PrivateData,
    /// `0x02` DVB reserved (ECM).
    Ecm,
    /// Any other value (DVB reserved).
    Reserved(u8),
}

impl DataType {
    /// Decode a `data_type` byte.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Emm,
            0x01 => Self::PrivateData,
            0x02 => Self::Ecm,
            other => Self::Reserved(other),
        }
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Emm => 0x00,
            Self::PrivateData => 0x01,
            Self::Ecm => 0x02,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Emm => "EMM",
            Self::PrivateData => "private data",
            Self::Ecm => "ECM",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DataType, Reserved);

/// `section_TSpkt_flag` values (TS 103 197 §6.2.3 p. 43) — the datagram framing
/// in `datagram` parameters. (The same flag, with the same meaning, is carried
/// on the ECMG⇔SCS interface for `ECM_datagram`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SectionTspktFlag {
    /// `0x00` EMMs / private datagrams in MPEG-2 section format.
    SectionFormat,
    /// `0x01` MPEG-2 TS packet format (all TS packets 188 bytes).
    TsPacketFormat,
    /// `0x02` arbitrary-length EMMs/KMMs per IP Datacast SPP (annex N).
    ArbitraryLength,
    /// Any other value (DVB reserved).
    Reserved(u8),
}

impl SectionTspktFlag {
    /// Decode a `section_TSpkt_flag` byte.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::SectionFormat,
            0x01 => Self::TsPacketFormat,
            0x02 => Self::ArbitraryLength,
            other => Self::Reserved(other),
        }
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::SectionFormat => 0x00,
            Self::TsPacketFormat => 0x01,
            Self::ArbitraryLength => 0x02,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SectionFormat => "section_format",
            Self::TsPacketFormat => "ts_packet_format",
            Self::ArbitraryLength => "arbitrary_length",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(SectionTspktFlag, Reserved);

// ===========================================================================
// error_status (Table 6 / Table 8)
// ===========================================================================

/// ECMG⇔SCS `error_status` values (TS 103 197 Table 6, §5.6 p. 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EcmgErrorStatus {
    /// `0x0001` invalid message.
    InvalidMessage,
    /// `0x0002` unsupported protocol version.
    UnsupportedProtocolVersion,
    /// `0x0003` unknown message_type value.
    UnknownMessageType,
    /// `0x0004` message too long.
    MessageTooLong,
    /// `0x0005` unknown Super_CAS_id value.
    UnknownSuperCasId,
    /// `0x0006` unknown ECM_channel_id value.
    UnknownEcmChannelId,
    /// `0x0007` unknown ECM_stream_id value.
    UnknownEcmStreamId,
    /// `0x0008` too many channels on this ECMG.
    TooManyChannels,
    /// `0x0009` too many ECM streams on this channel.
    TooManyStreamsOnChannel,
    /// `0x000A` too many ECM streams on this ECMG.
    TooManyStreamsOnEcmg,
    /// `0x000B` not enough control words to compute ECM.
    NotEnoughControlWords,
    /// `0x000C` ECMG out of storage capacity.
    OutOfStorage,
    /// `0x000D` ECMG out of computational resources.
    OutOfResources,
    /// `0x000E` unknown parameter_type value.
    UnknownParameterType,
    /// `0x000F` inconsistent length for DVB parameter.
    InconsistentLength,
    /// `0x0010` missing mandatory DVB parameter.
    MissingMandatoryParameter,
    /// `0x0011` invalid value for DVB parameter.
    InvalidParameterValue,
    /// `0x0012` unknown ECM_id value.
    UnknownEcmId,
    /// `0x0013` ECM_channel_id value already in use.
    EcmChannelIdInUse,
    /// `0x0014` ECM_stream_id value already in use.
    EcmStreamIdInUse,
    /// `0x0015` ECM_id value already in use.
    EcmIdInUse,
    /// `0x7000` unknown error.
    UnknownError,
    /// `0x7001` unrecoverable error.
    UnrecoverableError,
    /// Any value not in the registry (DVB-reserved / CA-system / user-defined).
    Reserved(u16),
}

impl EcmgErrorStatus {
    /// Decode a 16-bit `error_status` value.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x0001 => Self::InvalidMessage,
            0x0002 => Self::UnsupportedProtocolVersion,
            0x0003 => Self::UnknownMessageType,
            0x0004 => Self::MessageTooLong,
            0x0005 => Self::UnknownSuperCasId,
            0x0006 => Self::UnknownEcmChannelId,
            0x0007 => Self::UnknownEcmStreamId,
            0x0008 => Self::TooManyChannels,
            0x0009 => Self::TooManyStreamsOnChannel,
            0x000A => Self::TooManyStreamsOnEcmg,
            0x000B => Self::NotEnoughControlWords,
            0x000C => Self::OutOfStorage,
            0x000D => Self::OutOfResources,
            0x000E => Self::UnknownParameterType,
            0x000F => Self::InconsistentLength,
            0x0010 => Self::MissingMandatoryParameter,
            0x0011 => Self::InvalidParameterValue,
            0x0012 => Self::UnknownEcmId,
            0x0013 => Self::EcmChannelIdInUse,
            0x0014 => Self::EcmStreamIdInUse,
            0x0015 => Self::EcmIdInUse,
            0x7000 => Self::UnknownError,
            0x7001 => Self::UnrecoverableError,
            other => Self::Reserved(other),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::InvalidMessage => 0x0001,
            Self::UnsupportedProtocolVersion => 0x0002,
            Self::UnknownMessageType => 0x0003,
            Self::MessageTooLong => 0x0004,
            Self::UnknownSuperCasId => 0x0005,
            Self::UnknownEcmChannelId => 0x0006,
            Self::UnknownEcmStreamId => 0x0007,
            Self::TooManyChannels => 0x0008,
            Self::TooManyStreamsOnChannel => 0x0009,
            Self::TooManyStreamsOnEcmg => 0x000A,
            Self::NotEnoughControlWords => 0x000B,
            Self::OutOfStorage => 0x000C,
            Self::OutOfResources => 0x000D,
            Self::UnknownParameterType => 0x000E,
            Self::InconsistentLength => 0x000F,
            Self::MissingMandatoryParameter => 0x0010,
            Self::InvalidParameterValue => 0x0011,
            Self::UnknownEcmId => 0x0012,
            Self::EcmChannelIdInUse => 0x0013,
            Self::EcmStreamIdInUse => 0x0014,
            Self::EcmIdInUse => 0x0015,
            Self::UnknownError => 0x7000,
            Self::UnrecoverableError => 0x7001,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InvalidMessage => "invalid message",
            Self::UnsupportedProtocolVersion => "unsupported protocol version",
            Self::UnknownMessageType => "unknown message_type value",
            Self::MessageTooLong => "message too long",
            Self::UnknownSuperCasId => "unknown Super_CAS_id value",
            Self::UnknownEcmChannelId => "unknown ECM_channel_id value",
            Self::UnknownEcmStreamId => "unknown ECM_stream_id value",
            Self::TooManyChannels => "too many channels on this ECMG",
            Self::TooManyStreamsOnChannel => "too many ECM streams on this channel",
            Self::TooManyStreamsOnEcmg => "too many ECM streams on this ECMG",
            Self::NotEnoughControlWords => "not enough control words to compute ECM",
            Self::OutOfStorage => "ECMG out of storage capacity",
            Self::OutOfResources => "ECMG out of computational resources",
            Self::UnknownParameterType => "unknown parameter_type value",
            Self::InconsistentLength => "inconsistent length for DVB parameter",
            Self::MissingMandatoryParameter => "missing mandatory DVB parameter",
            Self::InvalidParameterValue => "invalid value for DVB parameter",
            Self::UnknownEcmId => "unknown ECM_id value",
            Self::EcmChannelIdInUse => "ECM_channel_id value already in use",
            Self::EcmStreamIdInUse => "ECM_stream_id value already in use",
            Self::EcmIdInUse => "ECM_id value already in use",
            Self::UnknownError => "unknown error",
            Self::UnrecoverableError => "unrecoverable error",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EcmgErrorStatus, Reserved);

/// EMMG/PDG⇔MUX `error_status` values (TS 103 197 Table 8, §6.2.6 p. 47).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EmmgErrorStatus {
    /// `0x0001` invalid message.
    InvalidMessage,
    /// `0x0002` unsupported protocol version.
    UnsupportedProtocolVersion,
    /// `0x0003` unknown message_type value.
    UnknownMessageType,
    /// `0x0004` message too long.
    MessageTooLong,
    /// `0x0005` unknown data_stream_id value.
    UnknownDataStreamId,
    /// `0x0006` unknown data_channel_id value.
    UnknownDataChannelId,
    /// `0x0007` too many channels on this MUX.
    TooManyChannels,
    /// `0x0008` too many data streams on this channel.
    TooManyStreamsOnChannel,
    /// `0x0009` too many data streams on this MUX.
    TooManyStreamsOnMux,
    /// `0x000A` unknown parameter_type.
    UnknownParameterType,
    /// `0x000B` inconsistent length for DVB parameter.
    InconsistentLength,
    /// `0x000C` missing mandatory DVB parameter.
    MissingMandatoryParameter,
    /// `0x000D` invalid value for DVB parameter.
    InvalidParameterValue,
    /// `0x000E` unknown client_id value.
    UnknownClientId,
    /// `0x000F` exceeded bandwidth.
    ExceededBandwidth,
    /// `0x0010` unknown data_id value.
    UnknownDataId,
    /// `0x0011` data_channel_id value already in use.
    DataChannelIdInUse,
    /// `0x0012` data_stream_id value already in use.
    DataStreamIdInUse,
    /// `0x0013` data_id value already in use.
    DataIdInUse,
    /// `0x0014` client_id value already in use.
    ClientIdInUse,
    /// `0x7000` unknown error.
    UnknownError,
    /// `0x7001` unrecoverable error.
    UnrecoverableError,
    /// Any value not in the registry (DVB-reserved / CA-system / user-defined).
    Reserved(u16),
}

impl EmmgErrorStatus {
    /// Decode a 16-bit `error_status` value.
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x0001 => Self::InvalidMessage,
            0x0002 => Self::UnsupportedProtocolVersion,
            0x0003 => Self::UnknownMessageType,
            0x0004 => Self::MessageTooLong,
            0x0005 => Self::UnknownDataStreamId,
            0x0006 => Self::UnknownDataChannelId,
            0x0007 => Self::TooManyChannels,
            0x0008 => Self::TooManyStreamsOnChannel,
            0x0009 => Self::TooManyStreamsOnMux,
            0x000A => Self::UnknownParameterType,
            0x000B => Self::InconsistentLength,
            0x000C => Self::MissingMandatoryParameter,
            0x000D => Self::InvalidParameterValue,
            0x000E => Self::UnknownClientId,
            0x000F => Self::ExceededBandwidth,
            0x0010 => Self::UnknownDataId,
            0x0011 => Self::DataChannelIdInUse,
            0x0012 => Self::DataStreamIdInUse,
            0x0013 => Self::DataIdInUse,
            0x0014 => Self::ClientIdInUse,
            0x7000 => Self::UnknownError,
            0x7001 => Self::UnrecoverableError,
            other => Self::Reserved(other),
        }
    }

    /// The 16-bit wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::InvalidMessage => 0x0001,
            Self::UnsupportedProtocolVersion => 0x0002,
            Self::UnknownMessageType => 0x0003,
            Self::MessageTooLong => 0x0004,
            Self::UnknownDataStreamId => 0x0005,
            Self::UnknownDataChannelId => 0x0006,
            Self::TooManyChannels => 0x0007,
            Self::TooManyStreamsOnChannel => 0x0008,
            Self::TooManyStreamsOnMux => 0x0009,
            Self::UnknownParameterType => 0x000A,
            Self::InconsistentLength => 0x000B,
            Self::MissingMandatoryParameter => 0x000C,
            Self::InvalidParameterValue => 0x000D,
            Self::UnknownClientId => 0x000E,
            Self::ExceededBandwidth => 0x000F,
            Self::UnknownDataId => 0x0010,
            Self::DataChannelIdInUse => 0x0011,
            Self::DataStreamIdInUse => 0x0012,
            Self::DataIdInUse => 0x0013,
            Self::ClientIdInUse => 0x0014,
            Self::UnknownError => 0x7000,
            Self::UnrecoverableError => 0x7001,
            Self::Reserved(v) => v,
        }
    }

    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InvalidMessage => "invalid message",
            Self::UnsupportedProtocolVersion => "unsupported protocol version",
            Self::UnknownMessageType => "unknown message_type value",
            Self::MessageTooLong => "message too long",
            Self::UnknownDataStreamId => "unknown data_stream_id value",
            Self::UnknownDataChannelId => "unknown data_channel_id value",
            Self::TooManyChannels => "too many channels on this MUX",
            Self::TooManyStreamsOnChannel => "too many data streams on this channel",
            Self::TooManyStreamsOnMux => "too many data streams on this MUX",
            Self::UnknownParameterType => "unknown parameter_type",
            Self::InconsistentLength => "inconsistent length for DVB parameter",
            Self::MissingMandatoryParameter => "missing mandatory DVB parameter",
            Self::InvalidParameterValue => "invalid value for DVB parameter",
            Self::UnknownClientId => "unknown client_id value",
            Self::ExceededBandwidth => "exceeded bandwidth",
            Self::UnknownDataId => "unknown data_id value",
            Self::DataChannelIdInUse => "data_channel_id value already in use",
            Self::DataStreamIdInUse => "data_stream_id value already in use",
            Self::DataIdInUse => "data_id value already in use",
            Self::ClientIdInUse => "client_id value already in use",
            Self::UnknownError => "unknown error",
            Self::UnrecoverableError => "unrecoverable error",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EmmgErrorStatus, Reserved);

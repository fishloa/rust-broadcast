//! `DescriptorTag` enum — typed descriptor tag byte values.
//!
//! Source: ETSI EN 300 468 §6.1 Table 12 plus MPEG-2 descriptors from
//! ISO/IEC 13818-1 when they appear inside PMT.
//!
//! Phase-1 of the crate populates this enum incrementally as descriptors land.
//! The enum is intentionally non-exhaustive so adding variants is a minor bump.

/// Typed descriptor tag.
///
/// Unknown or reserved tags are returned as
/// [`crate::descriptors::AnyDescriptor::Unknown`] during parse and should not
/// appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, num_enum::TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[repr(u8)]
#[allow(missing_docs)]
#[rustfmt::skip] // deliberate column-aligned discriminants
pub enum DescriptorTag {
    // ── MPEG-2 descriptors (0x02..=0x3F) ───────────────────────────────────
    // Populated as phases 2+ land.
    VideoStream                = 0x02,
    AudioStream                = 0x03,
    Hierarchy                  = 0x04,
    Registration               = 0x05,
    DataStreamAlignment        = 0x06,
    TargetBackgroundGrid       = 0x07,
    VideoWindow                = 0x08,
    ConditionalAccess          = 0x09,
    Iso639Language             = 0x0A,
    SystemClock                = 0x0B,
    MultiplexBufferUtilization = 0x0C,
    Copyright                  = 0x0D,
    MaximumBitrate             = 0x0E,
    PrivateDataIndicator       = 0x0F,
    SmoothingBuffer            = 0x10,
    Std                        = 0x11,
    Ibp                        = 0x12,

    // ── DVB SI descriptors (0x40..=0x7F) ───────────────────────────────────
    NetworkName                = 0x40,
    ServiceList                = 0x41,
    Stuffing                   = 0x42,
    SatelliteDeliverySystem    = 0x43,
    CableDeliverySystem        = 0x44,
    VbiData                    = 0x45,
    VbiTeletext                = 0x46,
    BouquetName                = 0x47,
    Service                    = 0x48,
    CountryAvailability        = 0x49,
    Linkage                    = 0x4A,
    NvodReference              = 0x4B,
    TimeShiftedService         = 0x4C,
    ShortEvent                 = 0x4D,
    ExtendedEvent              = 0x4E,
    TimeShiftedEvent           = 0x4F,
    Component                  = 0x50,
    Mosaic                     = 0x51,
    StreamIdentifier           = 0x52,
    CaIdentifier               = 0x53,
    Content                    = 0x54,
    ParentalRating             = 0x55,
    Teletext                   = 0x56,
    Telephone                  = 0x57,
    LocalTimeOffset            = 0x58,
    Subtitling                 = 0x59,
    TerrestrialDeliverySystem  = 0x5A,
    MultilingualNetworkName    = 0x5B,
    MultilingualBouquetName    = 0x5C,
    MultilingualServiceName    = 0x5D,
    MultilingualComponent      = 0x5E,
    PrivateDataSpecifier       = 0x5F,
    ServiceMove                = 0x60,
    ShortSmoothingBuffer       = 0x61,
    FrequencyList              = 0x62,
    PartialTransportStream     = 0x63,
    DataBroadcast              = 0x64,
    Scrambling                 = 0x65,
    DataBroadcastId            = 0x66,
    TransportStream            = 0x67,
    Dsng                       = 0x68,
    Pdc                        = 0x69,
    Ac3                        = 0x6A,
    AncillaryData              = 0x6B,
    CellList                   = 0x6C,
    CellFrequencyLink          = 0x6D,
    AnnouncementSupport        = 0x6E,
    ApplicationSignalling      = 0x6F,
    AdaptationFieldData        = 0x70,
    ServiceIdentifier          = 0x71,
    ServiceAvailability        = 0x72,
    DefaultAuthority           = 0x73,
    RelatedContent             = 0x74,
    TvaId                      = 0x75,
    ContentIdentifier          = 0x76,
    TimeSliceFecIdentifier     = 0x77,
    EcmRepetitionRate          = 0x78,
    S2SatelliteDeliverySystem  = 0x79,
    EnhancedAc3                = 0x7A,
    Dts                        = 0x7B,
    Aac                        = 0x7C,
    XaitLocation               = 0x7D,
    FtaContentManagement       = 0x7E,
    Extension                  = 0x7F,
}

impl From<DescriptorTag> for u8 {
    fn from(t: DescriptorTag) -> Self {
        t as u8
    }
}

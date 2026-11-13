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
#[rustfmt::skip] // deliberate column-aligned discriminants
pub enum DescriptorTag {
    // ── MPEG-2 descriptors (0x02..=0x3F) ───────────────────────────────────
    // Populated as phases 2+ land.
    /// `VideoStream` (descriptor tag 0x02).
    VideoStream                = 0x02,
    /// `AudioStream` (descriptor tag 0x03).
    AudioStream                = 0x03,
    /// `Hierarchy` (descriptor tag 0x04).
    Hierarchy                  = 0x04,
    /// `Registration` (descriptor tag 0x05).
    Registration               = 0x05,
    /// `DataStreamAlignment` (descriptor tag 0x06).
    DataStreamAlignment        = 0x06,
    /// `TargetBackgroundGrid` (descriptor tag 0x07).
    TargetBackgroundGrid       = 0x07,
    /// `VideoWindow` (descriptor tag 0x08).
    VideoWindow                = 0x08,
    /// `ConditionalAccess` (descriptor tag 0x09).
    ConditionalAccess          = 0x09,
    /// `Iso639Language` (descriptor tag 0x0A).
    Iso639Language             = 0x0A,
    /// `SystemClock` (descriptor tag 0x0B).
    SystemClock                = 0x0B,
    /// `MultiplexBufferUtilization` (descriptor tag 0x0C).
    MultiplexBufferUtilization = 0x0C,
    /// `Copyright` (descriptor tag 0x0D).
    Copyright                  = 0x0D,
    /// `MaximumBitrate` (descriptor tag 0x0E).
    MaximumBitrate             = 0x0E,
    /// `PrivateDataIndicator` (descriptor tag 0x0F).
    PrivateDataIndicator       = 0x0F,
    /// `SmoothingBuffer` (descriptor tag 0x10).
    SmoothingBuffer            = 0x10,
    /// `Std` (descriptor tag 0x11).
    Std                        = 0x11,
    /// `Ibp` (descriptor tag 0x12).
    Ibp                        = 0x12,
    /// `CarouselIdentifier` (descriptor tag 0x13).
    CarouselIdentifier         = 0x13,
    /// `AssociationTag` (descriptor tag 0x14).
    AssociationTag             = 0x14,
    /// `DeferredAssociationTags` (descriptor tag 0x15).
    DeferredAssociationTags    = 0x15,
    /// `Mpeg4Video` (descriptor tag 0x1B).
    Mpeg4Video                 = 0x1B,
    /// `Mpeg4Audio` (descriptor tag 0x1C).
    Mpeg4Audio                 = 0x1C,
    /// `Iod` (descriptor tag 0x1D).
    Iod                        = 0x1D,
    /// `Sl` (descriptor tag 0x1E).
    Sl                         = 0x1E,
    /// `Fmc` (descriptor tag 0x1F).
    Fmc                        = 0x1F,
    /// `ExternalEsId` (descriptor tag 0x20).
    ExternalEsId               = 0x20,
    /// `Muxcode` (descriptor tag 0x21).
    Muxcode                    = 0x21,
    /// `FmxBufferSize` (descriptor tag 0x22).
    FmxBufferSize              = 0x22,
    /// `MultiplexBuffer` (descriptor tag 0x23).
    MultiplexBuffer            = 0x23,
    /// `ContentLabeling` (descriptor tag 0x24).
    ContentLabeling            = 0x24,
    /// `MetadataPointer` (descriptor tag 0x25).
    MetadataPointer            = 0x25,
    /// `Metadata` (descriptor tag 0x26).
    Metadata                   = 0x26,
    /// `MetadataStd` (descriptor tag 0x27).
    MetadataStd                = 0x27,
    /// `AvcVideo` (descriptor tag 0x28).
    AvcVideo                   = 0x28,
    /// `AvcTimingAndHrd` (descriptor tag 0x2A).
    AvcTimingAndHrd            = 0x2A,
    /// `Mpeg2AacAudio` (descriptor tag 0x2B).
    Mpeg2AacAudio              = 0x2B,
    /// `FlexMuxTiming` (descriptor tag 0x2C).
    FlexMuxTiming              = 0x2C,
    /// `Mpeg4Text` (descriptor tag 0x2D).
    Mpeg4Text                  = 0x2D,
    /// `Mpeg4AudioExtension` (descriptor tag 0x2E).
    Mpeg4AudioExtension        = 0x2E,
    /// `AuxiliaryVideoStream` (descriptor tag 0x2F).
    AuxiliaryVideoStream       = 0x2F,
    /// `SvcExtension` (descriptor tag 0x30).
    SvcExtension               = 0x30,
    /// `MvcExtension` (descriptor tag 0x31).
    MvcExtension               = 0x31,
    /// `J2kVideo` (descriptor tag 0x32).
    J2kVideo                   = 0x32,
    /// `MvcOperationPoint` (descriptor tag 0x33).
    MvcOperationPoint          = 0x33,
    /// `Mpeg2StereoscopicVideoFormat` (descriptor tag 0x34).
    Mpeg2StereoscopicVideoFormat = 0x34,
    /// `StereoscopicProgramInfo` (descriptor tag 0x35).
    StereoscopicProgramInfo    = 0x35,
    /// `StereoscopicVideoInfo` (descriptor tag 0x36).
    StereoscopicVideoInfo      = 0x36,
    /// `TransportProfile` (descriptor tag 0x37).
    TransportProfile           = 0x37,
    /// `HevcVideo` (descriptor tag 0x38).
    HevcVideo                  = 0x38,
    /// `VvcVideo` (descriptor tag 0x39).
    VvcVideo                   = 0x39,
    /// `EvcVideo` (descriptor tag 0x3A).
    EvcVideo                   = 0x3A,
    /// `Mpeg2Extension` (descriptor tag 0x3F).
    Mpeg2Extension             = 0x3F,

    // ── DVB SI descriptors (0x40..=0x7F) ───────────────────────────────────
    /// `NetworkName` (descriptor tag 0x40).
    NetworkName                = 0x40,
    /// `ServiceList` (descriptor tag 0x41).
    ServiceList                = 0x41,
    /// `Stuffing` (descriptor tag 0x42).
    Stuffing                   = 0x42,
    /// `SatelliteDeliverySystem` (descriptor tag 0x43).
    SatelliteDeliverySystem    = 0x43,
    /// `CableDeliverySystem` (descriptor tag 0x44).
    CableDeliverySystem        = 0x44,
    /// `VbiData` (descriptor tag 0x45).
    VbiData                    = 0x45,
    /// `VbiTeletext` (descriptor tag 0x46).
    VbiTeletext                = 0x46,
    /// `BouquetName` (descriptor tag 0x47).
    BouquetName                = 0x47,
    /// `Service` (descriptor tag 0x48).
    Service                    = 0x48,
    /// `CountryAvailability` (descriptor tag 0x49).
    CountryAvailability        = 0x49,
    /// `Linkage` (descriptor tag 0x4A).
    Linkage                    = 0x4A,
    /// `NvodReference` (descriptor tag 0x4B).
    NvodReference              = 0x4B,
    /// `TimeShiftedService` (descriptor tag 0x4C).
    TimeShiftedService         = 0x4C,
    /// `ShortEvent` (descriptor tag 0x4D).
    ShortEvent                 = 0x4D,
    /// `ExtendedEvent` (descriptor tag 0x4E).
    ExtendedEvent              = 0x4E,
    /// `TimeShiftedEvent` (descriptor tag 0x4F).
    TimeShiftedEvent           = 0x4F,
    /// `Component` (descriptor tag 0x50).
    Component                  = 0x50,
    /// `Mosaic` (descriptor tag 0x51).
    Mosaic                     = 0x51,
    /// `StreamIdentifier` (descriptor tag 0x52).
    StreamIdentifier           = 0x52,
    /// `CaIdentifier` (descriptor tag 0x53).
    CaIdentifier               = 0x53,
    /// `Content` (descriptor tag 0x54).
    Content                    = 0x54,
    /// `ParentalRating` (descriptor tag 0x55).
    ParentalRating             = 0x55,
    /// `Teletext` (descriptor tag 0x56).
    Teletext                   = 0x56,
    /// `Telephone` (descriptor tag 0x57).
    Telephone                  = 0x57,
    /// `LocalTimeOffset` (descriptor tag 0x58).
    LocalTimeOffset            = 0x58,
    /// `Subtitling` (descriptor tag 0x59).
    Subtitling                 = 0x59,
    /// `TerrestrialDeliverySystem` (descriptor tag 0x5A).
    TerrestrialDeliverySystem  = 0x5A,
    /// `MultilingualNetworkName` (descriptor tag 0x5B).
    MultilingualNetworkName    = 0x5B,
    /// `MultilingualBouquetName` (descriptor tag 0x5C).
    MultilingualBouquetName    = 0x5C,
    /// `MultilingualServiceName` (descriptor tag 0x5D).
    MultilingualServiceName    = 0x5D,
    /// `MultilingualComponent` (descriptor tag 0x5E).
    MultilingualComponent      = 0x5E,
    /// `PrivateDataSpecifier` (descriptor tag 0x5F).
    PrivateDataSpecifier       = 0x5F,
    /// `ServiceMove` (descriptor tag 0x60).
    ServiceMove                = 0x60,
    /// `ShortSmoothingBuffer` (descriptor tag 0x61).
    ShortSmoothingBuffer       = 0x61,
    /// `FrequencyList` (descriptor tag 0x62).
    FrequencyList              = 0x62,
    /// `PartialTransportStream` (descriptor tag 0x63).
    PartialTransportStream     = 0x63,
    /// `DataBroadcast` (descriptor tag 0x64).
    DataBroadcast              = 0x64,
    /// `Scrambling` (descriptor tag 0x65).
    Scrambling                 = 0x65,
    /// `DataBroadcastId` (descriptor tag 0x66).
    DataBroadcastId            = 0x66,
    /// `TransportStream` (descriptor tag 0x67).
    TransportStream            = 0x67,
    /// `Dsng` (descriptor tag 0x68).
    Dsng                       = 0x68,
    /// `Pdc` (descriptor tag 0x69).
    Pdc                        = 0x69,
    /// `Ac3` (descriptor tag 0x6A).
    Ac3                        = 0x6A,
    /// `AncillaryData` (descriptor tag 0x6B).
    AncillaryData              = 0x6B,
    /// `CellList` (descriptor tag 0x6C).
    CellList                   = 0x6C,
    /// `CellFrequencyLink` (descriptor tag 0x6D).
    CellFrequencyLink          = 0x6D,
    /// `AnnouncementSupport` (descriptor tag 0x6E).
    AnnouncementSupport        = 0x6E,
    /// `ApplicationSignalling` (descriptor tag 0x6F).
    ApplicationSignalling      = 0x6F,
    /// `AdaptationFieldData` (descriptor tag 0x70).
    AdaptationFieldData        = 0x70,
    /// `ServiceIdentifier` (descriptor tag 0x71).
    ServiceIdentifier          = 0x71,
    /// `ServiceAvailability` (descriptor tag 0x72).
    ServiceAvailability        = 0x72,
    /// `DefaultAuthority` (descriptor tag 0x73).
    DefaultAuthority           = 0x73,
    /// `RelatedContent` (descriptor tag 0x74).
    RelatedContent             = 0x74,
    /// `TvaId` (descriptor tag 0x75).
    TvaId                      = 0x75,
    /// `ContentIdentifier` (descriptor tag 0x76).
    ContentIdentifier          = 0x76,
    /// `TimeSliceFecIdentifier` (descriptor tag 0x77).
    TimeSliceFecIdentifier     = 0x77,
    /// `EcmRepetitionRate` (descriptor tag 0x78).
    EcmRepetitionRate          = 0x78,
    /// `S2SatelliteDeliverySystem` (descriptor tag 0x79).
    S2SatelliteDeliverySystem  = 0x79,
    /// `EnhancedAc3` (descriptor tag 0x7A).
    EnhancedAc3                = 0x7A,
    /// `Dts` (descriptor tag 0x7B).
    Dts                        = 0x7B,
    /// `Aac` (descriptor tag 0x7C).
    Aac                        = 0x7C,
    /// `XaitLocation` (descriptor tag 0x7D).
    XaitLocation               = 0x7D,
    /// `FtaContentManagement` (descriptor tag 0x7E).
    FtaContentManagement       = 0x7E,
    /// `Extension` (descriptor tag 0x7F).
    Extension                  = 0x7F,
}
impl DescriptorTag {
    /// Human-readable spec name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::VideoStream => "video_stream_descriptor",
            Self::AudioStream => "audio_stream_descriptor",
            Self::Hierarchy => "hierarchy_descriptor",
            Self::Registration => "registration_descriptor",
            Self::DataStreamAlignment => "data_stream_alignment_descriptor",
            Self::TargetBackgroundGrid => "target_background_grid_descriptor",
            Self::VideoWindow => "video_window_descriptor",
            Self::ConditionalAccess => "CA_descriptor",
            Self::Iso639Language => "ISO_639_language_descriptor",
            Self::SystemClock => "system_clock_descriptor",
            Self::MultiplexBufferUtilization => "multiplex_buffer_utilization_descriptor",
            Self::Copyright => "copyright_descriptor",
            Self::MaximumBitrate => "maximum_bitrate_descriptor",
            Self::PrivateDataIndicator => "private_data_indicator_descriptor",
            Self::SmoothingBuffer => "smoothing_buffer_descriptor",
            Self::Std => "STD_descriptor",
            Self::Ibp => "IBP_descriptor",
            Self::CarouselIdentifier => "carousel_identifier_descriptor",
            Self::AssociationTag => "association_tag_descriptor",
            Self::DeferredAssociationTags => "deferred_association_tags_descriptor",
            Self::Mpeg4Video => "MPEG-4_video_descriptor",
            Self::Mpeg4Audio => "MPEG-4_audio_descriptor",
            Self::Iod => "IOD_descriptor",
            Self::Sl => "SL_descriptor",
            Self::Fmc => "FMC_descriptor",
            Self::ExternalEsId => "External_ES_ID_descriptor",
            Self::Muxcode => "Muxcode_descriptor",
            Self::FmxBufferSize => "FmxBufferSize_descriptor",
            Self::MultiplexBuffer => "MultiplexBuffer_descriptor",
            Self::ContentLabeling => "content_labeling_descriptor",
            Self::MetadataPointer => "metadata_pointer_descriptor",
            Self::Metadata => "metadata_descriptor",
            Self::MetadataStd => "Metadata_STD_descriptor",
            Self::AvcVideo => "AVC_video_descriptor",
            Self::AvcTimingAndHrd => "AVC_timing_and_HRD_descriptor",
            Self::Mpeg2AacAudio => "MPEG-2_AAC_audio_descriptor",
            Self::FlexMuxTiming => "FlexMuxTiming_descriptor",
            Self::Mpeg4Text => "MPEG-4_text_descriptor",
            Self::Mpeg4AudioExtension => "MPEG-4_audio_extension_descriptor",
            Self::AuxiliaryVideoStream => "Auxiliary_video_stream_descriptor",
            Self::SvcExtension => "SVC_extension_descriptor",
            Self::MvcExtension => "MVC_extension_descriptor",
            Self::J2kVideo => "J2K_video_descriptor",
            Self::MvcOperationPoint => "MVC_operation_point_descriptor",
            Self::Mpeg2StereoscopicVideoFormat => "MPEG2_stereoscopic_video_format_descriptor",
            Self::StereoscopicProgramInfo => "stereoscopic_program_info_descriptor",
            Self::StereoscopicVideoInfo => "stereoscopic_video_info_descriptor",
            Self::TransportProfile => "transport_profile_descriptor",
            Self::HevcVideo => "HEVC_video_descriptor",
            Self::VvcVideo => "VVC_video_descriptor",
            Self::EvcVideo => "EVC_video_descriptor",
            Self::Mpeg2Extension => "Extension_descriptor (ITU-T H.222.0 Table 2-45, tag 63)",
            Self::NetworkName => "network_name_descriptor",
            Self::ServiceList => "service_list_descriptor",
            Self::Stuffing => "stuffing_descriptor",
            Self::SatelliteDeliverySystem => "satellite_delivery_system_descriptor",
            Self::CableDeliverySystem => "cable_delivery_system_descriptor",
            Self::VbiData => "VBI_data_descriptor",
            Self::VbiTeletext => "VBI_teletext_descriptor",
            Self::BouquetName => "bouquet_name_descriptor",
            Self::Service => "service_descriptor",
            Self::CountryAvailability => "country_availability_descriptor",
            Self::Linkage => "linkage_descriptor",
            Self::NvodReference => "NVOD_reference_descriptor",
            Self::TimeShiftedService => "time_shifted_service_descriptor",
            Self::ShortEvent => "short_event_descriptor",
            Self::ExtendedEvent => "extended_event_descriptor",
            Self::TimeShiftedEvent => "time_shifted_event_descriptor",
            Self::Component => "component_descriptor",
            Self::Mosaic => "mosaic_descriptor",
            Self::StreamIdentifier => "stream_identifier_descriptor",
            Self::CaIdentifier => "CA_identifier_descriptor",
            Self::Content => "content_descriptor",
            Self::ParentalRating => "parental_rating_descriptor",
            Self::Teletext => "teletext_descriptor",
            Self::Telephone => "telephone_descriptor",
            Self::LocalTimeOffset => "local_time_offset_descriptor",
            Self::Subtitling => "subtitling_descriptor",
            Self::TerrestrialDeliverySystem => "terrestrial_delivery_system_descriptor",
            Self::MultilingualNetworkName => "multilingual_network_name_descriptor",
            Self::MultilingualBouquetName => "multilingual_bouquet_name_descriptor",
            Self::MultilingualServiceName => "multilingual_service_name_descriptor",
            Self::MultilingualComponent => "multilingual_component_descriptor",
            Self::PrivateDataSpecifier => "private_data_specifier_descriptor",
            Self::ServiceMove => "service_move_descriptor",
            Self::ShortSmoothingBuffer => "short_smoothing_buffer_descriptor",
            Self::FrequencyList => "frequency_list_descriptor",
            Self::PartialTransportStream => "partial_transport_stream_descriptor",
            Self::DataBroadcast => "data_broadcast_descriptor",
            Self::Scrambling => "scrambling_descriptor",
            Self::DataBroadcastId => "data_broadcast_id_descriptor",
            Self::TransportStream => "transport_stream_descriptor",
            Self::Dsng => "DSNG_descriptor",
            Self::Pdc => "PDC_descriptor",
            Self::Ac3 => "AC-3_descriptor",
            Self::AncillaryData => "ancillary_data_descriptor",
            Self::CellList => "cell_list_descriptor",
            Self::CellFrequencyLink => "cell_frequency_link_descriptor",
            Self::AnnouncementSupport => "announcement_support_descriptor",
            Self::ApplicationSignalling => "application_signalling_descriptor",
            Self::AdaptationFieldData => "adaptation_field_data_descriptor",
            Self::ServiceIdentifier => "service_identifier_descriptor",
            Self::ServiceAvailability => "service_availability_descriptor",
            Self::DefaultAuthority => "default_authority_descriptor",
            Self::RelatedContent => "related_content_descriptor",
            Self::TvaId => "TVA_id_descriptor",
            Self::ContentIdentifier => "content_identifier_descriptor",
            Self::TimeSliceFecIdentifier => "time_slice_fec_identifier_descriptor",
            Self::EcmRepetitionRate => "ECM_repetition_rate_descriptor",
            Self::S2SatelliteDeliverySystem => "S2_satellite_delivery_system_descriptor",
            Self::EnhancedAc3 => "enhanced_AC-3_descriptor",
            Self::Dts => "DTS_descriptor",
            Self::Aac => "AAC_descriptor",
            Self::XaitLocation => "XAIT_location_descriptor",
            Self::FtaContentManagement => "FTA_content_management_descriptor",
            Self::Extension => "extension_descriptor",
        }
    }
}

impl From<DescriptorTag> for u8 {
    fn from(t: DescriptorTag) -> Self {
        t as u8
    }
}

broadcast_common::impl_spec_display!(DescriptorTag);

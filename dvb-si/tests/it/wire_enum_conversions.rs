//! Every `TryFrom<u8>` spec enum round-trips each wire value through
//! `try_from` and back through `From<Self> for u8`, and rejects an
//! unallocated value with the crate's own typed error carrying that value.

use broadcast_common::error::InvalidValue;
use dvb_si::Error;
use dvb_si::descriptor_tag::DescriptorTag;
use dvb_si::table_id::TableId;
use dvb_si::tables::sat::SatTableId;

fn assert_rejected<T: core::fmt::Debug>(field: &'static str, value: u64, got: Result<T, Error>) {
    match got {
        Err(Error::InvalidValue(InvalidValue {
            field: f, value: v, ..
        })) => assert_eq!((f, v), (field, value)),
        other => panic!("expected InvalidValue for {field}, got {other:?}"),
    }
}

#[test]
fn table_id() {
    for (byte, v) in [
        (0x00u8, TableId::Pat),
        (0x01, TableId::Cat),
        (0x02, TableId::Pmt),
        (0x03, TableId::TransportStreamDescription),
        (0x04, TableId::Iso14496SceneDescription),
        (0x05, TableId::Iso14496ObjectDescriptor),
        (0x06, TableId::Metadata),
        (0x08, TableId::Iso14496),
        (0x09, TableId::GreenAccessUnit),
        (0x40, TableId::NetworkInformationActual),
        (0x41, TableId::NetworkInformationOther),
        (0x42, TableId::ServiceDescriptionActual),
        (0x46, TableId::ServiceDescriptionOther),
        (0x4A, TableId::BouquetAssociation),
        (0x4B, TableId::UpdateNotification),
        (0x4C, TableId::IpMacNotification),
        (0x4D, TableId::SatelliteAccess),
        (0x4E, TableId::EventInformationPfActual),
        (0x4F, TableId::EventInformationPfOther),
        (0x70, TableId::TimeAndDate),
        (0x71, TableId::RunningStatus),
        (0x72, TableId::Stuffing),
        (0x73, TableId::TimeOffset),
        (0x74, TableId::ApplicationInformation),
        (0x75, TableId::Container),
        (0x76, TableId::RelatedContent),
        (0x77, TableId::ContentIdentifier),
        (0x78, TableId::MpeFec),
        (0x79, TableId::ResolutionNotification),
        (0x7A, TableId::MpeIfec),
        (0x7B, TableId::ProtectionMessage),
        (0x7C, TableId::DownloadableFontInfo),
        (0x7E, TableId::DiscontinuityInformation),
        (0x7F, TableId::SelectionInformation),
    ] {
        assert_eq!(TableId::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    assert_rejected("table_id", 0x07, TableId::try_from(0x07));
}

#[test]
fn descriptor_tag() {
    for (byte, v) in [
        (0x02u8, DescriptorTag::VideoStream),
        (0x03, DescriptorTag::AudioStream),
        (0x04, DescriptorTag::Hierarchy),
        (0x05, DescriptorTag::Registration),
        (0x06, DescriptorTag::DataStreamAlignment),
        (0x07, DescriptorTag::TargetBackgroundGrid),
        (0x08, DescriptorTag::VideoWindow),
        (0x09, DescriptorTag::ConditionalAccess),
        (0x0A, DescriptorTag::Iso639Language),
        (0x0B, DescriptorTag::SystemClock),
        (0x0C, DescriptorTag::MultiplexBufferUtilization),
        (0x0D, DescriptorTag::Copyright),
        (0x0E, DescriptorTag::MaximumBitrate),
        (0x0F, DescriptorTag::PrivateDataIndicator),
        (0x10, DescriptorTag::SmoothingBuffer),
        (0x11, DescriptorTag::Std),
        (0x12, DescriptorTag::Ibp),
        (0x13, DescriptorTag::CarouselIdentifier),
        (0x14, DescriptorTag::AssociationTag),
        (0x15, DescriptorTag::DeferredAssociationTags),
        (0x1B, DescriptorTag::Mpeg4Video),
        (0x1C, DescriptorTag::Mpeg4Audio),
        (0x1D, DescriptorTag::Iod),
        (0x1E, DescriptorTag::Sl),
        (0x1F, DescriptorTag::Fmc),
        (0x20, DescriptorTag::ExternalEsId),
        (0x21, DescriptorTag::Muxcode),
        (0x22, DescriptorTag::FmxBufferSize),
        (0x23, DescriptorTag::MultiplexBuffer),
        (0x24, DescriptorTag::ContentLabeling),
        (0x25, DescriptorTag::MetadataPointer),
        (0x26, DescriptorTag::Metadata),
        (0x27, DescriptorTag::MetadataStd),
        (0x28, DescriptorTag::AvcVideo),
        (0x2A, DescriptorTag::AvcTimingAndHrd),
        (0x2B, DescriptorTag::Mpeg2AacAudio),
        (0x2C, DescriptorTag::FlexMuxTiming),
        (0x2D, DescriptorTag::Mpeg4Text),
        (0x2E, DescriptorTag::Mpeg4AudioExtension),
        (0x2F, DescriptorTag::AuxiliaryVideoStream),
        (0x30, DescriptorTag::SvcExtension),
        (0x31, DescriptorTag::MvcExtension),
        (0x32, DescriptorTag::J2kVideo),
        (0x33, DescriptorTag::MvcOperationPoint),
        (0x34, DescriptorTag::Mpeg2StereoscopicVideoFormat),
        (0x35, DescriptorTag::StereoscopicProgramInfo),
        (0x36, DescriptorTag::StereoscopicVideoInfo),
        (0x37, DescriptorTag::TransportProfile),
        (0x38, DescriptorTag::HevcVideo),
        (0x39, DescriptorTag::VvcVideo),
        (0x3A, DescriptorTag::EvcVideo),
        (0x3F, DescriptorTag::Mpeg2Extension),
        (0x40, DescriptorTag::NetworkName),
        (0x41, DescriptorTag::ServiceList),
        (0x42, DescriptorTag::Stuffing),
        (0x43, DescriptorTag::SatelliteDeliverySystem),
        (0x44, DescriptorTag::CableDeliverySystem),
        (0x45, DescriptorTag::VbiData),
        (0x46, DescriptorTag::VbiTeletext),
        (0x47, DescriptorTag::BouquetName),
        (0x48, DescriptorTag::Service),
        (0x49, DescriptorTag::CountryAvailability),
        (0x4A, DescriptorTag::Linkage),
        (0x4B, DescriptorTag::NvodReference),
        (0x4C, DescriptorTag::TimeShiftedService),
        (0x4D, DescriptorTag::ShortEvent),
        (0x4E, DescriptorTag::ExtendedEvent),
        (0x4F, DescriptorTag::TimeShiftedEvent),
        (0x50, DescriptorTag::Component),
        (0x51, DescriptorTag::Mosaic),
        (0x52, DescriptorTag::StreamIdentifier),
        (0x53, DescriptorTag::CaIdentifier),
        (0x54, DescriptorTag::Content),
        (0x55, DescriptorTag::ParentalRating),
        (0x56, DescriptorTag::Teletext),
        (0x57, DescriptorTag::Telephone),
        (0x58, DescriptorTag::LocalTimeOffset),
        (0x59, DescriptorTag::Subtitling),
        (0x5A, DescriptorTag::TerrestrialDeliverySystem),
        (0x5B, DescriptorTag::MultilingualNetworkName),
        (0x5C, DescriptorTag::MultilingualBouquetName),
        (0x5D, DescriptorTag::MultilingualServiceName),
        (0x5E, DescriptorTag::MultilingualComponent),
        (0x5F, DescriptorTag::PrivateDataSpecifier),
        (0x60, DescriptorTag::ServiceMove),
        (0x61, DescriptorTag::ShortSmoothingBuffer),
        (0x62, DescriptorTag::FrequencyList),
        (0x63, DescriptorTag::PartialTransportStream),
        (0x64, DescriptorTag::DataBroadcast),
        (0x65, DescriptorTag::Scrambling),
        (0x66, DescriptorTag::DataBroadcastId),
        (0x67, DescriptorTag::TransportStream),
        (0x68, DescriptorTag::Dsng),
        (0x69, DescriptorTag::Pdc),
        (0x6A, DescriptorTag::Ac3),
        (0x6B, DescriptorTag::AncillaryData),
        (0x6C, DescriptorTag::CellList),
        (0x6D, DescriptorTag::CellFrequencyLink),
        (0x6E, DescriptorTag::AnnouncementSupport),
        (0x6F, DescriptorTag::ApplicationSignalling),
        (0x70, DescriptorTag::AdaptationFieldData),
        (0x71, DescriptorTag::ServiceIdentifier),
        (0x72, DescriptorTag::ServiceAvailability),
        (0x73, DescriptorTag::DefaultAuthority),
        (0x74, DescriptorTag::RelatedContent),
        (0x75, DescriptorTag::TvaId),
        (0x76, DescriptorTag::ContentIdentifier),
        (0x77, DescriptorTag::TimeSliceFecIdentifier),
        (0x78, DescriptorTag::EcmRepetitionRate),
        (0x79, DescriptorTag::S2SatelliteDeliverySystem),
        (0x7A, DescriptorTag::EnhancedAc3),
        (0x7B, DescriptorTag::Dts),
        (0x7C, DescriptorTag::Aac),
        (0x7D, DescriptorTag::XaitLocation),
        (0x7E, DescriptorTag::FtaContentManagement),
        (0x7F, DescriptorTag::Extension),
    ] {
        assert_eq!(DescriptorTag::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    assert_rejected("descriptor_tag", 0x00, DescriptorTag::try_from(0x00));
}

#[test]
fn sat_table_id() {
    for (byte, v) in [
        (0u8, SatTableId::PositionV2),
        (1, SatTableId::CellFragment),
        (2, SatTableId::TimeAssociation),
        (3, SatTableId::BeamhoppingTimePlan),
        (4, SatTableId::PositionV3),
    ] {
        assert_eq!(SatTableId::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    assert_rejected("satellite_table_id", 5, SatTableId::try_from(5));
}

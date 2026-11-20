//! Spec-token label spelling (issue #2277).

#[test]
fn table_id_and_descriptor_tag_labels_match_spec() {
    use dvb_si::DescriptorTag;
    use dvb_si::TableId;
    use dvb_si::descriptors::extension::ExtensionTag;

    assert_eq!(TableId::Pat.name(), "program_association_section");
    assert_eq!(
        TableId::NetworkInformationActual.name(),
        "network_information_section - actual network"
    );
    assert_eq!(DescriptorTag::NetworkName.name(), "network_name_descriptor");
    assert_eq!(ExtensionTag::ImageIcon.name(), "image_icon_descriptor");
}

macro_rules! check_label {
    ($variant:expr, $label:literal) => {
        assert_eq!($variant.name(), $label);
        assert_eq!($variant.to_string(), $label);
    };
    ($variant:expr, $label:literal, $display:literal) => {
        assert_eq!($variant.name(), $label);
        assert_eq!($variant.to_string(), $display);
    };
}

#[test]
fn linkage_labels_match_the_spec_tables() {
    // EN 300 468 Tables 60, 62, 66 and 67.
    use dvb_si::descriptors::linkage::{HandOverType, LinkType, LinkageType, TargetIdType};

    check_label!(LinkageType::InformationService, "information service");
    check_label!(LinkageType::EpgService, "EPG service");
    check_label!(LinkageType::CaReplacementService, "CA replacement service");
    check_label!(
        LinkageType::TsContainingCompleteSi,
        "TS containing complete network/bouquet SI"
    );
    check_label!(
        LinkageType::ServiceReplacementService,
        "service replacement service"
    );
    check_label!(LinkageType::DataBroadcastService, "data broadcast service");
    check_label!(LinkageType::RcsMap, "RCS map");
    check_label!(LinkageType::MobileHandOver, "mobile hand-over");
    check_label!(LinkageType::SsuService, "SSU service");
    check_label!(
        LinkageType::TsContainingSsuBatOrNit,
        "TS containing SSU BAT or NIT"
    );
    check_label!(
        LinkageType::IpMacNotificationService,
        "IP/MAC notification service"
    );
    check_label!(
        LinkageType::TsContainingIntBatOrNit,
        "TS containing INT BAT or NIT"
    );
    check_label!(LinkageType::EventLinkage, "event linkage");
    check_label!(
        LinkageType::ExtendedEventLinkage(0x0E),
        "extended event linkage",
        "extended event linkage(0x0E)"
    );
    check_label!(
        LinkageType::DownloadableFontInfoLinkage,
        "downloadable font info linkage"
    );
    check_label!(
        LinkageType::NativeIpBootstrapMpeStream,
        "Native IP bootstrap MPE stream"
    );
    check_label!(LinkageType::Reserved(0x22), "reserved", "reserved(0x22)");

    check_label!(
        HandOverType::DvbIdenticalNeighbouringCountry,
        "DVB hand-over to an identical service in a neighbouring country"
    );
    check_label!(
        HandOverType::DvbLocalVariation,
        "DVB hand-over to a local variation of the same service"
    );
    check_label!(
        HandOverType::DvbAssociatedService,
        "DVB hand-over to an associated service"
    );
    check_label!(HandOverType::Reserved(0x4), "reserved", "reserved(0x04)");

    check_label!(LinkType::SdOrUhd, "SD (0x0E) / UHD (0x0F)");
    check_label!(
        LinkType::HdOrServiceFrameCompatible,
        "HD (0x0E) / service frame compatible plano-stereoscopic (0x0F)"
    );
    check_label!(
        LinkType::FrameCompatiblePlanoStereoscopic,
        "frame compatible plano-stereoscopic H.264/AVC"
    );
    check_label!(
        LinkType::ServiceCompatiblePlanoStereoscopicMvc,
        "service compatible plano-stereoscopic MVC"
    );
    check_label!(LinkType::Reserved(0x4), "reserved", "reserved(0x04)");

    check_label!(
        TargetIdType::UseTransportStreamId,
        "use transport_stream_id"
    );
    check_label!(
        TargetIdType::UseTargetTransportStreamId,
        "use target_transport_stream_id"
    );
    check_label!(
        TargetIdType::MatchAnyTransportStreamId,
        "match any transport_stream_id (wildcard)"
    );
    check_label!(TargetIdType::UseUserDefinedId, "use user_defined_id");
    check_label!(TargetIdType::Reserved(0x4), "reserved", "reserved(0x04)");
}

#[test]
fn cable_labels_match_the_spec_tables() {
    // EN 300 468 Tables 34, 35 and 36.
    use dvb_si::descriptors::cable_delivery_system::{FecInner, FecOuter, Modulation};

    check_label!(FecOuter::NotDefined, "not defined");
    check_label!(FecOuter::NoOuterFec, "no outer FEC coding");
    check_label!(FecOuter::ReedSolomon204_188, "Reed-Solomon (204, 188)");
    check_label!(FecOuter::Reserved(0x3), "reserved", "reserved(0x03)");

    check_label!(Modulation::NotDefined, "not defined");
    check_label!(Modulation::Qam16, "16-QAM");
    check_label!(Modulation::Qam32, "32-QAM");
    check_label!(Modulation::Qam64, "64-QAM");
    check_label!(Modulation::Qam128, "128-QAM");
    check_label!(Modulation::Qam256, "256-QAM");
    check_label!(Modulation::Reserved(0x6), "reserved", "reserved(0x06)");

    check_label!(FecInner::NotDefined, "not defined");
    check_label!(FecInner::Rate1_2, "1/2");
    check_label!(FecInner::Rate2_3, "2/3");
    check_label!(FecInner::Rate3_4, "3/4");
    check_label!(FecInner::Rate5_6, "5/6");
    check_label!(FecInner::Rate7_8, "7/8");
    check_label!(FecInner::Rate8_9, "8/9");
    check_label!(FecInner::Rate3_5, "3/5");
    check_label!(FecInner::Rate4_5, "4/5");
    check_label!(FecInner::Rate9_10, "9/10");
    check_label!(FecInner::NoConvCoding, "no convolutional coding");
    check_label!(FecInner::Reserved(0xA), "reserved", "reserved(0x0A)");
}

#[test]
fn satellite_labels_match_the_spec_tables() {
    // EN 300 468 Tables 38, 40, 41, 39 and 43.
    use dvb_si::descriptors::s2_satellite_delivery_system::TsGsMode;
    use dvb_si::descriptors::satellite_delivery_system::{
        ModulationSystem, ModulationType, Polarization, RollOff,
    };

    check_label!(Polarization::LinearHorizontal, "linear horizontal");
    check_label!(Polarization::LinearVertical, "linear vertical");
    check_label!(Polarization::CircularLeft, "circular left");
    check_label!(Polarization::CircularRight, "circular right");

    check_label!(ModulationSystem::DvbS, "DVB-S");
    check_label!(ModulationSystem::DvbS2, "DVB-S2");

    check_label!(ModulationType::Auto, "Auto");
    check_label!(ModulationType::Qpsk, "QPSK");
    check_label!(ModulationType::Psk8, "8PSK");
    check_label!(ModulationType::Qam16, "16-QAM");

    check_label!(RollOff::Alpha035, "α=0.35");
    check_label!(RollOff::Alpha025, "α=0.25");
    check_label!(RollOff::Alpha020, "α=0.20");
    check_label!(RollOff::Reserved(0x3), "reserved", "reserved(0x03)");

    check_label!(TsGsMode::GenericPacketized, "Generic Packetized");
    check_label!(TsGsMode::Gse, "Generic Stream Encapsulation (GSE)");
    check_label!(TsGsMode::DvbTransportStream, "DVB transport stream");
    check_label!(TsGsMode::Reserved(0x3), "reserved", "reserved(0x03)");
}

#[test]
fn terrestrial_labels_match_the_spec_tables() {
    // EN 300 468 Tables 45, 47, 48, 49, 50 and 51.
    use dvb_si::descriptors::terrestrial_delivery_system::{
        Bandwidth, CodeRate, Constellation, GuardInterval, Hierarchy, TransmissionMode,
    };

    check_label!(Bandwidth::Mhz8, "8 MHz");
    check_label!(Bandwidth::Mhz7, "7 MHz");
    check_label!(Bandwidth::Mhz6, "6 MHz");
    check_label!(Bandwidth::Mhz5, "5 MHz");
    check_label!(Bandwidth::Reserved(0x4), "reserved", "reserved(0x04)");

    check_label!(Constellation::Qpsk, "QPSK");
    check_label!(Constellation::Qam16, "16-QAM");
    check_label!(Constellation::Qam64, "64-QAM");
    check_label!(Constellation::Reserved(0x3), "reserved", "reserved(0x03)");

    check_label!(
        Hierarchy::NonHierarchicalNative,
        "non-hierarchical, native interleaver"
    );
    check_label!(Hierarchy::Alpha1Native, "α=1, native interleaver");
    check_label!(Hierarchy::Alpha2Native, "α=2, native interleaver");
    check_label!(Hierarchy::Alpha4Native, "α=4, native interleaver");
    check_label!(
        Hierarchy::NonHierarchicalInDepth,
        "non-hierarchical, in-depth interleaver"
    );
    check_label!(Hierarchy::Alpha1InDepth, "α=1, in-depth interleaver");
    check_label!(Hierarchy::Alpha2InDepth, "α=2, in-depth interleaver");
    check_label!(Hierarchy::Alpha4InDepth, "α=4, in-depth interleaver");
    check_label!(Hierarchy::Reserved(0x8), "reserved", "reserved(0x08)");

    check_label!(CodeRate::Rate1_2, "1/2");
    check_label!(CodeRate::Rate2_3, "2/3");
    check_label!(CodeRate::Rate3_4, "3/4");
    check_label!(CodeRate::Rate5_6, "5/6");
    check_label!(CodeRate::Rate7_8, "7/8");
    check_label!(CodeRate::Reserved(0x5), "reserved", "reserved(0x05)");

    check_label!(GuardInterval::G1_32, "1/32");
    check_label!(GuardInterval::G1_16, "1/16");
    check_label!(GuardInterval::G1_8, "1/8");
    check_label!(GuardInterval::G1_4, "1/4");

    check_label!(TransmissionMode::Mode2k, "2k mode");
    check_label!(TransmissionMode::Mode8k, "8k mode");
    check_label!(TransmissionMode::Mode4k, "4k mode");
    check_label!(
        TransmissionMode::Reserved(0x3),
        "reserved",
        "reserved(0x03)"
    );
}

#[test]
fn aac_component_type_labels_match_table_26() {
    // EN 300 468 Table 26, stream_content 0x06 (HE-AAC) rows.
    use dvb_si::descriptors::aac::AacExtension;

    let cases = [
        (0x01, "HE-AAC audio, single mono channel"),
        (0x03, "HE-AAC audio, stereo"),
        (0x05, "HE-AAC audio, surround sound"),
        (0x40, "HE-AAC audio description for the visually impaired"),
        (0x41, "HE-AAC audio for the hard of hearing"),
        (0x42, "HE-AAC receiver-mix supplementary audio"),
        (0x43, "HE-AAC v2 audio, stereo"),
        (
            0x44,
            "HE-AAC v2 audio description for the visually impaired",
        ),
        (0x45, "HE-AAC v2 audio for the hard of hearing"),
        (0x46, "HE-AAC v2 receiver-mix supplementary audio"),
        (
            0x47,
            "HE-AAC receiver-mix audio description for the visually impaired",
        ),
        (
            0x48,
            "HE-AAC receiver-mix audio description for the visually impaired",
        ),
        (
            0x49,
            "HE-AAC v2 receiver-mix audio description for the visually impaired",
        ),
        (
            0x4A,
            "HE-AAC v2 receiver-mix audio description for the visually impaired",
        ),
        (0xA0, "HE-AAC or HE-AAC v2 with SAOC-DE ancillary data"),
    ];
    for (code, label) in cases {
        let ty = AacExtension::new(false, Some(code), &[])
            .decoded_component_type()
            .unwrap();
        assert_eq!(ty.name(), label, "aac_type = 0x{code:02X}");
    }
    assert_eq!(
        AacExtension::new(false, Some(0xFF), &[]).decoded_component_type(),
        None
    );
}

#[test]
fn announcement_and_frequency_labels_match_the_spec_tables() {
    // EN 300 468 Tables 19, 20 and 31.
    use dvb_si::descriptors::announcement_support::{AnnouncementType, ReferenceType};
    use dvb_si::descriptors::frequency_list::CodingType;

    check_label!(AnnouncementType::EmergencyAlarm, "Emergency alarm");
    check_label!(AnnouncementType::RoadTrafficFlash, "Road Traffic flash");
    check_label!(
        AnnouncementType::PublicTransportFlash,
        "Public Transport flash"
    );
    check_label!(AnnouncementType::WarningMessage, "Warning message");
    check_label!(AnnouncementType::NewsFlash, "News flash");
    check_label!(AnnouncementType::WeatherFlash, "Weather flash");
    check_label!(AnnouncementType::EventAnnouncement, "Event announcement");
    check_label!(AnnouncementType::PersonalCall, "Personal call");
    check_label!(AnnouncementType::Reserved(0x0F), "reserved", "reserved(0x0F)");

    check_label!(
        ReferenceType::UsualAudioStream,
        "broadcast in the usual audio stream of the service"
    );
    check_label!(
        ReferenceType::SeparateAudioStream,
        "broadcast in a separate audio stream that is part of the service"
    );
    check_label!(
        ReferenceType::DifferentServiceSameTs,
        "broadcast by a different service within the same DVB transport stream"
    );
    check_label!(
        ReferenceType::DifferentServiceDifferentTs,
        "broadcast by a different service within a different DVB transport stream"
    );
    check_label!(ReferenceType::Reserved(0x05), "reserved", "reserved(0x05)");

    check_label!(CodingType::Undefined, "not defined");
    check_label!(CodingType::Satellite, "satellite");
    check_label!(CodingType::Cable, "cable");
    check_label!(CodingType::Terrestrial, "terrestrial");
}

#[test]
fn content_time_base_and_fta_labels_match_the_spec_tables() {
    // ISO/IEC 13818-1 Table 2-85 and EN 300 468 Table 58.
    use dvb_si::descriptors::content_time_base_indicator::ContentTimeBaseIndicator;
    use dvb_si::descriptors::fta_content_management::ControlRemoteAccess;

    check_label!(ContentTimeBaseIndicator::None, "none");
    check_label!(ContentTimeBaseIndicator::Stc, "STC");
    check_label!(ContentTimeBaseIndicator::Npt, "NPT");
    check_label!(
        ContentTimeBaseIndicator::Reserved(0x03),
        "reserved",
        "reserved(0x03)"
    );
    check_label!(
        ContentTimeBaseIndicator::Private(0x08),
        "private",
        "private(0x08)"
    );

    check_label!(
        ControlRemoteAccess::Enabled,
        "redistribution enabled"
    );
    check_label!(
        ControlRemoteAccess::EnabledManagedDomain,
        "redistribution enabled (managed domain only)"
    );
    check_label!(
        ControlRemoteAccess::EnabledManagedDomainTimeLimited,
        "redistribution enabled (managed domain, time-limited)"
    );
    check_label!(
        ControlRemoteAccess::NotAllowed,
        "redistribution not allowed"
    );
    check_label!(
        ControlRemoteAccess::Reserved(0x04),
        "reserved",
        "reserved(0x04)"
    );
}

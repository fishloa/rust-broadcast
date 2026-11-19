//! The #204 `name()` labels of the table enums, pinned to the spec tokens so a
//! blank or placeholder label fails the suite (`Display` is generated from
//! `name()`, and `dvb-tools dump` prints it).

#[test]
fn application_type_labels() {
    use dvb_si::tables::ait::ApplicationType;

    assert_eq!(ApplicationType::DvbJ.name(), "DVB-J");
    assert_eq!(ApplicationType::DvbHtml.to_string(), "DVB-HTML");
    assert_eq!(ApplicationType::HbbTv.name(), "HbbTV");
    assert_eq!(ApplicationType::OipfDae.name(), "OIPF DAE");
    assert_eq!(ApplicationType::Reserved(0x1234).name(), "Reserved");
    assert_eq!(
        ApplicationType::Reserved(0x1234).to_string(),
        "Reserved(0x1234)"
    );
}

#[test]
fn section_fragment_indication_labels() {
    use dvb_si::tables::metadata_section::SectionFragmentIndication;

    assert_eq!(
        SectionFragmentIndication::Complete.name(),
        "complete metadata Access Unit"
    );
    assert_eq!(SectionFragmentIndication::First.name(), "first section");
    assert_eq!(SectionFragmentIndication::Last.to_string(), "last section");
    assert_eq!(
        SectionFragmentIndication::Middle.name(),
        "neither first nor last section"
    );
}

#[test]
fn reference_type_labels() {
    use dvb_si::tables::protection_message::ReferenceType;

    assert_eq!(ReferenceType::Reserved.name(), "Reserved");
    assert_eq!(ReferenceType::SameEs.to_string(), "Same ES");
    assert_eq!(ReferenceType::ComponentTagEs.name(), "Component Tag ES");
    assert_eq!(ReferenceType::Unallocated(3).name(), "Unallocated");
    assert_eq!(
        ReferenceType::Unallocated(3).to_string(),
        "Unallocated(0x03)"
    );
}

#[test]
fn link_type_labels() {
    use dvb_si::tables::rct::LinkType;

    assert_eq!(LinkType::UriString.name(), "URI String");
    assert_eq!(LinkType::BinaryLocator.to_string(), "Binary Locator");
    assert_eq!(LinkType::Both.name(), "Both");
    assert_eq!(LinkType::Descriptor.name(), "Descriptor");
    assert_eq!(LinkType::DvbReserved(5).name(), "DVB Reserved");
    assert_eq!(LinkType::DvbReserved(5).to_string(), "DVB Reserved(0x05)");
}

#[test]
fn how_related_labels() {
    use dvb_si::tables::rct::HowRelated;

    assert_eq!(HowRelated::Cs2004.name(), "HowRelatedCS:2004");
    assert_eq!(HowRelated::Cs2005.to_string(), "HowRelatedCS:2005");
    assert_eq!(HowRelated::Cs2007.name(), "HowRelatedCS:2007");
    assert_eq!(HowRelated::DvbReserved(3).name(), "DVB Reserved");
    assert_eq!(
        HowRelated::UserPrivate(0x30).to_string(),
        "User Private(0x30)"
    );
}

#[test]
fn crid_authority_policy_labels() {
    use dvb_si::tables::rnt::CridAuthorityPolicy;

    assert_eq!(CridAuthorityPolicy::Permanent.name(), "Permanent");
    assert_eq!(CridAuthorityPolicy::Transient.to_string(), "Transient");
    assert_eq!(CridAuthorityPolicy::Either.name(), "Either");
    assert_eq!(CridAuthorityPolicy::Reserved.name(), "Reserved");
}

#[test]
fn context_id_type_labels() {
    use dvb_si::tables::rnt::ContextIdType;

    assert_eq!(ContextIdType::BouquetId.name(), "Bouquet ID");
    assert_eq!(
        ContextIdType::OriginalNetworkId.to_string(),
        "Original Network ID"
    );
    assert_eq!(ContextIdType::NetworkId.name(), "Network ID");
    assert_eq!(ContextIdType::DvbReserved(3).name(), "DVB Reserved");
    assert_eq!(
        ContextIdType::UserDefined(0x80).to_string(),
        "User Defined(0x80)"
    );
}

#[test]
fn unt_action_type_labels() {
    use dvb_si::tables::unt::UntActionType;

    assert_eq!(UntActionType::Reserved.name(), "Reserved");
    assert_eq!(
        UntActionType::SystemSoftwareUpdate.to_string(),
        "System Software Update"
    );
    assert_eq!(UntActionType::DvbReserved(2).name(), "DVB Reserved");
    assert_eq!(
        UntActionType::UserDefined(0x80).to_string(),
        "User Defined(0x80)"
    );
}

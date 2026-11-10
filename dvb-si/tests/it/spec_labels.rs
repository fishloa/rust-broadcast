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

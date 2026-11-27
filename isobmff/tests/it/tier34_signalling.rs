//! Family 4 of issue #4167: the newer audio and colour signalling.
//!
//! No installed tool writes these boxes, so each byte sequence is assembled
//! from the clause it names. The reader methods that surface the visual ones
//! run over an assembled file.

use broadcast_core::{Parse, Serialize};
use isobmff::audio_boxes::LabelBox;
use isobmff::audio_elements::{
    AudioElementBox, AudioElementChild, AudioElementDescriptionBox,
    AudioElementPosInteractivityPolarBox, AudioElementProminenceInteractivityBox,
    AudioElementSelectionBox, AudioElementSelectionDescriptionBox, AudioRenderingIndication,
    AudioRenderingIndicationBox,
};
use isobmff::index_boxes::{
    AlternativeStartupEntry, AlternativeStartupSequencePropertiesBox, TrackExtensionPropertiesBox,
};
use isobmff::visual_ext::{AmbientViewingEnvironmentBox, ContentColourVolumeBox};

use crate::tier34_schemes::{boxed, drive, file_with_sample_entry, full};

fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

#[test]
fn audio_rendering_indication_labels_and_round_trips() {
    for (code, label) in [
        (0u8, "no-preference"),
        (1, "stereo"),
        (2, "two-dimensional"),
        (3, "three-dimensional"),
        (4, "headphones"),
        (5, "reserved"),
    ] {
        let bytes = boxed(*b"ardi", &full(0, 0, &[code]));
        let parsed = AudioRenderingIndicationBox::parse(&bytes).expect("ardi");
        assert_eq!(parsed.indication().code(), code);
        assert_eq!(parsed.indication().name(), label);
        let display = parsed.indication().to_string();
        assert!(display.starts_with(label));
        assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
    }
    assert_eq!(
        AudioRenderingIndicationBox::new(AudioRenderingIndication::Stereo)
            .indication()
            .name(),
        "stereo"
    );
}

#[test]
fn ambient_viewing_environment_round_trips() {
    let body = [0u32.to_be_bytes(), [0x01, 0xF4, 0x00, 0x32]].concat();
    let amve = AmbientViewingEnvironmentBox::parse(&body).expect("amve");
    assert_eq!(amve.ambient_illuminance(), 0);
    assert_eq!(amve.ambient_light_x(), 500);
    assert_eq!(amve.ambient_light_y(), 50);
    let mut out = vec![0u8; amve.serialized_len()];
    let written = amve.serialize_into(&mut out).expect("serialize");
    assert_eq!(&out[..written], &body);
}

#[test]
fn content_colour_volume_round_trips_all_presence_forms() {
    let none = boxed(*b"cclv", &[0]);
    let parsed = ContentColourVolumeBox::parse(&none[8..]).expect("cclv");
    assert_eq!(parsed.flags(), 0);
    assert_eq!(parsed.primaries(), None);
    assert_eq!(parsed.min_luminance_value(), None);
    let mut out = vec![0u8; parsed.serialized_len()];
    let written = parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(&out[..written], &none[8..]);

    let primaries = [(1i32, 2i32), (3, 4), (5, 6)];
    let full = ContentColourVolumeBox::new()
        .with_primaries(primaries)
        .with_min_luminance(7)
        .with_max_luminance(8)
        .with_avg_luminance(9);
    let mut body = vec![full.flags()];
    for (x, y) in primaries {
        body.extend_from_slice(&x.to_be_bytes());
        body.extend_from_slice(&y.to_be_bytes());
    }
    body.extend_from_slice(&u32s(&[7, 8, 9]));
    assert_eq!(full.flags() & 0x04, 0x04);
    let parsed = ContentColourVolumeBox::parse(&body).expect("cclv full");
    assert_eq!(parsed.primaries(), Some(primaries));
    assert_eq!(parsed.min_luminance_value(), Some(7));
    assert_eq!(parsed.max_luminance_value(), Some(8));
    assert_eq!(parsed.avg_luminance_value(), Some(9));
    let mut out = vec![0u8; parsed.serialized_len()];
    let written = parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(&out[..written], &body);

    let only_avg = ContentColourVolumeBox::new().with_avg_luminance(4);
    let mut out = vec![0u8; only_avg.serialized_len()];
    let written = only_avg.serialize_into(&mut out).expect("serialize");
    assert_eq!(&out[..written], &[0x04, 0, 0, 0, 4]);
}

#[test]
fn the_reader_surfaces_the_visual_signalling() {
    let amve = boxed(*b"amve", &[0, 0, 0, 0, 0x01, 0xF4, 0x00, 0x32]);
    let cclv = boxed(*b"cclv", &[0x04, 0, 0, 0, 9]);
    let tracks = drive(&file_with_sample_entry(
        *b"resv",
        78,
        &[amve, cclv].concat(),
    ));
    let entry = &tracks[0].entries[0];
    let ambient = entry.ambient_viewing_environment().expect("amve");
    assert_eq!(ambient.ambient_light_x(), 500);
    let volume = entry.content_colour_volume().expect("cclv");
    assert_eq!(volume.avg_luminance_value(), Some(9));
}

#[test]
fn audio_element_positioning_round_trips_each_present_field() {
    let full = AudioElementPosInteractivityPolarBox::new()
        .with_azimuth([-30, 30, 0])
        .with_elevation([-10, 10, 0])
        .with_distance([100, 200, 150]);
    let mut out = vec![0u8; full.serialized_len()];
    let written = full.serialize_into(&mut out).expect("serialize");
    let parsed = AudioElementPosInteractivityPolarBox::parse(&out[..written]).expect("aepp");
    assert_eq!(parsed.azimuth(), Some([-30, 30, 0]));
    assert_eq!(parsed.elevation(), Some([-10, 10, 0]));
    assert_eq!(parsed.distance(), Some([100, 200, 150]));
    assert_eq!(
        parsed.try_to_bytes().expect("bytes"),
        out[..written].to_vec()
    );

    let azimuth_only = AudioElementPosInteractivityPolarBox::new().with_azimuth([1, 2, 3]);
    let mut out = vec![0u8; azimuth_only.serialized_len()];
    let written = azimuth_only.serialize_into(&mut out).expect("serialize");
    let parsed = AudioElementPosInteractivityPolarBox::parse(&out[..written]).expect("aepp");
    assert_eq!(parsed.azimuth(), Some([1, 2, 3]));
    assert_eq!(parsed.elevation(), None);
    assert_eq!(parsed.distance(), None);
}

#[test]
fn audio_element_prominence_keeps_its_words() {
    let values = [0x41A0_0000u32, 0x41C8_0000, 0x41B0_0000];
    let bytes = boxed(*b"aepr", &full(0, 0, &u32s(&values)));
    let parsed = AudioElementProminenceInteractivityBox::parse(&bytes).expect("aepr");
    assert_eq!(parsed.values(), values);
    assert_eq!(parsed.min_prominence(), 20.0);
    assert_eq!(parsed.max_prominence(), 25.0);
    assert_eq!(parsed.default_prominence(), 22.0);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn audio_element_selection_nests_its_description_and_elements() {
    let aedb = AudioElementDescriptionBox::new("dialog")
        .with_toggleable(true)
        .with_default_enabled(true)
        .with_child(AudioElementChild::Positioning(
            AudioElementPosInteractivityPolarBox::new().with_azimuth([0, 90, 45]),
        ));
    let aelm = AudioElementBox::new()
        .with_child(AudioElementChild::Description(aedb))
        .with_child(AudioElementChild::Label(
            LabelBox::new("en-US", "Dialog").with_label_id(1),
        ));
    let aesd = AudioElementSelectionDescriptionBox::new("language-selection");
    let aesb = AudioElementSelectionBox::new()
        .with_child(AudioElementChild::SelectionDescription(aesd))
        .with_child(AudioElementChild::Element(aelm));

    let mut out = vec![0u8; aesb.serialized_len()];
    let written = aesb.serialize_into(&mut out).expect("serialize");
    let parsed = AudioElementSelectionBox::parse(&out[..written]).expect("aesb");
    assert_eq!(
        parsed
            .selection_description()
            .and_then(|d| d.selection_tag_str()),
        Some("language-selection")
    );
    let elements = parsed.elements();
    assert_eq!(elements.len(), 1);
    let description = elements[0].description().expect("aedb");
    assert_eq!(description.audio_element_tag_str(), Some("dialog"));
    assert!(description.is_toggleable());
    assert!(description.is_default_enabled());
    assert_eq!(
        description.positioning().and_then(|p| p.azimuth()),
        Some([0, 90, 45])
    );
    assert_eq!(elements[0].labels().len(), 1);
    assert_eq!(
        parsed.try_to_bytes().expect("bytes"),
        out[..written].to_vec()
    );
}

#[test]
fn alternative_startup_sequence_properties_round_trip_both_versions() {
    let v0 = AlternativeStartupSequencePropertiesBox::new(-7);
    let mut out = vec![0u8; v0.serialized_len()];
    let written = v0.serialize_into(&mut out).expect("serialize");
    let parsed = AlternativeStartupSequencePropertiesBox::parse(&out[..written]).expect("assp v0");
    assert_eq!(parsed.version(), 0);
    assert_eq!(parsed.min_initial_alt_startup_offset(), Some(-7));
    assert_eq!(
        parsed.try_to_bytes().expect("bytes"),
        out[..written].to_vec()
    );

    let v1 = AlternativeStartupSequencePropertiesBox::per_grouping(vec![
        AlternativeStartupEntry::new(1, -3),
        AlternativeStartupEntry::new(2, 4),
    ]);
    let mut out = vec![0u8; v1.serialized_len()];
    let written = v1.serialize_into(&mut out).expect("serialize");
    let parsed = AlternativeStartupSequencePropertiesBox::parse(&out[..written]).expect("assp v1");
    assert_eq!(parsed.version(), 1);
    assert_eq!(parsed.entries().len(), 2);
    assert_eq!(parsed.entries()[1].min_initial_alt_startup_offset(), 4);
    assert_eq!(
        parsed.try_to_bytes().expect("bytes"),
        out[..written].to_vec()
    );

    let unsupported = boxed(*b"assp", &full(2, 0, &[]));
    assert!(AlternativeStartupSequencePropertiesBox::parse(&unsupported).is_err());
}

#[test]
fn a_track_extension_properties_box_keeps_its_startup_properties_in_place() {
    let child = boxed(*b"zzzz", &[1, 2, 3]);
    let assp = AlternativeStartupSequencePropertiesBox::new(-2);
    let mut assp_bytes = vec![0u8; assp.serialized_len()];
    let written = assp.serialize_into(&mut assp_bytes).expect("serialize");
    let mut body = vec![0u8, 0, 0, 0];
    body.extend_from_slice(&5u32.to_be_bytes());
    body.extend_from_slice(&child);
    body.extend_from_slice(&assp_bytes[..written]);
    let bytes = boxed(*b"trep", &body);
    let parsed = TrackExtensionPropertiesBox::parse(&bytes).expect("trep");
    assert_eq!(parsed.children().len(), 1);
    assert_eq!(
        parsed
            .alternative_startup()
            .and_then(|a| a.min_initial_alt_startup_offset()),
        Some(-2)
    );
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

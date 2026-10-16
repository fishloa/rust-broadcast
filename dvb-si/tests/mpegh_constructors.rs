//! The MPEG-H 3D audio sub-structs are `#[non_exhaustive]`, so a downstream
//! crate cannot use struct literals: this integration test (an external crate
//! from `dvb-si`'s point of view) builds every one of them through its public
//! constructor, wraps them in an `Mpeg2ExtensionDescriptor`, serializes, and
//! parses the bytes back to an equal value.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
use dvb_si::descriptors::mpegh_3d_audio::*;

fn round_trip(body: Mpeg2ExtensionBody) -> Vec<u8> {
    let d = Mpeg2ExtensionDescriptor::new(body);
    let mut buf = vec![0u8; d.serialized_len()];
    let n = d.serialize_into(&mut buf).expect("serialize");
    buf.truncate(n);
    let back = Mpeg2ExtensionDescriptor::parse(&buf).expect("parse back");
    assert_eq!(back, d, "serialize -> parse must be an identity");
    let mut again = vec![0u8; back.serialized_len()];
    let m = back.serialize_into(&mut again).expect("re-serialize");
    assert_eq!(&again[..m], &buf[..], "byte-identical re-serialize");
    buf
}

#[test]
fn config_and_command_constructors() {
    let b = round_trip(Mpeg2ExtensionBody::MpegH3dAudioConfig(
        MpegH3dAudioConfig::new(vec![0x01, 0x02, 0x03]),
    ));
    assert_eq!(b, [0x3F, 0x04, 0x09, 0x01, 0x02, 0x03]);
    let b = round_trip(Mpeg2ExtensionBody::MpegH3dAudioCommand(
        MpegH3dAudioCommand::new(vec![0xC0, 0x01]),
    ));
    assert_eq!(b, [0x3F, 0x03, 0x0E, 0xC0, 0x01]);
}

#[test]
fn scene_with_switch_group_and_preset_constructors() {
    let mut group = SceneGroup::new(1, true, true, 2);
    group.position = Some(PositionInteractivity::new(1, 2, 3, 4, 5, 6));
    group.gain = Some(GainInteractivity::new(1, 2));
    group.content_language = Some(*b"eng");
    let switch = SwitchGroup::new(
        3,
        true,
        false,
        vec![SwitchGroupMember::new(1), SwitchGroupMember::new(2)],
        1,
    );
    let preset = GroupPreset::new(
        2,
        1,
        vec![
            PresetCondition::new(1, None),
            PresetCondition::new(
                2,
                Some(PresetConditionDetail::new(
                    true,
                    false,
                    Some(0x10),
                    Some(PresetPosition::new(1, 2, 3)),
                )),
            ),
        ],
    );
    let scene = MpegH3dAudioScene::new(7, Some(vec![group]), Some(vec![switch]), Some(vec![preset]));
    round_trip(Mpeg2ExtensionBody::MpegH3dAudioScene(scene));
}

#[test]
fn text_label_constructors() {
    let lang = TextLabelLanguage::new(
        *b"eng",
        vec![TextLabelEntry::group(1, b"Dialog".to_vec())],
        vec![TextLabelEntry::switch_group_or_preset(2, b"Lang".to_vec())],
        vec![TextLabelEntry::switch_group_or_preset(3, b"Preset".to_vec())],
    );
    round_trip(Mpeg2ExtensionBody::MpegH3dAudioTextLabel(
        MpegH3dAudioTextLabel::new(5, vec![lang]),
    ));
}

#[test]
fn multi_stream_constructors() {
    let main = MultiStreamMain::new(
        1,
        vec![
            MultiStreamGroup::in_main_stream(1),
            MultiStreamGroup::in_auxiliary_stream(2, AuxiliaryLocation::new(true, 9)),
        ],
    );
    round_trip(Mpeg2ExtensionBody::MpegH3dAudioMultiStream(
        MpegH3dAudioMultiStream::new(0, Some(main)),
    ));
    round_trip(Mpeg2ExtensionBody::MpegH3dAudioMultiStream(
        MpegH3dAudioMultiStream::new(4, None),
    ));
}

#[test]
fn drc_loudness_constructors() {
    let mut drc = DrcInstructions::new(DrcScope::group(2, 5), 1, 2, 0x0102);
    drc.limiter_peak_target = Some(0x20);
    drc.target_loudness = Some(DrcTargetLoudness::new(10, 0, 1));
    let plain = DrcInstructions::new(DrcScope::Plain(0), 2, 3, 0);
    let info = DrcLoudnessInfo::new(
        vec![drc, plain],
        vec![
            LoudnessInfoEntry::new(DrcScope::Plain(0), vec![0xAA, 0xBB]),
            LoudnessInfoEntry::new(DrcScope::preset(4), vec![0xCC]),
        ],
        vec![DownmixDefinition::new(1, 2, 3)],
    );
    round_trip(Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(
        MpegH3dAudioDrcLoudness::new(Some(info)),
    ));
    round_trip(Mpeg2ExtensionBody::MpegH3dAudioDrcLoudness(
        MpegH3dAudioDrcLoudness::new(None),
    ));
}

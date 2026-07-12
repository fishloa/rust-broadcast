//! Integration-level round-trip coverage across element boundaries:
//! multi-element frames, `Plex`-escalated `ElementSize`s, reserved/unknown
//! element pass-through, and (when the `serde` feature is enabled) serde
//! round-trips. Per-module unit round-trips already cover each element type
//! in isolation.

use broadcast_common::{Parse, Serialize};
use rdd29::{
    AnyElement, AtmosFrame, AudioDataDlc, AudioDescription, BedChannel, BedDefinition1, BitDepth,
    ChannelId, DecorCoefPrefix, FrameRate, ObjectDefinition1, ObjectSpreadMode, PanInfo,
    PanSubBlock, SampleRate,
};

fn full_pan_sub_blocks(frame_rate: FrameRate) -> Vec<PanSubBlock> {
    let n = usize::from(frame_rate.num_pan_sub_blocks().unwrap());
    let mut blocks = vec![PanSubBlock {
        pan: Some(PanInfo {
            pos_x: 0x1000,
            pos_y: 0x2000,
            pos_z: 0x3000,
            snap: true,
            zone_gains: None,
            spread_mode: ObjectSpreadMode::OneD,
            spread: 0xABC,
            decor_coef_prefix: DecorCoefPrefix::MaxDecorrelation,
            decor_coef: None,
        }),
    }];
    blocks.resize(n, PanSubBlock { pan: None });
    blocks
}

fn full_frame(frame_rate: FrameRate, large_payload_len: usize) -> AtmosFrame<'static> {
    let bed = BedDefinition1::new(
        1,
        vec![
            BedChannel {
                channel_id: ChannelId::LeftScreen,
                audio_data_id: 10,
            },
            BedChannel {
                channel_id: ChannelId::Lfe,
                audio_data_id: 11,
            },
        ],
    );
    let object = ObjectDefinition1::new(
        2,
        12,
        full_pan_sub_blocks(frame_rate),
        AudioDescription::with_text(b"explosion").unwrap(),
    )
    .unwrap();

    // A payload long enough to push ElementSize's Plex(8) past the direct
    // 8-bit range (>0xFE bytes), exercising Plex escalation across a real
    // element boundary, not just plex.rs's own unit tests.
    let large_payload: &'static [u8] =
        Box::leak(vec![0xA5u8; large_payload_len].into_boxed_slice());

    let left = AudioDataDlc::new(10, b"left").unwrap();
    let lfe = AudioDataDlc::new(11, b"lfe").unwrap();
    let object_audio = AudioDataDlc::new(12, large_payload).unwrap();

    AtmosFrame::new(
        SampleRate::Hz96000,
        BitDepth::Bits24,
        frame_rate,
        3,
        vec![
            AnyElement::BedDefinition1(bed),
            AnyElement::ObjectDefinition1(object),
            AnyElement::AudioDataDlc(left),
            AnyElement::AudioDataDlc(lfe),
            AnyElement::AudioDataDlc(object_audio),
            AnyElement::Unknown {
                element_id: 0x20, // Table 1's reserved code
                data: b"future extension bytes",
            },
        ],
    )
}

#[test]
fn full_frame_round_trips_with_plex_escalated_element_size() {
    let frame = full_frame(FrameRate::Fps48, 300); // > 0xFE forces Plex(8) -> 16-bit escalation
    let bytes = frame.to_bytes();
    let parsed = AtmosFrame::parse(&bytes).expect("parse");
    assert_eq!(parsed, frame);

    let bytes2 = parsed.to_bytes();
    assert_eq!(
        bytes, bytes2,
        "serialize(parse(serialize(frame))) is byte-identical"
    );
}

#[test]
fn full_frame_round_trips_at_every_frame_rate() {
    for fr in [
        FrameRate::Fps24,
        FrameRate::Fps25,
        FrameRate::Fps30,
        FrameRate::Fps48,
        FrameRate::Fps50,
        FrameRate::Fps60,
        FrameRate::Fps96,
        FrameRate::Fps100,
        FrameRate::Fps120,
    ] {
        let frame = full_frame(fr, 16);
        let bytes = frame.to_bytes();
        let parsed = AtmosFrame::parse(&bytes).unwrap_or_else(|e| panic!("frame_rate {fr:?}: {e}"));
        assert_eq!(parsed, frame, "frame_rate {fr:?}");
    }
}

#[test]
fn reserved_element_id_round_trips_verbatim() {
    let frame = full_frame(FrameRate::Fps24, 8);
    let bytes = frame.to_bytes();
    let parsed = AtmosFrame::parse(&bytes).unwrap();
    let last = parsed.elements.last().unwrap();
    match last {
        AnyElement::Unknown { element_id, data } => {
            assert_eq!(*element_id, 0x20);
            assert_eq!(*data, b"future extension bytes");
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[cfg(feature = "serde")]
#[test]
fn bed_definition_serde_round_trips() {
    let bed = BedDefinition1::new(
        3,
        vec![BedChannel {
            channel_id: ChannelId::Lfe,
            audio_data_id: 99,
        }],
    );
    let json = serde_json::to_string(&bed).unwrap();
    let back: BedDefinition1 = serde_json::from_str(&json).unwrap();
    assert_eq!(back, bed);
}

#[cfg(feature = "serde")]
#[test]
fn audio_data_dlc_serializes() {
    // AudioDataDlc holds a borrowed `&[u8]` payload: serde_json's default
    // array-of-numbers representation cannot round-trip back into a
    // borrowed byte slice (it needs the deserializer's "bytes" hint, which
    // a plain JSON array does not give), so -- like `st337::Burst` -- this
    // type derives `Serialize` only. Confirm it serializes to the expected
    // shape rather than round-tripping through `Deserialize`.
    let payload = [1u8, 2, 3, 4];
    let dlc = AudioDataDlc::new(5, &payload).unwrap();
    let json = serde_json::to_value(dlc).unwrap();
    assert_eq!(json["audio_data_id"], 5);
    assert_eq!(json["payload"], serde_json::json!([1, 2, 3, 4]));
}

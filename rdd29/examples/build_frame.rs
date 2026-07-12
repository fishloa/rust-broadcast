//! Build an `ATMOSFrame` from typed bed/object/audio-essence elements, and
//! serialize it to wire bytes — SMPTE RDD 29:2019 §2/§4.
//!
//! Run with `cargo run -p rdd29 --example build_frame`.

use broadcast_common::Serialize;
use rdd29::{
    AnyElement, AtmosFrame, AudioDataDlc, AudioDescription, BedChannel, BedDefinition1, BitDepth,
    ChannelId, DecorCoefPrefix, FrameRate, ObjectDefinition1, ObjectSpreadMode, PanInfo,
    PanSubBlock, SampleRate,
};

fn main() {
    // A 2.0 bed: Left/Right screen speakers, pointing at two AudioDataDLC
    // tracks (audio_data_id 10/11).
    let bed = BedDefinition1::new(
        1,
        vec![
            BedChannel {
                channel_id: ChannelId::LeftScreen,
                audio_data_id: 10,
            },
            BedChannel {
                channel_id: ChannelId::RightScreen,
                audio_data_id: 11,
            },
        ],
    );

    // One panned object, centered in the room, snapping to the closest
    // speaker. FrameRate::Fps24 requires 8 pan sub-blocks (Table 7); only
    // sub-block 0 carries real pan info here, the rest repeat it.
    let mut pan_sub_blocks = vec![PanSubBlock {
        pan: Some(PanInfo {
            pos_x: 0x8000,
            pos_y: 0x8000,
            pos_z: 0x4000,
            snap: true,
            zone_gains: None,
            spread_mode: ObjectSpreadMode::Lowrez,
            spread: 32,
            decor_coef_prefix: DecorCoefPrefix::NoDecorrelation,
            decor_coef: None,
        }),
    }];
    pan_sub_blocks.resize(8, PanSubBlock { pan: None });
    let object = ObjectDefinition1::new(
        2,
        12,
        pan_sub_blocks,
        AudioDescription::with_text(b"footsteps").unwrap(),
    )
    .expect("build ObjectDefinition1");

    // Opaque audio-essence payloads -- this crate never decodes DLC audio,
    // so any bytes work here (see docs/rdd29.md scope decision 3).
    let left = AudioDataDlc::new(10, b"left channel DLC payload").unwrap();
    let right = AudioDataDlc::new(11, b"right channel DLC payload").unwrap();
    let object_audio = AudioDataDlc::new(12, b"object DLC payload").unwrap();

    let frame = AtmosFrame::new(
        SampleRate::Hz48000,
        BitDepth::Bits24,
        FrameRate::Fps24,
        3, // MaxRendered: 2 bed channels + 1 object
        vec![
            AnyElement::BedDefinition1(bed),
            AnyElement::ObjectDefinition1(object),
            AnyElement::AudioDataDlc(left),
            AnyElement::AudioDataDlc(right),
            AnyElement::AudioDataDlc(object_audio),
        ],
    );

    let bytes = frame.to_bytes();
    println!("serialized ATMOSFrame: {} bytes", bytes.len());
    println!(
        "version={} sample_rate={} bit_depth={} frame_rate={} max_rendered={}",
        frame.version, frame.sample_rate, frame.bit_depth, frame.frame_rate, frame.max_rendered
    );
    println!("sub-elements: {}", frame.elements.len());
}

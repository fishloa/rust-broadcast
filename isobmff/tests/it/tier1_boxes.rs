//! Tier 1 boxes no installed tool emits, tested with bytes hand-assembled from
//! the clause.
//!
//! GPAC 26.07 and ffmpeg 8.1.2 expose no option for these (see
//! `fixtures/mp4/boxes/PROVENANCE.md`, "Boxes no installed tool produced"), so
//! each byte sequence below is built from the syntax block it names. The four
//! boxes a real file does carry (`elng`, `kind`, `trgr`, `ssix`) are covered in
//! `tier1_fixtures.rs`.

use broadcast_core::{Parse, Serialize};

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn full(version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

#[test]
fn chnl_v0_explicit_positions_round_trip() {
    // §12.2.4.2: stream_structure=1 (channels), definedLayout=0, two positions.
    let body = full(0, 0, &[0x01, 0x00, 0x01, 0x02]);
    let bytes = boxed(*b"chnl", &body);
    let layout = isobmff::ChannelLayoutBox::parse(&bytes).expect("chnl");
    assert!(layout.is_channel_structured());
    assert!(!layout.is_object_structured());
    assert_eq!(layout.defined_layout(), Some(0));
    assert_eq!(layout.speaker_positions().len(), 2);
    assert_eq!(layout.speaker_positions()[1].speaker_position(), 2);
    assert_eq!(layout.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn chnl_v1_defined_layout_round_trip() {
    // §12.2.4.2 version 1: stream_structure=1, format_ordering=1, base=2,
    // definedLayout=2, channel_order_definition=0, no omitted map.
    let body = full(1, 0, &[0x11, 0x02, 0x02, 0x00]);
    let bytes = boxed(*b"chnl", &body);
    let layout = isobmff::ChannelLayoutBox::parse(&bytes).expect("chnl v1");
    assert_eq!(layout.version(), 1);
    assert_eq!(
        layout.format_ordering(),
        Some(isobmff::FormatOrdering::ChannelsThenObjects)
    );
    assert_eq!(layout.base_channel_count(), Some(2));
    assert_eq!(layout.defined_layout(), Some(2));
    assert_eq!(layout.channel_order_definition(), Some(0));
    assert_eq!(layout.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn dmix_v0_in_stream_round_trip() {
    // §12.2.5.2 version 0: one instruction, in-stream coefficients.
    let body = full(0, 0, &[0x02, 0x02, 0x84]);
    let bytes = boxed(*b"dmix", &body);
    let dmix = isobmff::DownMixInstructionsBox::parse(&bytes).expect("dmix");
    assert_eq!(dmix.version(), 0);
    assert_eq!(dmix.instructions().len(), 1);
    let instruction = &dmix.instructions()[0];
    assert_eq!(instruction.target_layout(), 2);
    assert_eq!(instruction.target_channel_count(), 2);
    assert!(instruction.in_stream());
    assert_eq!(instruction.downmix_id(), 4);
    assert_eq!(dmix.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn dmix_v1_two_in_stream_instructions_round_trip() {
    // §12.2.5.2 version 1: reserved(1)+count(7), then two in-stream entries.
    let body = full(1, 0, &[0x02, 0x02, 0x02, 0x81, 0x01, 0x01, 0x82]);
    let bytes = boxed(*b"dmix", &body);
    let dmix = isobmff::DownMixInstructionsBox::parse(&bytes).expect("dmix v1");
    assert_eq!(dmix.version(), 1);
    assert_eq!(dmix.instructions().len(), 2);
    assert_eq!(dmix.instructions()[1].downmix_id(), 2);
    assert_eq!(dmix.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn ludt_with_a_tlou_entry_round_trips() {
    // §12.2.7.2 version 0 entry: downmix 0, DRC 0, sample peak -1,
    // true peak -1, no measurements (the 56-bit entry is 7 bytes).
    let tlou_body = full(0, 0, &[0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00]);
    let body = boxed(*b"tlou", &tlou_body);
    let ludt = boxed(*b"ludt", &body);
    let parsed = isobmff::LoudnessBox::parse(&ludt).expect("ludt");
    assert_eq!(parsed.track().len(), 1);
    let entry = &parsed.track()[0].base().entries()[0];
    assert_eq!(entry.downmix_id(), 0);
    assert_eq!(entry.drc_set_id(), 0);
    assert_eq!(entry.sample_peak_level(), -1);
    assert_eq!(entry.true_peak_level(), -1);
    assert_eq!(entry.measurements().len(), 0);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), ludt);
}

#[test]
fn labl_round_trips() {
    // §8.10.5.2: label_id 5, language "en-US", label "Main".
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&5u16.to_be_bytes());
    body.extend_from_slice(b"en-US\0Main\0");
    let bytes = boxed(*b"labl", &body);
    let label = isobmff::LabelBox::parse(&bytes).expect("labl");
    assert_eq!(label.label_id(), 5);
    assert!(!label.is_group_label());
    assert_eq!(label.language_str(), Some("en-US"));
    assert_eq!(label.label_str(), Some("Main"));
    assert_eq!(label.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn tsel_round_trips() {
    // §8.10.3.3: switch_group 7, attributes `cdec`, `scsz`.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&7i32.to_be_bytes());
    body.extend_from_slice(b"cdecscsz");
    let bytes = boxed(*b"tsel", &body);
    let tsel = isobmff::TrackSelectionBox::parse(&bytes).expect("tsel");
    assert_eq!(tsel.switch_group(), 7);
    assert_eq!(tsel.attributes(), &[*b"cdec", *b"scsz"]);
    assert_eq!(tsel.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn stvi_round_trips() {
    // §13.5.4.2: single_view_allowed=3, stereo_scheme=1, four indication bytes.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&3u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&4u32.to_be_bytes());
    body.extend_from_slice(&8u32.to_be_bytes());
    let bytes = boxed(*b"stvi", &body);
    let stvi = isobmff::StereoVideoBox::parse(&bytes).expect("stvi");
    assert_eq!(stvi.single_view_allowed(), 3);
    assert_eq!(stvi.stereo_scheme(), 1);
    assert_eq!(stvi.stereo_indication(), &8u32.to_be_bytes());
    assert_eq!(stvi.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn leva_round_trips() {
    // §8.8.13.2: two levels, one sample-group assignment and one track level.
    let mut body = full(0, 0, &[2]);
    body.extend_from_slice(&1u32.to_be_bytes());
    body.push(0x00); // padding_flag 0, assignment_type 0
    body.extend_from_slice(b"tele");
    body.extend_from_slice(&2u32.to_be_bytes());
    body.push(0x02); // padding_flag 0, assignment_type 2
    let bytes = boxed(*b"leva", &body);
    let leva = isobmff::LevelAssignmentBox::parse(&bytes).expect("leva");
    assert_eq!(leva.levels().len(), 2);
    assert_eq!(
        leva.levels()[0].assignment(),
        isobmff::LevelAssignmentType::SampleGroup
    );
    assert_eq!(
        leva.levels()[0].grouping_type(),
        Some(u32::from_be_bytes(*b"tele"))
    );
    assert_eq!(
        leva.levels()[1].assignment(),
        isobmff::LevelAssignmentType::Track
    );
    assert_eq!(leva.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn pdin_round_trips() {
    // §8.1.3.2: two (rate, initial_delay) pairs.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&1000u32.to_be_bytes());
    body.extend_from_slice(&500u32.to_be_bytes());
    body.extend_from_slice(&2000u32.to_be_bytes());
    body.extend_from_slice(&250u32.to_be_bytes());
    let bytes = boxed(*b"pdin", &body);
    let pdin = isobmff::ProgressiveDownloadInfoBox::parse(&bytes).expect("pdin");
    assert_eq!(pdin.entries().len(), 2);
    assert_eq!(pdin.entries()[0].rate(), 1000);
    assert_eq!(pdin.entries()[1].initial_delay(), 250);
    assert_eq!(pdin.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn trep_round_trips_with_an_opaque_child() {
    // §8.8.15.2: track_ID then any number of boxes.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&3u32.to_be_bytes());
    body.extend_from_slice(&boxed(*b"abcd", &[1, 2, 3]));
    let bytes = boxed(*b"trep", &body);
    let trep = isobmff::TrackExtensionPropertiesBox::parse(&bytes).expect("trep");
    assert_eq!(trep.track_id(), 3);
    assert_eq!(trep.children().len(), 1);
    assert_eq!(trep.children()[0].box_type, *b"abcd");
    assert_eq!(trep.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn csgp_and_sbgp_feed_the_one_sample_group_lookup() {
    // §8.9.5.2: four-bit pattern/count/index fields, two patterns:
    // pattern [1,2] over 5 samples and pattern [0] over 2 samples.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(b"roll");
    body.extend_from_slice(&2u32.to_be_bytes());
    body.extend_from_slice(&[0x25, 0x12, 0x12, 0x00]);
    let bytes = boxed(*b"csgp", &body);
    let csgp = isobmff::sample_groups::CompactSampleToGroupBox::parse(&bytes).expect("csgp");
    assert_eq!(csgp.patterns().len(), 2);
    assert_eq!(csgp.patterns()[0].sample_count(), 5);
    assert_eq!(csgp.patterns()[0].indices(), &[1, 2]);
    assert_eq!(csgp.try_to_bytes().expect("bytes"), bytes);

    let csgp_mapping = csgp.mapping();
    assert_eq!(csgp_mapping.sample_count(), 7);
    assert_eq!(
        csgp_mapping
            .runs()
            .iter()
            .map(|run| (run.sample_count, run.group_description_index))
            .collect::<Vec<_>>(),
        vec![(1, 1), (1, 2), (1, 1), (1, 2), (1, 1), (2, 0)]
    );

    // The same mapping as an `sbgp` run list (§8.9.2.2).
    let mut sbgp_body = full(0, 0, &[]);
    sbgp_body.extend_from_slice(b"roll");
    sbgp_body.extend_from_slice(&6u32.to_be_bytes());
    for (count, index) in [(1u32, 1u32), (1, 2), (1, 1), (1, 2), (1, 1), (2, 0)] {
        sbgp_body.extend_from_slice(&count.to_be_bytes());
        sbgp_body.extend_from_slice(&index.to_be_bytes());
    }
    let sbgp = isobmff::sample_groups::SampleToGroupBox::parse(&boxed(*b"sbgp", &sbgp_body))
        .expect("sbgp");
    let sbgp_mapping = sbgp.mapping();
    for sample in 0..7 {
        assert_eq!(
            csgp_mapping.description_index(sample),
            sbgp_mapping.description_index(sample),
            "sample {sample}"
        );
    }
}

#[test]
fn stdp_round_trips() {
    // §8.5.3.2: one 16-bit priority per sample.
    let mut body = full(0, 0, &[]);
    for priority in [1u16, 2, 3] {
        body.extend_from_slice(&priority.to_be_bytes());
    }
    let bytes = boxed(*b"stdp", &body);
    let stdp = isobmff::DegradationPriorityBox::parse(&bytes).expect("stdp");
    assert_eq!(stdp.priorities(), &[1, 2, 3]);
    assert_eq!(stdp.priority(1), Some(2));
    assert_eq!(stdp.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn padb_round_trips() {
    // §8.7.6.2: three samples, pads 1, 2 and 3.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&3u32.to_be_bytes());
    body.extend_from_slice(&[0x12, 0x30]);
    let bytes = boxed(*b"padb", &body);
    let padb = isobmff::PaddingBitsBox::parse(&bytes).expect("padb");
    assert_eq!(padb.sample_count(), 3);
    assert_eq!(padb.padding_bits(), &[1, 2, 3]);
    assert_eq!(padb.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn stsg_round_trips() {
    // §8.13.6.2: grouping_type `roll`, two description indices.
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(b"roll");
    body.extend_from_slice(&2u16.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&2u32.to_be_bytes());
    let bytes = boxed(*b"stsg", &body);
    let stsg = isobmff::SubTrackSampleGroupBox::parse(&bytes).expect("stsg");
    assert_eq!(stsg.grouping_type(), u32::from_be_bytes(*b"roll"));
    assert_eq!(stsg.group_description_indices(), &[1, 2]);
    assert_eq!(stsg.try_to_bytes().expect("bytes"), bytes);
}

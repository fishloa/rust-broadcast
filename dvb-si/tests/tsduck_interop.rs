//! Interop test: decode a PMT compiled by TSDuck (the reference DVB toolkit,
//! `tstabcomp`) carrying MPEG-2 systems descriptors, and assert our typed
//! decode matches what TSDuck encoded.
//!
//! Fixture `tsduck-mpeg-descriptors-pmt.bin` was produced from
//! `tsduck-mpeg-descriptors-pmt.xml` via `tstabcomp -c`. This is independent
//! cross-validation — TSDuck encodes, we decode — far stronger than a
//! self-round-trip. In particular TSDuck sets the HEVC reserved bits [3:2] to
//! '1', so the `HDR_WCG_idc` (bits [1:0]) assertion catches any parser that
//! reads it from the wrong bit position.

use std::fs;

use broadcast_common::Parse;
use dvb_si::descriptors::hdr_wcg_idc::HdrWcgIdc;
use dvb_si::descriptors::video_stream::FrameRateCode;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::pmt::PmtSection;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../fixtures/dvb-si/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    fs::read(path).unwrap_or_else(|e| panic!("fixture {name} must be present: {e}"))
}

#[test]
fn decodes_tsduck_compiled_mpeg_descriptors() {
    let data = fixture("tsduck-mpeg-descriptors-pmt.bin");
    let pmt = PmtSection::parse(&data).expect("TSDuck PMT section must parse");

    let mut seen_video = false;
    let mut seen_audio = false;
    let mut seen_avc = false;
    let mut seen_hevc = false;

    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            match desc.expect("descriptor must parse") {
                AnyDescriptor::VideoStream(d) => {
                    seen_video = true;
                    assert!(d.multiple_frame_rate_flag);
                    assert_eq!(d.frame_rate_code, FrameRateCode::Frame29_97); // code 4
                    assert!(!d.mpeg_1_only_flag);
                    assert_eq!(d.profile_and_level_indication, Some(0x4D));
                    assert_eq!(d.chroma_format, Some(1));
                }
                AnyDescriptor::AudioStream(d) => {
                    seen_audio = true;
                    assert!(d.id);
                    assert_eq!(d.layer, 2);
                    assert!(!d.free_format_flag);
                }
                AnyDescriptor::AvcVideo(d) => {
                    seen_avc = true;
                    assert_eq!(d.profile_idc, 0x64);
                    assert!(d.constraint_set0_flag);
                    assert!(d.constraint_set3_flag);
                    assert!(d.constraint_set5_flag);
                    assert_eq!(d.avc_compatible_flags, 0x03);
                    assert_eq!(d.level_idc, 0x29);
                    assert!(d.avc_24_hour_picture_flag);
                }
                AnyDescriptor::HevcVideo(d) => {
                    seen_hevc = true;
                    assert_eq!(d.profile_space, 1);
                    assert!(d.tier_flag);
                    assert_eq!(d.profile_idc, 2);
                    assert_eq!(d.profile_compatibility_indication, 0xDEAD_BEEF);
                    assert_eq!(d.copied_44bits, 0x123_4567_89AB);
                    assert_eq!(d.level_idc, 0x99);
                    assert!(d.hevc_still_present_flag);
                    // TSDuck sets reserved bits [3:2] to 1; HDR_WCG_idc is bits [1:0].
                    // A parser reading the wrong bits would decode NoIndication (3).
                    assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
                    let ts = d.temporal_sub.expect("temporal sub-block present");
                    assert_eq!(ts.temporal_id_min, 3);
                    assert_eq!(ts.temporal_id_max, 6);
                }
                _ => {}
            }
        }
    }

    assert!(seen_video, "video_stream_descriptor not decoded");
    assert!(seen_audio, "audio_stream_descriptor not decoded");
    assert!(seen_avc, "AVC_video_descriptor not decoded");
    assert!(seen_hevc, "HEVC_video_descriptor not decoded");
}


#[test]
fn decodes_tsduck_compiled_conditional_descriptors() {
    use dvb_si::descriptors::decoder_config_flags::DecoderConfigFlags;
    use dvb_si::descriptors::metadata::DecoderConfig;
    use dvb_si::descriptors::metadata_format::MetadataFormat;
    use dvb_si::descriptors::mpeg_carriage_flags::MpegCarriageFlags;

    let data = fixture("tsduck-metadata-j2k-pmt.bin");
    let pmt = PmtSection::parse(&data).expect("TSDuck PMT section must parse");
    let mut seen_ptr = false;
    let mut seen_meta = false;
    let mut seen_j2k = false;

    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            match desc.expect("descriptor must parse") {
                AnyDescriptor::MetadataPointer(d) => {
                    seen_ptr = true;
                    assert_eq!(d.metadata_application_format, 0x0010);
                    assert_eq!(d.metadata_format, MetadataFormat::TeM);
                    assert_eq!(d.metadata_service_id, 0x07);
                    assert_eq!(d.mpeg_carriage_flags, MpegCarriageFlags::SameTs);
                    assert_eq!(d.program_number, Some(0x1234));
                }
                AnyDescriptor::Metadata(d) => {
                    seen_meta = true;
                    assert_eq!(d.metadata_application_format, 0x0011);
                    assert_eq!(d.metadata_format, MetadataFormat::AppFormat);
                    assert_eq!(d.metadata_service_id, 0x09);
                    assert_eq!(d.decoder_config_flags, DecoderConfigFlags::OtherService);
                    match d.decoder_config {
                        DecoderConfig::OtherService(ref o) => {
                            assert_eq!(o.decoder_config_metadata_service_id, 0x2A);
                        }
                        _ => panic!("expected OtherService decoder_config"),
                    }
                }
                AnyDescriptor::J2kVideo(d) => {
                    seen_j2k = true;
                    assert!(!d.extended_capability_flag);
                    assert!(d.extended_capability.is_none());
                    assert_eq!(d.profile_and_level, 0x0102);
                    assert_eq!(d.horizontal_size, 1920);
                    assert_eq!(d.vertical_size, 1080);
                    assert_eq!(d.color_specification, Some(0x01));
                    assert!(!d.still_mode);
                    assert!(d.interlaced_video);
                }
                _ => {}
            }
        }
    }
    assert!(seen_ptr, "metadata_pointer_descriptor not decoded");
    assert!(seen_meta, "metadata_descriptor not decoded");
    assert!(seen_j2k, "J2K_video_descriptor not decoded");
}


#[test]
fn decodes_tsduck_compiled_simple_descriptors() {
    let data = fixture("tsduck-mpeg-simple-pmt.bin");
    let pmt = PmtSection::parse(&data).expect("TSDuck PMT section must parse");
    let mut seen = std::collections::BTreeSet::new();
    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            match desc.expect("descriptor must parse") {
                AnyDescriptor::Hierarchy(d) => {
                    seen.insert("hierarchy");
                    assert_eq!(d.hierarchy_layer_index, 2);
                }
                AnyDescriptor::TargetBackgroundGrid(d) => {
                    seen.insert("tbg");
                    assert_eq!(d.horizontal_size, 1920);
                    assert_eq!(d.aspect_ratio_information, 3);
                }
                AnyDescriptor::VideoWindow(d) => {
                    seen.insert("vw");
                    assert_eq!(d.window_priority, 5);
                }
                AnyDescriptor::SystemClock(d) => {
                    seen.insert("sysclk");
                    assert!(d.external_clock_reference_indicator);
                    assert_eq!(d.clock_accuracy_integer, 30);
                }
                AnyDescriptor::MaximumBitrate(d) => {
                    seen.insert("maxbr");
                    // TSDuck XML "50000" is bits/s; the raw 22-bit field is in
                    // units of 50 bytes/s = 400 bits/s → 50000/400 = 125.
                    assert_eq!(d.maximum_bitrate, 125);
                }
                AnyDescriptor::SmoothingBuffer(d) => {
                    seen.insert("smooth");
                    assert_eq!(d.sb_size, 8192);
                }
                AnyDescriptor::Std(d) => {
                    seen.insert("std");
                    assert!(d.leak_valid_flag);
                }
                AnyDescriptor::Ibp(d) => {
                    seen.insert("ibp");
                    assert!(d.closed_gop_flag);
                    assert_eq!(d.max_gop_length, 15);
                }
                AnyDescriptor::AvcTimingAndHrd(d) => {
                    seen.insert("avctiming");
                    assert!(d.hrd_management_valid_flag);
                }
                AnyDescriptor::Mpeg2AacAudio(d) => {
                    seen.insert("aac");
                    assert_eq!(d.mpeg_2_aac_profile, 0x40);
                }
                AnyDescriptor::SvcExtension(d) => {
                    seen.insert("svc");
                    assert_eq!(d.width, 1920);
                }
                AnyDescriptor::MvcExtension(d) => {
                    seen.insert("mvc");
                    assert_eq!(d.view_order_index_max, 512);
                }
                _ => {}
            }
        }
    }
    for k in [
        "hierarchy",
        "tbg",
        "vw",
        "sysclk",
        "maxbr",
        "smooth",
        "std",
        "ibp",
        "avctiming",
        "aac",
        "svc",
        "mvc",
    ] {
        assert!(
            seen.contains(k),
            "{k} descriptor not decoded from TSDuck PMT"
        );
    }
}


#[test]
fn decodes_tsduck_compiled_protection_message() {
    use broadcast_common::Serialize;
    use dvb_si::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    let data = fixture("tsduck-protection-message-pmt.bin");
    let pmt = PmtSection::parse(&data).expect("TSDuck PMT section must parse");
    let mut seen = false;
    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            if let AnyDescriptor::Extension(ext) = desc.expect("descriptor must parse") {
                if let ExtensionBody::ProtectionMessage(pm) = &ext.body {
                    seen = true;
                    assert_eq!(ext.kind(), Some(ExtensionTag::ProtectionMessage));
                    // TSDuck encoded reserved bits as all-ones; we must preserve them.
                    assert_eq!(pm.reserved, 0x0F);
                    assert_eq!(pm.component_tags, &[0x10, 0x20, 0x33]);
                    // Byte-exact round-trip of the descriptor TSDuck produced.
                    let mut buf = vec![0u8; ext.serialized_len()];
                    ext.serialize_into(&mut buf).unwrap();
                    let mut orig = vec![0u8; ext.serialized_len()];
                    ExtensionDescriptor::parse(&buf)
                        .unwrap()
                        .serialize_into(&mut orig)
                        .unwrap();
                    assert_eq!(buf, orig);
                }
            }
        }
    }
    assert!(
        seen,
        "protection_message_descriptor not decoded from TSDuck PMT"
    );
}


#[test]
fn decodes_tsduck_compiled_cpcm_delivery_signalling() {
    use broadcast_common::Serialize;
    use dvb_si::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    let data = fixture("tsduck-cpcm-delivery-signalling-pmt.bin");
    let pmt = PmtSection::parse(&data).expect("TSDuck PMT section must parse");
    let mut seen = false;
    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            if let AnyDescriptor::Extension(ext) = desc.expect("descriptor must parse") {
                if let ExtensionBody::CpcmDeliverySignalling(cpcm) = &ext.body {
                    seen = true;
                    assert_eq!(ext.kind(), Some(ExtensionTag::CpcmDeliverySignalling));
                    assert_eq!(cpcm.cpcm_version, 0x01);
                    assert!(!cpcm.selector_bytes.is_empty());
                    let mut buf = vec![0u8; ext.serialized_len()];
                    ext.serialize_into(&mut buf).unwrap();
                    let mut round = vec![0u8; ext.serialized_len()];
                    ExtensionDescriptor::parse(&buf)
                        .unwrap()
                        .serialize_into(&mut round)
                        .unwrap();
                    assert_eq!(buf, round);
                }
            }
        }
    }
    assert!(
        seen,
        "cpcm_delivery_signalling_descriptor not decoded from TSDuck PMT"
    );
}


#[test]
fn decodes_tsduck_compiled_dts_descriptors() {
    use broadcast_common::Serialize;
    use dvb_si::descriptors::extension::SamplingFrequency;
    use dvb_si::descriptors::extension::{
        ExtensionBody, ExtensionDescriptor, ExtensionTag, FrameDurationCode, MaxPayloadCode,
    };
    use dvb_si::text::LangCode;

    let data = fixture("tsduck-dts-descriptors-pmt.bin");
    let pmt = PmtSection::parse(&data).expect("TSDuck PMT section must parse");
    let mut seen_hd = false;
    let mut seen_uhd = false;
    let mut seen_neural = false;

    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            if let AnyDescriptor::Extension(ext) = desc.expect("descriptor must parse") {
                match &ext.body {
                    ExtensionBody::DtsHd(hd) => {
                        seen_hd = true;
                        assert_eq!(ext.kind(), Some(ExtensionTag::DtsHd));
                        assert!(hd.substream_core_flag);
                        assert!(!hd.substream_0_flag);
                        assert!(!hd.substream_1_flag);
                        assert!(!hd.substream_2_flag);
                        assert!(!hd.substream_3_flag);
                        assert_eq!(hd.reserved, 7);
                        assert_eq!(hd.substreams.len(), 1);
                        let s = &hd.substreams[0];
                        assert_eq!(s.channel_count, 6);
                        assert!(s.lfe_flag);
                        assert_eq!(s.sampling_frequency, SamplingFrequency::Khz48);
                        assert!(s.sample_resolution);
                        assert_eq!(s.reserved, 3);
                        assert_eq!(s.assets.len(), 1);
                        let a = &s.assets[0];
                        assert_eq!(a.asset_construction, 1);
                        assert!(!a.vbr_flag);
                        assert!(!a.post_encode_br_scaling_flag);
                        assert!(a.component_type_flag);
                        assert!(a.language_code_flag);
                        assert_eq!(a.bit_rate_or_scaled, 755);
                        assert_eq!(a.reserved, 3);
                        assert_eq!(a.component_type, Some(0x42));
                        assert_eq!(a.iso_639_language_code, Some(LangCode(*b"eng")));
                        assert!(hd.additional_info.is_empty());

                        // Byte-exact round-trip
                        let mut buf = vec![0u8; ext.serialized_len()];
                        ext.serialize_into(&mut buf).unwrap();
                        let mut round = vec![0u8; ext.serialized_len()];
                        ExtensionDescriptor::parse(&buf)
                            .unwrap()
                            .serialize_into(&mut round)
                            .unwrap();
                        assert_eq!(buf, round);
                    }
                    ExtensionBody::DtsUhd(uhd) => {
                        seen_uhd = true;
                        assert_eq!(ext.kind(), Some(ExtensionTag::DtsUhd));
                        assert_eq!(uhd.decoder_profile_code, 1);
                        assert_eq!(uhd.decoder_profile(), 3);
                        assert_eq!(uhd.frame_duration_code, FrameDurationCode::Samples2048);
                        assert_eq!(uhd.max_payload_code, MaxPayloadCode::Byte8192);
                        assert_eq!(uhd.dts_reserved, 0);
                        assert_eq!(uhd.stream_index, 3);
                        assert_eq!(uhd.codec_selector, &[0xAB, 0xCD]);

                        // Byte-exact round-trip
                        let mut buf = vec![0u8; ext.serialized_len()];
                        ext.serialize_into(&mut buf).unwrap();
                        let mut round = vec![0u8; ext.serialized_len()];
                        ExtensionDescriptor::parse(&buf)
                            .unwrap()
                            .serialize_into(&mut round)
                            .unwrap();
                        assert_eq!(buf, round);
                    }
                    ExtensionBody::DtsNeural(neural) => {
                        seen_neural = true;
                        assert_eq!(ext.kind(), Some(ExtensionTag::DtsNeural));
                        assert_eq!(neural.config_id, 0x07);
                        assert_eq!(neural.additional_info, &[0xFF]);

                        // Byte-exact round-trip
                        let mut buf = vec![0u8; ext.serialized_len()];
                        ext.serialize_into(&mut buf).unwrap();
                        let mut round = vec![0u8; ext.serialized_len()];
                        ExtensionDescriptor::parse(&buf)
                            .unwrap()
                            .serialize_into(&mut round)
                            .unwrap();
                        assert_eq!(buf, round);
                    }
                    _ => {}
                }
            }
        }
    }
    assert!(seen_hd, "DTS-HD descriptor not decoded from TSDuck PMT");
    assert!(seen_uhd, "DTS-UHD descriptor not decoded from TSDuck PMT");
    assert!(
        seen_neural,
        "DTS-Neural descriptor not decoded from TSDuck PMT"
    );
}

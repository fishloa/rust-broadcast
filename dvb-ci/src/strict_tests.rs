//! One table over EVERY parser that rejects trailing bytes (audit r10-O-1):
//! for each, a valid body must parse and re-serialize byte-identically, and the
//! same APDU with a declared-and-present extra byte must be refused with the
//! `trailing bytes` error. Deleting any `reject_trailing_body` call makes the
//! matching row fail.

use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::tag::ApduTag;

const TRAILING: &str = "trailing bytes after the fixed body";
/// Longest fixed body in the table (the CI+ `cc_PIN_event`/`sd_start_reply`).
const MAX_PROBE_BODY: usize = 40;
/// Fill bytes tried when no explicit body is given: zeros, ones, and values
/// that satisfy marker-bit/reserved-bit checks.
const FILLS: [u8; 5] = [0x00, 0xFF, 0x01, 0x80, 0x41];

fn apdu(tag: ApduTag, body: &[u8]) -> Vec<u8> {
    let mut v = tag.to_bytes().to_vec();
    v.push(u8::try_from(body.len()).unwrap()); // short-form length, < 128
    v.extend_from_slice(body);
    v
}

/// Rewrite the `length_field` of `wire` (which starts with a `tag_len`-byte
/// tag and a minimal length) into the long form with `octets` length octets.
fn widen(wire: &[u8], tag_len: usize, octets: usize) -> Vec<u8> {
    let (value, hdr) = crate::length::decode(&wire[tag_len..]).unwrap();
    let mut out = wire[..tag_len].to_vec();
    out.push(0x80 | u8::try_from(octets).unwrap());
    for i in (0..octets).rev() {
        out.push(u8::try_from((value >> (8 * i)) & 0xFF).unwrap());
    }
    out.extend_from_slice(&wire[tag_len + hdr..]);
    out
}

/// r10-W-4: a non-minimal `length_field` (`81 NN`, `82 00 NN`, `83 00 00 NN`,
/// `84 00 00 00 NN`) must parse and re-serialize byte-for-byte — the real CAM
/// traffic form (`82 00 09`, dvb-ci-runtime #337) is not rewritten minimally.
fn assert_forms_round_trip(
    name: &str,
    wire: &[u8],
    tag_len: usize,
    rt: &dyn Fn(&[u8]) -> Result<Vec<u8>>,
) {
    for octets in 1..=crate::length::MAX_LENGTH_OCTETS as usize {
        let wide = widen(wire, tag_len, octets);
        if octets == 1 && wide == wire {
            continue; // value >= 0x80: the 1-octet long form IS the minimal one
        }
        let out = rt(&wide).unwrap_or_else(|e| panic!("{name}: {octets}-octet form: {e:?}"));
        assert_eq!(
            out, wide,
            "{name}: {octets}-octet length form not preserved"
        );
        assert_ne!(out, wire, "{name}: re-encoded minimally");
    }
}

fn check(name: &str, tag: ApduTag, explicit: Option<&[u8]>, rt: &dyn Fn(&[u8]) -> Result<Vec<u8>>) {
    let candidates: Vec<Vec<u8>> = match explicit {
        Some(b) => alloc::vec![b.to_vec()],
        None => (0..=MAX_PROBE_BODY)
            .flat_map(|n| FILLS.iter().map(move |&f| alloc::vec![f; n]))
            .collect(),
    };
    let valid = candidates
        .into_iter()
        .find(|b| {
            let wire = apdu(tag, b);
            rt(&wire).is_ok_and(|out| out == wire)
        })
        .unwrap_or_else(|| panic!("{name}: no valid body found"));
    assert_forms_round_trip(name, &apdu(tag, &valid), tag.to_bytes().len(), rt);
    let mut padded = valid.clone();
    padded.push(0xEE);
    let err = rt(&apdu(tag, &padded)).expect_err(name);
    assert!(
        matches!(&err, Error::InvalidObject { reason, .. } if *reason == TRAILING),
        "{name}: expected the trailing-bytes error, got {err:?}"
    );
}

macro_rules! strict {
    ($ty:ty, $tag:expr) => {
        check(stringify!($ty), $tag, None, &|b| {
            <$ty>::parse(b)?.try_to_bytes()
        })
    };
    ($ty:ty, $tag:expr, $body:expr) => {
        check(stringify!($ty), $tag, Some(&$body), &|b| {
            <$ty>::parse(b)?.try_to_bytes()
        })
    };
}

#[test]
fn every_fixed_layout_parser_rejects_a_padding_byte() {
    use crate::ci_ext::{
        application_mmi, broadcast_service_gateway, copy_protection, event_manager, power_manager,
        resource_manager_v2, service_gateway, stream_input,
    };
    use crate::ci_plus::{
        cicam_player as cp, content_control as cc, file_retrieval, low_speed_comms_v4 as lsc4,
        multistream, multistream_host_control as mhc, sample_decryption as sd,
    };
    use crate::objects::{host_control, low_speed_comms, mmi_close, mmi_display, mmi_high};
    use crate::tag as t;

    // objects/ (EN 50221)
    strict!(host_control::Tune, t::TUNE);
    strict!(host_control::Replace, t::REPLACE);
    strict!(low_speed_comms::CommsReply, t::COMMS_REPLY);
    strict!(mmi_display::DownloadReply, t::DOWNLOAD_REPLY);
    // r10-O-1 parsers with a branch-dependent body: both branches.
    strict!(mmi_close::CloseMmi, t::CLOSE_MMI, [0x00]); // immediate
    strict!(mmi_close::CloseMmi, t::CLOSE_MMI, [0x01, 0x05]); // delay + seconds
    strict!(mmi_display::DisplayControl, t::DISPLAY_CONTROL, [0x02]); // no mmi_mode
    strict!(
        mmi_display::DisplayControl,
        t::DISPLAY_CONTROL,
        [0x01, 0x01]
    ); // set_mmi_mode
    strict!(mmi_high::Answ, t::ANSW, [0x00]); // cancel
    strict!(
        low_speed_comms::CommsCmd,
        t::COMMS_CMD,
        [0x01, 0x9F, 0x8C, 0x01, 0x02, 0x02, 0x07, 0x03, 0x1E] // connect
    );
    strict!(low_speed_comms::CommsCmd, t::COMMS_CMD, [0x02]); // disconnect
    strict!(low_speed_comms::CommsCmd, t::COMMS_CMD, [0x03, 0x0A, 0x14]); // set_params
    strict!(low_speed_comms::CommsCmd, t::COMMS_CMD, [0x05, 0x01]); // get_next_buffer
    strict!(
        cp::PlayerCapabilitiesReply,
        cp::tag::CAPABILITIES_REPLY,
        [0x00, 0x00]
    ); // n = 0
    strict!(
        cp::PlayerCapabilitiesReply,
        cp::tag::CAPABILITIES_REPLY,
        [0x00, 0x01, 0x1F, 0x07] // n = 1
    );

    // ci_ext/
    strict!(
        resource_manager_v2::ModuleIdSend,
        resource_manager_v2::tag::MODULE_ID_SEND
    );
    strict!(
        resource_manager_v2::ModuleIdCommand,
        resource_manager_v2::tag::MODULE_ID_COMMAND
    );
    strict!(
        application_mmi::RequestStartAck,
        application_mmi::tag::REQUEST_START_ACK
    );
    strict!(copy_protection::CpReply, copy_protection::tag::CP_REPLY);
    strict!(
        event_manager::EventRequestAck,
        event_manager::tag::EVENT_REQUEST_ACK
    );
    strict!(
        event_manager::EventNotification,
        event_manager::tag::EVENT_NOTIFICATION
    );
    strict!(
        power_manager::ActivationStateChangeRequest,
        power_manager::tag::ACTIVATION_STATE_CHANGE_REQUEST
    );
    strict!(
        power_manager::ActivationStateChangeAck,
        power_manager::tag::ACTIVATION_STATE_CHANGE_ACK
    );
    strict!(
        service_gateway::GetServiceAck,
        service_gateway::tag::GET_SERVICE_ACK
    );
    strict!(
        broadcast_service_gateway::EitSectionReq,
        broadcast_service_gateway::tag::EIT_SECTION_REQ
    );
    strict!(stream_input::ScanAck, stream_input::tag::SCAN_ACK);
    strict!(stream_input::TuneTSAck, stream_input::tag::TUNE_TS_ACK);

    // ci_plus/
    strict!(sd::SdStartReply, sd::tag::SD_START_REPLY);
    strict!(sd::SdUpdateReply, sd::tag::SD_UPDATE_REPLY);
    strict!(
        multistream::CicamMultistreamCapability,
        multistream::tag::CICAM_MULTISTREAM_CAPABILITY
    );
    strict!(cp::PlayerVerifyReply, cp::tag::VERIFY_REPLY);
    strict!(cp::PlayerStartReply, cp::tag::START_REPLY);
    strict!(cp::PlayerStatusError, cp::tag::STATUS_ERROR);
    strict!(cp::PlayerInfoReply, cp::tag::INFO_REPLY);
    strict!(cp::PlayerAssetEnd, cp::tag::ASSET_END);
    strict!(cp::PlayerUpdateReply, cp::tag::UPDATE_REPLY);
    strict!(
        cp::PlayerControlReq,
        cp::tag::CONTROL_REQ,
        [0x01, 0x02, 0x00, 0x64]
    ); // set_speed
    strict!(
        cp::PlayerControlReq,
        cp::tag::CONTROL_REQ,
        [0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0xE8] // set_position
    );
    strict!(cp::PlayerControlReq, cp::tag::CONTROL_REQ, [0x01, 0x00]); // reserved command
    strict!(lsc4::CommsInfoReply, lsc4::tag::COMMS_INFO_REPLY);
    strict!(
        file_retrieval::FileSystemAck,
        file_retrieval::tag::FILE_SYSTEM_ACK
    );
    strict!(cc::CcPinReply, cc::tag::CC_PIN_REPLY);
    strict!(cc::CcPinEvent, cc::tag::CC_PIN_EVENT);
    strict!(mhc::TuneTripletReq, mhc::tag::TUNE_TRIPLET_REQ);
    strict!(mhc::TuneLcnReq, mhc::tag::TUNE_LCN_REQ);
}

/// Drift guard over every `declare_resource_apdus!` enum: its `TAGS` list has
/// no duplicates, and for every declared tag an APDU carrying that tag parses
/// to the variant whose `tag()` is that tag (so no list line dispatches to the
/// wrong variant). Byte-exact round trips are covered per type elsewhere.
macro_rules! drift {
    ($ty:ty $(, $xtag:expr => $xbody:expr)* $(,)?) => {{
        // Explicit bodies for variants whose layout fill bytes cannot reach.
        let explicit: &[(ApduTag, &[u8])] = &[ $( ($xtag, &$xbody) ),* ];
        let tags = <$ty>::TAGS;
        for (i, a) in tags.iter().enumerate() {
            assert!(
                tags[i + 1..].iter().all(|b| b != a),
                "{}: duplicate tag {a:?}",
                stringify!($ty)
            );
        }
        for &tag in tags {
            let hit = explicit
                .iter()
                .filter(|(t, _)| *t == tag)
                .map(|(_, b)| b.to_vec())
                .chain(
                    (0..=MAX_PROBE_BODY)
                        .flat_map(|n| FILLS.iter().map(move |&f| alloc::vec![f; n])),
                )
                .map(|b| apdu(tag, &b))
                .find(|wire| {
                    <$ty>::parse(wire).is_ok_and(|v| v.tag() == tag)
                });
            assert!(
                hit.is_some(),
                "{}: no probe body parsed back to tag {tag:?}",
                stringify!($ty)
            );
        }
    }};
}

#[test]
fn generated_dispatch_enums_agree_with_their_tag_lists() {
    use crate::ci_ext::{
        application_info_v2::ApplicationInfoV2Apdu, application_mmi::ApplicationMmiApdu,
        ca_pipeline::CaPipelineApdu, copy_protection::CopyProtectionApdu,
        event_manager::EventManagerApdu, power_manager::PowerManagerApdu,
        resource_manager_v2::ResourceManagerV2Apdu, service_gateway::ServiceGatewayApdu,
        software_download::DownloadApdu, status_query::StatusQueryApdu,
        stream_input::StreamInputApdu,
    };
    use crate::ci_plus::{
        ca_support::CaSupportApdu, cicam_player::CicamPlayerApdu,
        content_control::ContentControlApdu, file_retrieval::FileRetrievalApdu,
        low_speed_comms_v4::LscV4Apdu, multistream::MultistreamApdu,
        sample_decryption::SampleDecryptionApdu,
    };
    drift!(ApplicationInfoV2Apdu);
    drift!(ApplicationMmiApdu);
    drift!(CaPipelineApdu);
    drift!(CopyProtectionApdu);
    drift!(EventManagerApdu);
    drift!(PowerManagerApdu);
    drift!(ResourceManagerV2Apdu);
    drift!(ServiceGatewayApdu);
    drift!(DownloadApdu);
    drift!(StatusQueryApdu);
    drift!(StreamInputApdu);
    drift!(CaSupportApdu);
    drift!(CicamPlayerApdu);
    drift!(ContentControlApdu);
    drift!(FileRetrievalApdu);
    drift!(LscV4Apdu);
    drift!(MultistreamApdu);
    drift!(SampleDecryptionApdu);
}

/// Every resource-scoped dispatch enum: for each declared tag that some probe
/// body round-trips through, the same APDU in every non-minimal length form
/// re-serializes byte-for-byte (r10-W-4).
macro_rules! forms {
    ($covered:ident, $ty:ty) => {
        for &tag in <$ty>::TAGS {
            let rt = |b: &[u8]| -> Result<Vec<u8>> { <$ty>::parse(b)?.try_to_bytes() };
            let hit = (0..=MAX_PROBE_BODY)
                .flat_map(|n| FILLS.iter().map(move |&f| alloc::vec![f; n]))
                .map(|b| apdu(tag, &b))
                .find(|wire| rt(wire).is_ok_and(|out| &out == wire));
            if let Some(wire) = hit {
                assert_forms_round_trip(stringify!($ty), &wire, 3, &rt);
                $covered += 1;
            }
        }
    };
}

#[test]
fn non_minimal_length_forms_round_trip_across_the_dispatch_enums() {
    use crate::ci_ext::{
        application_info_v2::ApplicationInfoV2Apdu, application_mmi::ApplicationMmiApdu,
        ca_pipeline::CaPipelineApdu, copy_protection::CopyProtectionApdu,
        event_manager::EventManagerApdu, power_manager::PowerManagerApdu,
        resource_manager_v2::ResourceManagerV2Apdu, service_gateway::ServiceGatewayApdu,
        software_download::DownloadApdu, status_query::StatusQueryApdu,
        stream_input::StreamInputApdu,
    };
    use crate::ci_plus::{
        ca_support::CaSupportApdu, cicam_player::CicamPlayerApdu,
        content_control::ContentControlApdu, file_retrieval::FileRetrievalApdu,
        low_speed_comms_v4::LscV4Apdu, multistream::MultistreamApdu,
        sample_decryption::SampleDecryptionApdu,
    };
    let mut covered = 0usize;
    forms!(covered, ApplicationInfoV2Apdu);
    forms!(covered, ApplicationMmiApdu);
    forms!(covered, CaPipelineApdu);
    forms!(covered, CopyProtectionApdu);
    forms!(covered, EventManagerApdu);
    forms!(covered, PowerManagerApdu);
    forms!(covered, ResourceManagerV2Apdu);
    forms!(covered, ServiceGatewayApdu);
    forms!(covered, DownloadApdu);
    forms!(covered, StatusQueryApdu);
    forms!(covered, StreamInputApdu);
    forms!(covered, CaSupportApdu);
    forms!(covered, CicamPlayerApdu);
    forms!(covered, ContentControlApdu);
    forms!(covered, FileRetrievalApdu);
    forms!(covered, LscV4Apdu);
    forms!(covered, MultistreamApdu);
    forms!(covered, SampleDecryptionApdu);
    assert!(covered >= 40, "only {covered} APDU tags were probed");
}

/// The EN 50221 `objects/` APDUs (no dispatch enum): explicit wires, each
/// parsed and re-serialized in every non-minimal length form.
#[test]
fn non_minimal_length_forms_round_trip_across_the_en50221_objects() {
    use crate::objects::{
        application_info::{ApplicationInfo, ApplicationInfoEnq, EnterMenu},
        ca_info::{CaInfo, CaInfoEnq},
        date_time::DateTimeEnq,
        resource_manager::{Profile, ProfileChange, ProfileEnq},
    };
    macro_rules! obj {
        ($ty:ty, $wire:expr) => {{
            let wire: &[u8] = &$wire;
            let rt = |b: &[u8]| -> Result<Vec<u8>> { <$ty>::parse(b)?.try_to_bytes() };
            assert_eq!(rt(wire).unwrap(), wire, "{}: baseline", stringify!($ty));
            assert_forms_round_trip(stringify!($ty), wire, 3, &rt);
        }};
    }
    obj!(ApplicationInfoEnq, [0x9F, 0x80, 0x20, 0x00]);
    obj!(EnterMenu, [0x9F, 0x80, 0x22, 0x00]);
    obj!(
        ApplicationInfo<'_>,
        [
            0x9F, 0x80, 0x21, 0x08, 0x01, 0x12, 0x34, 0x56, 0x78, 0x02, 0x41, 0x42
        ]
    );
    obj!(CaInfoEnq, [0x9F, 0x80, 0x30, 0x00]);
    obj!(CaInfo, [0x9F, 0x80, 0x31, 0x04, 0x01, 0x00, 0x09, 0x63]);
    obj!(ProfileEnq, [0x9F, 0x80, 0x10, 0x00]);
    obj!(ProfileChange, [0x9F, 0x80, 0x12, 0x00]);
    obj!(
        Profile,
        [
            0x9F, 0x80, 0x11, 0x08, 0x00, 0x01, 0x00, 0x41, 0x00, 0x02, 0x00, 0x41
        ]
    );
    obj!(DateTimeEnq, [0x9F, 0x84, 0x40, 0x01, 0x05]);
}

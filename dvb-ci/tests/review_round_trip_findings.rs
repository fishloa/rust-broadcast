//! Adversarial review (branch `review/ci-cam-crypto`): FAILING regression
//! tests for parse -> serialize round-trip violations found by a randomized
//! sweep of every `Parse` impl in this crate. Each input is a minimal APDU /
//! SPDU a CAM can send. The invariant (CLAUDE.md, and the r10-W-4 / r10-O-1
//! CHANGELOG entries) is: whatever `parse` accepts must re-serialize
//! byte-for-byte. A fix may either reject the input or preserve the bytes;
//! silently dropping or rewriting them is the defect.

use broadcast_common::{Parse, Serialize};
use dvb_ci::ci_ext::{
    broadcast_service_gateway, copy_protection, resource_manager_v2, service_gateway,
};
use dvb_ci::ci_plus::{cicam_player, content_control, multistream_host_control};
use dvb_ci::objects::{application_info, ca_pmt, host_control, mmi_display, mmi_high};

/// Accepts `$wire` only if it round-trips byte-exactly.
macro_rules! assert_round_trips_or_rejects {
    ($ty:path, $wire:expr) => {{
        let wire: &[u8] = &$wire;
        if let Ok(v) = <$ty>::parse(wire) {
            let out = v.try_to_bytes().expect("a parsed value must re-serialize");
            assert_eq!(
                out,
                wire,
                "{} accepted {:02X?} but re-serialized {:02X?}",
                stringify!($ty),
                wire,
                out
            );
        }
    }};
}

/// r10-O-1 made fixed-layout APDUs reject a `length_field` that declares bytes
/// past the fixed body, but these fixed-layout objects were missed: they take
/// the first byte(s) and silently drop the rest (`9F 88 0B 02 01 FF` parses as
/// `menu_answ(choice_ref = 1)` and re-serializes as `9F 88 0B 01 01`).
#[test]
fn fixed_layout_apdus_missed_by_r10_o_1_drop_trailing_bytes() {
    assert_round_trips_or_rejects!(mmi_high::MenuAnsw, [0x9F, 0x88, 0x0B, 0x02, 0x01, 0xFF]);
    assert_round_trips_or_rejects!(mmi_display::Keypress, [0x9F, 0x88, 0x06, 0x02, 0x01, 0xFF]);
    assert_round_trips_or_rejects!(
        mmi_display::KeypadControl,
        [0x9F, 0x88, 0x05, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        mmi_display::DisplayMessage,
        [0x9F, 0x88, 0x10, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        mmi_display::SceneEndMark,
        [0x9F, 0x88, 0x11, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        mmi_display::SceneDoneMessage,
        [0x9F, 0x88, 0x12, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        mmi_display::SceneControl,
        [0x9F, 0x88, 0x13, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        host_control::ClearReplace,
        [0x9F, 0x84, 0x02, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        cicam_player::PlayerStop,
        [0x9F, 0xA0, 0x0B, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        cicam_player::PlayerEnd,
        [0x9F, 0xA0, 0x0C, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        service_gateway::ServiceListChanged,
        [0x9F, 0x80, 0x04, 0x02, 0x01, 0xFF]
    );
    assert_round_trips_or_rejects!(
        copy_protection::CpQuery,
        [
            0x9F, 0x80, 0x00, 0x81, 0x06, 0x79, 0xEA, 0x80, 0x15, 0x80, 0x9E
        ]
    );
}

/// Same defect at the session layer: an `open_session_request` (EN 50221
/// Table 8, `length_field = 4`) declaring 5 bytes is accepted and its 5th byte
/// dropped.
#[test]
fn open_session_request_drops_trailing_bytes() {
    assert_round_trips_or_rejects!(
        dvb_ci::spdu::OpenSessionRequest,
        [0x91, 0x05, 0x00, 0x01, 0x00, 0x41, 0xFF]
    );
}

/// Variable-layout objects drop whatever follows their last counted field:
/// `application_info` with an empty `menu_string` and one extra body byte
/// (what a newer-version CAM appends) re-serializes without it.
#[test]
fn application_info_drops_bytes_after_menu_string() {
    assert_round_trips_or_rejects!(
        application_info::ApplicationInfo,
        [
            0x9F, 0x80, 0x21, 0x07, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, 0xAA
        ]
    );
}

/// Accepted-then-zeroed bits: these parsers accept a non-zero value in a
/// field and re-serialize it as zero (the inputs are the sweep's own
/// findings), so the value a CAM sent is silently lost.
#[test]
fn accepted_bits_are_cleared_on_reserialize() {
    assert_round_trips_or_rejects!(
        content_control::CcPinReply,
        [0x9F, 0x90, 0x14, 0x03, 0x02, 0x8F, 0x3F]
    );
    assert_round_trips_or_rejects!(
        resource_manager_v2::ModuleIdSend,
        [0x9F, 0x80, 0x13, 0x81, 0x01, 0x67]
    );
    assert_round_trips_or_rejects!(
        broadcast_service_gateway::EitSectionReq,
        [
            0x9F, 0x80, 0x10, 0x82, 0x00, 0x08, 0x3C, 0x02, 0x23, 0xCE, 0xCA, 0xFF, 0x20, 0x42
        ]
    );
    assert_round_trips_or_rejects!(
        multistream_host_control::TuneLcnReq,
        [0x9F, 0x84, 0x07, 0x03, 0x02, 0x8F, 0x3F]
    );
}

/// Reserved bits are rewritten to 1 on serialize (dvb-si preserves them since
/// #1177): a `ca_pmt` with `reserved = 00` in the version byte and the
/// `program_info_length` word comes back as `C1`/`F0 00`.
#[test]
fn ca_pmt_reserved_bits_are_not_preserved() {
    assert_round_trips_or_rejects!(
        ca_pmt::CaPmt,
        [0x9F, 0x80, 0x32, 0x06, 0x03, 0x00, 0x01, 0x01, 0x00, 0x00]
    );
}

/// serialize -> parse must give back an equal value. A hand-built
/// `LengthForm::Long { octets: 1 }` for a body of >= 128 bytes is the minimal
/// coding (`81 80`), which `parse` records as `Minimal`, so the round trip
/// changes the value.
#[test]
fn long_form_one_octet_for_a_large_body_does_not_round_trip() {
    let info = dvb_ci::objects::ca_info::CaInfo {
        length_form: dvb_ci::length::LengthForm::Long { octets: 1 },
        ca_system_ids: (0..64u16).collect(),
    };
    let wire = info.try_to_bytes().unwrap();
    let back = dvb_ci::objects::ca_info::CaInfo::parse(&wire).unwrap();
    assert_eq!(back, info, "serialize -> parse changed length_form");
}

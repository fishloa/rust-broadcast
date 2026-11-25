//! `LiveJoin` decision, policy, VVC and leading-picture boundary gates
//! (issue #1162 R4; H.264 §7.4.1.2.4, H.265 Table 7-1, H.266 Table 5).
#![cfg(feature = "std")]
use transmux::{
    CodecConfig, DropReason, JoinDecision, JoinPolicy, LiveJoin, NalCodec, RapKind, Sample,
};

fn timed_au(nals: &[&[u8]], pts: i64) -> Sample {
    let mut data = Vec::new();
    for nal in nals {
        data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        data.extend_from_slice(nal);
    }
    Sample::new(data, Some(pts), Some(pts), Some(1), true).expect("valid timing")
}

fn vvc_config() -> CodecConfig {
    let bytes = test_fixtures::read("mp4/frag/vvc.frag.mp4");
    let media = test_harness::collect::media_from_mp4(
        bytes.as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux vvc fixture");
    media
        .tracks
        .iter()
        .find_map(|t| match t.config() {
            CodecConfig::Vvc { .. } => Some(t.config().clone()),
            _ => None,
        })
        .expect("fixture carries a VVC track")
}

/// A two-byte VVC NAL header carrying `nal_unit_type` (H.266 §7.3.1.1).
fn vvc_nal(nal_unit_type: u8) -> Vec<u8> {
    vec![0x00, nal_unit_type << 3]
}

#[test]
fn is_keep_is_false_only_for_drop() {
    assert!(!JoinDecision::Drop(DropReason::LeadingPicture).is_keep());
    assert!(!JoinDecision::Drop(DropReason::BeforeRandomAccessPoint).is_keep());
    assert!(JoinDecision::JoinPoint(RapKind::Idr).is_keep());
    assert!(JoinDecision::Keep.is_keep());
}

#[test]
fn the_policy_getter_returns_what_was_set() {
    let mut join = LiveJoin::new(NalCodec::Avc, 4);
    assert_eq!(join.policy(), JoinPolicy::AnyRap);
    join.set_policy(JoinPolicy::SelfContainedOnly);
    assert_eq!(join.policy(), JoinPolicy::SelfContainedOnly);
}

#[test]
fn a_vvc_track_joins_at_idr_and_cra() {
    let config = vvc_config();
    for (nal_type, expected) in [
        (7u8, RapKind::Idr), // VVC_IDR_W_RADL
        (8, RapKind::Idr),   // VVC_IDR_N_LP
        (9, RapKind::Cra),   // VVC_CRA
    ] {
        let mut join = LiveJoin::for_config(&config).expect("VVC config is joinable");
        let sample = timed_au(&[&vvc_nal(nal_type)], 100);
        assert_eq!(join.push(&sample), JoinDecision::JoinPoint(expected));
    }
}

#[test]
fn a_vvc_before_the_first_rap_is_dropped() {
    let config = vvc_config();
    let mut join = LiveJoin::for_config(&config).expect("VVC config is joinable");
    let sample = timed_au(&[&vvc_nal(1)], 100);
    let decision = join.push(&sample);
    assert_eq!(
        decision,
        JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
    );
    assert!(!decision.is_keep());
}

#[test]
fn an_avc_leading_picture_boundary_is_strictly_before_the_join_pts() {
    let idr = [0x65u8, 0x88];
    let inter = [0x41u8, 0x88];
    let mut join = LiveJoin::new(NalCodec::Avc, 4);
    assert_eq!(
        join.push(&timed_au(&[&idr], 100)),
        JoinDecision::JoinPoint(RapKind::Idr)
    );
    assert_eq!(
        join.push(&timed_au(&[&inter], 100)),
        JoinDecision::Keep,
        "pts == rap_pts is not a leading picture"
    );
    let mut join = LiveJoin::new(NalCodec::Avc, 4);
    join.push(&timed_au(&[&idr], 100));
    assert_eq!(
        join.push(&timed_au(&[&inter], 99)),
        JoinDecision::Drop(DropReason::LeadingPicture),
        "pts == rap_pts - 1 is a leading picture"
    );
}

#[test]
fn an_hevc_sample_with_a_parameter_set_and_rasl_is_still_leading() {
    let cra = [0x2Au8, 0x01];
    let vps = [0x40u8, 0x01];
    let rasl = [0x10u8, 0x01];
    let mut join = LiveJoin::new(NalCodec::Hevc, 4);
    assert_eq!(
        join.push(&timed_au(&[&cra], 100)),
        JoinDecision::JoinPoint(RapKind::Cra)
    );
    assert_eq!(
        join.push(&timed_au(&[&vps, &rasl], 90)),
        JoinDecision::Drop(DropReason::LeadingPicture),
        "a non-VCL NAL must not hide the RASL picture"
    );
}

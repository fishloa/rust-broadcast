use crate::device::{DeviceOp, MockCaDevice};
use crate::driver::Driver;
use crate::driver::tests::{TestClock, build_ca_pmt_fixture, driver_with_sessions, sb};
use crate::session::SessionLayer;
use crate::transport::{TcState, Transport, TransportError};
use broadcast_common::Parse;
use dvb_ci::spdu::{CloseSessionRequest, CloseSessionResponse, SessionStatus};
use dvb_ci::tpdu::{SbValue, tags};
use dvb_si::tables::pmt::PmtSection;
use std::time::Duration;

fn r_tpdu(tag: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![tag, (1 + data.len()) as u8, 1];
    v.extend_from_slice(data);
    v.extend_from_slice(&[tags::SB, 0x02, 1, SbValue::new(false).0]);
    v
}

fn active_transport() -> Transport {
    let mut t = Transport::new(1);
    t.init();
    t.on_frame(&[tags::C_T_C_REPLY, 0x01, 0x01, tags::SB, 0x02, 0x01, 0x00]);
    assert_eq!(t.state(), TcState::Active);
    t
}

#[test]
fn add_service_without_an_open_ca_session_must_not_report_success() {
    let mut d = Driver::new(MockCaDevice::new([]));
    let pmt_bytes = build_ca_pmt_fixture(1546);
    let pmt = PmtSection::parse(&pmt_bytes).unwrap();
    let result = d.add_service(&pmt);
    let wrote_ca_pmt = d.device().ops.iter().any(|op| match op {
        DeviceOp::Write(w) => w.windows(3).any(|x| x == [0x9F, 0x80, 0x32]),
        _ => false,
    });
    assert!(!wrote_ca_pmt, "precondition: nothing can have been sent");
    assert!(
        result.is_err() || d.managed_ca().services().is_empty(),
        "add_service returned Ok and recorded service 1546 although no ca_pmt reached the CAM: {:?}",
        d.managed_ca().services().keys().collect::<Vec<_>>()
    );
}

#[test]
fn stale_partial_chain_must_not_survive_a_reply_timeout() {
    let mut t = active_transport();
    let out = t.on_frame(&r_tpdu(tags::DATA_MORE, &[1, 2, 3]));
    assert_eq!(out.writes.len(), 1, "T_RCV for the next fragment");
    let out = t.tick(Duration::from_millis(1500));
    assert_eq!(out.error, Some(TransportError::ReplyTimeout));
    let out = t.on_frame(&r_tpdu(tags::DATA_LAST, &[9, 9]));
    assert_eq!(
        out.spdus,
        vec![vec![9u8, 9]],
        "a fresh message after the timed-out chain must not carry the stale fragment"
    );
}

#[test]
fn setup_timeout_is_followed_by_a_new_create_t_c() {
    let mut d = Driver::new(MockCaDevice::new([]));
    let clock = TestClock::new();
    d = d.with_clock(clock.as_fn());
    d.init().unwrap();
    for _ in 0..60 {
        clock.advance(Duration::from_millis(100));
        d.pump(Duration::from_millis(100)).unwrap();
    }
    let create_t_c = d
        .device()
        .ops
        .iter()
        .filter(|op| matches!(op, DeviceOp::Write(w) if w.first() == Some(&tags::CREATE_T_C)))
        .count();
    assert!(
        create_t_c > 1,
        "6 s of a silent CAM produced {create_t_c} Create_T_C; the transport sits in Idle after the first SetupTimeout"
    );
}

#[test]
fn closing_an_unallocated_session_is_answered_not_allocated() {
    let mut s = SessionLayer::new();
    let req = {
        use broadcast_common::Serialize;
        CloseSessionRequest {
            length_form: dvb_ci::length::LengthForm::Minimal,
            session_nb: 77,
        }
        .try_to_bytes()
        .unwrap()
    };
    let out = s.on_spdu(&req, |_| true).unwrap();
    let resp = CloseSessionResponse::parse(&out.spdus[0]).unwrap();
    assert_ne!(
        resp.status,
        SessionStatus::Ok,
        "close_session_request for a session_nb that was never allocated must be answered 0xF0"
    );
    assert!(out.closed.is_empty(), "no session was closed");
}

#[test]
fn a_busy_cam_must_not_starve_the_requery_timer() {
    let clock = TestClock::new();
    let mut d = driver_with_sessions().with_clock(clock.as_fn());
    let pmt_bytes = build_ca_pmt_fixture(1546);
    let pmt = PmtSection::parse(&pmt_bytes).unwrap();
    d.add_service(&pmt).unwrap();
    d.set_requery_interval(Duration::from_secs(1));
    d.pump(Duration::from_millis(10)).unwrap();
    let ca_pmt_writes = |d: &Driver<MockCaDevice>| {
        d.device()
            .ops
            .iter()
            .filter(|op| matches!(op, DeviceOp::Write(w) if w.windows(3).any(|x| x == [0x9F, 0x80, 0x32])))
            .count()
    };
    let before = ca_pmt_writes(&d);
    for _ in 0..8 {
        clock.advance(Duration::from_millis(500));
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
    }
    assert!(
        ca_pmt_writes(&d) > before,
        "4 s passed with a 1 s requery interval but no re-query was sent: elapsed time of every readable pump is discarded"
    );
}

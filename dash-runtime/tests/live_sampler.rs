//! The `UTCTiming` candidate order and sampling rules, observed through the
//! client's own actions (DESIGN 3.3).

mod common;

use common::scenario::*;
use common::synth::{Addressing, MPD_URL, Synth};
use common::world::{MICROS, xs_datetime_ms};
use dash_runtime::client::{Action, DashClient, DashConfig, PullLimits, UtcMethod, UtcResponse};
use dash_runtime::timing::utc_timing::UtcScheme;

fn element(scheme: &str, value: &str) -> String {
    format!("<UTCTiming schemeIdUri=\"urn:mpeg:dash:utc:{scheme}:2014\" value=\"{value}\"/>")
}

fn client_with(elements: &[String], direct: Option<i64>) -> (DashClient, Vec<Action>) {
    let mut synth = Synth::new(AST_US, Addressing::Number);
    synth.utc_elements = elements.to_vec();
    if let Some(at) = direct {
        synth.utc_direct = true;
        synth.utc.skew_us = at;
    }
    let text = synth.mpd_at(AST_US + 41 * MICROS);
    let mut c = DashClient::new(MPD_URL, DashConfig::default(), PullLimits::default());
    assert!(matches!(c.poll(0), Some(Action::FetchMpd { .. })));
    c.on_mpd(0, text.as_bytes()).unwrap();
    let mut actions = Vec::new();
    while let Some(a) = c.poll(0) {
        actions.push(a);
        if actions.len() > 8 {
            break;
        }
    }
    (c, actions)
}

fn utc_urls(actions: &[Action]) -> Vec<(UtcScheme, String, UtcMethod)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchUtcTiming {
                scheme,
                url,
                method,
                ..
            } => Some((*scheme, url.clone(), *method)),
            _ => None,
        })
        .collect()
}

fn utc_id(actions: &[Action]) -> dash_runtime::client::UtcId {
    actions
        .iter()
        .find_map(|a| match a {
            Action::FetchUtcTiming { id, .. } => Some(*id),
            _ => None,
        })
        .expect("a UTC request")
}

fn drain(c: &mut DashClient) -> Vec<Action> {
    let mut v = Vec::new();
    while let Some(a) = c.poll(0) {
        v.push(a);
    }
    v
}

#[test]
fn http_head_is_tried_after_the_other_http_schemes() {
    let els = [
        element("http-head", "http://utc.test/head"),
        element("http-xsdate", "http://utc.test/xsdate"),
        element("http-iso", "http://utc.test/iso"),
    ];
    let (mut c, mut actions) = client_with(&els, None);
    let mut order = Vec::new();
    for _ in 0..3 {
        let seen = utc_urls(&actions);
        assert_eq!(seen.len(), 1, "one candidate at a time");
        order.push((seen[0].1.clone(), seen[0].2));
        c.on_utc_failure(0, utc_id(&actions), dash_runtime::client::Failure::Status(500))
            .unwrap();
        actions = drain(&mut c);
    }
    assert_eq!(
        order,
        vec![
            ("http://utc.test/xsdate".to_owned(), UtcMethod::Get),
            ("http://utc.test/iso".to_owned(), UtcMethod::Get),
            ("http://utc.test/head".to_owned(), UtcMethod::Head),
        ]
    );
}

#[test]
fn a_value_with_several_urls_offers_each_in_order() {
    let els = [element("http-xsdate", "http://utc.test/one http://utc.test/two")];
    let (mut c, first) = client_with(&els, None);
    let a = utc_urls(&first);
    assert_eq!(a[0].1, "http://utc.test/one");
    let id = first
        .iter()
        .find_map(|a| match a {
            Action::FetchUtcTiming { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    c.on_utc_failure(0, id, dash_runtime::client::Failure::Transport).unwrap();
    let mut next = Vec::new();
    while let Some(a) = c.poll(0) {
        next.push(a);
    }
    assert_eq!(utc_urls(&next)[0].1, "http://utc.test/two");
}

#[test]
fn non_http_urls_are_never_requested() {
    let els = [
        element("http-xsdate", "ftp://utc.test/time"),
        element("http-xsdate", "http://utc.test/ok"),
    ];
    let (_, first) = client_with(&els, None);
    assert_eq!(utc_urls(&first)[0].1, "http://utc.test/ok");
}

#[test]
fn a_valid_direct_value_anchors_without_any_request() {
    let (c, first) = client_with(&[], Some(0));
    assert!(utc_urls(&first).is_empty());
    let anchor = c.clock_anchor().unwrap();
    assert_eq!(anchor.source, dash_runtime::timing::clock::ClockSource::UtcTiming);
    assert_eq!(anchor.utc_us, AST_US + 41 * MICROS);
}

#[test]
fn an_unreadable_direct_value_falls_through_to_the_next_candidate() {
    let els = [
        "<UTCTiming schemeIdUri=\"urn:mpeg:dash:utc:direct:2014\" value=\"not-a-time\"/>".to_owned(),
        element("http-xsdate", "http://utc.test/after"),
    ];
    let (_, first) = client_with(&els, None);
    assert_eq!(utc_urls(&first)[0].1, "http://utc.test/after");
}

#[test]
fn an_answer_for_an_unknown_id_is_an_error_not_a_panic() {
    let els = [element("http-xsdate", "http://utc.test/xsdate")];
    let (mut c, first) = client_with(&els, None);
    let id = first
        .iter()
        .find_map(|a| match a {
            Action::FetchUtcTiming { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let body = xs_datetime_ms(AST_US + 41 * MICROS).into_bytes();
    let r = UtcResponse::new(0, 0, &body);
    c.on_utc_response(0, id, &r).unwrap();
    assert!(c.on_utc_response(0, id, &r).is_err(), "answered twice");
    assert!(c.on_utc_failure(0, id, dash_runtime::client::Failure::Timeout).is_err());
}

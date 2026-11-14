use broadcast_auth::{AuthResult, RequestContext, Verifier};
use broadcast_client::{Action, Config, ConnId, Credentials, Session};
use broadcast_common::stage::Timestamp;
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const TS_URL: &str = "http://cam.local:8080/stream/live.ts?ch=1&q=hd";
const ORIGIN_FORM: &str = "/stream/live.ts?ch=1&q=hd";
const REALM: &str = "cam";

fn first_request(session: &mut Session, now: u64) -> (ConnId, Vec<(String, String)>) {
    let mut found = None;
    while let Some(action) = session.poll(ts(now)) {
        if let Action::HttpRequest { conn, headers, .. } = action {
            found = Some((
                conn,
                headers
                    .iter()
                    .map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect(),
            ));
        }
    }
    found.expect("an HttpRequest")
}

fn authorization(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.clone())
        .expect("Authorization header")
}

fn digest_field(header: &str, name: &str) -> String {
    let needle = format!("{name}=\"");
    let start = header
        .find(&needle)
        .unwrap_or_else(|| panic!("no {name} in {header}"))
        + needle.len();
    let end = header[start..].find('"').unwrap() + start;
    header[start..end].to_string()
}

#[test]
fn digest_uri_is_the_origin_form_request_target_for_http() {
    let verifier =
        Verifier::new(Credentials::new("user", "pass"), REALM).expect("OS random source");
    let mut session = Session::open(Url::parse(TS_URL).unwrap(), Config::default(), ts(0)).unwrap();
    session
        .set_credentials(ts(0), Credentials::new("user", "pass"))
        .unwrap();
    let (conn, _) = first_request(&mut session, 0);
    let challenge = verifier.challenge();
    session.on_response(
        conn,
        401,
        &[("WWW-Authenticate".to_string(), challenge)],
        ts(0),
    );
    let (_, headers) = first_request(&mut session, 0);
    let header = authorization(&headers);
    assert!(header.starts_with("Digest "), "{header}");
    assert_eq!(
        digest_field(&header, "uri"),
        ORIGIN_FORM,
        "RFC 7616 3.4: uri is the request-target, not the absolute URL"
    );
    let raw: &[(&str, &str)] = &[("authorization", header.as_str())];
    let ctx = RequestContext::new("GET", ORIGIN_FORM).with_headers(raw);
    assert_eq!(verifier.verify(&ctx), AuthResult::Ok);
}

const SECOND_US: u64 = 1_000_000;

#[test]
fn cookie_max_age_counts_from_the_clock_of_the_response() {
    let mut session = Session::open(
        Url::parse("http://example/live.ts").unwrap(),
        Config::default(),
        ts(0),
    )
    .unwrap();
    let (conn, _) = first_request(&mut session, 0);
    session.on_response(
        conn,
        200,
        &[("Set-Cookie".to_string(), "sid=abc; Max-Age=60".to_string())],
        ts(100 * SECOND_US),
    );
    session.reconnect(ts(130 * SECOND_US));
    let (_, headers) = first_request(&mut session, 130 * SECOND_US);
    let cookie = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v.as_str());
    assert_eq!(
        cookie,
        Some("sid=abc"),
        "set at 100 s with Max-Age=60 it is still valid at 130 s"
    );
}

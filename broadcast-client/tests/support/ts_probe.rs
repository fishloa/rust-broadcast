use broadcast_client::{Action, Config, ConnId, Session};
use broadcast_common::stage::Timestamp;
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

pub const TS_URL: &str = "http://example/live.ts";

#[derive(Debug, Clone)]
pub struct Req {
    pub now: u64,
    pub conn: ConnId,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn count(&self, name: &str) -> usize {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .count()
    }

    pub fn cookie(&self) -> Option<&str> {
        self.header("cookie")
    }
}

fn entry(name: http::HeaderName, value: &str) -> (http::HeaderName, http::HeaderValue) {
    (
        name,
        http::HeaderValue::from_str(value).expect("a test writes a valid header value"),
    )
}

pub fn set_cookie(line: &str) -> (http::HeaderName, http::HeaderValue) {
    entry(http::header::SET_COOKIE, line)
}

pub fn set_cookies(lines: &[&str]) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    for line in lines {
        let (name, value) = set_cookie(line);
        map.append(name, value);
    }
    map
}

pub fn location(url: &str) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    let (name, value) = entry(http::header::LOCATION, url);
    map.append(name, value);
    map
}

pub fn requests(session: &mut Session, now: u64) -> Vec<Req> {
    let mut out = Vec::new();
    while let Some(a) = session.poll(ts(now)) {
        if let Action::HttpRequest {
            conn, url, headers, ..
        } = a
        {
            out.push(Req {
                now,
                conn,
                url: url.to_string(),
                headers: headers
                    .iter()
                    .map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect(),
            });
        }
    }
    out
}

pub fn one(session: &mut Session, now: u64) -> Req {
    let mut reqs = requests(session, now);
    assert_eq!(reqs.len(), 1, "expected exactly one request: {reqs:?}");
    reqs.remove(0)
}

pub fn open(url: &str, config: Config) -> Session {
    Session::open(Url::parse(url).unwrap(), config, ts(0)).expect("open")
}

pub fn respond(
    session: &mut Session,
    req: &Req,
    status: http::StatusCode,
    headers: &http::HeaderMap,
) {
    session.on_response(req.conn, status, headers, ts(req.now));
}

pub fn ok_with_cookies(session: &mut Session, req: &Req, lines: &[&str]) {
    respond(session, req, http::StatusCode::OK, &set_cookies(lines));
}

pub fn reconnect(session: &mut Session, now: u64) -> Req {
    session.reconnect(ts(now));
    one(session, now)
}

pub fn redirect_probe(start_url: &str, lines: &[&str], target: &str) -> Req {
    let mut session = open(start_url, Config::default());
    let first = one(&mut session, 0);
    let mut headers = set_cookies(lines);
    headers.extend(location(target));
    respond(&mut session, &first, http::StatusCode::FOUND, &headers);
    one(&mut session, 0)
}

use broadcast_client::{Action, Config, Session};

pub const TS_URL: &str = "http://example/live.ts";

#[derive(Debug, Clone)]
pub struct Req {
    pub now: u64,
    pub conn: u32,
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

pub fn set_cookie(line: &str) -> (String, String) {
    ("Set-Cookie".to_string(), line.to_string())
}

pub fn set_cookies(lines: &[&str]) -> Vec<(String, String)> {
    lines.iter().map(|l| set_cookie(l)).collect()
}

pub fn location(url: &str) -> (String, String) {
    ("Location".to_string(), url.to_string())
}

pub fn requests(session: &mut Session, now: u64) -> Vec<Req> {
    let mut out = Vec::new();
    while let Some(a) = session.poll(now) {
        if let Action::HttpRequest {
            conn, url, headers, ..
        } = a
        {
            out.push(Req {
                now,
                conn,
                url,
                headers,
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
    Session::open(url, config, 0).expect("open")
}

pub fn respond(session: &mut Session, req: &Req, status: u16, headers: &[(String, String)]) {
    session.on_response(req.conn, status, headers, req.now);
}

pub fn ok_with_cookies(session: &mut Session, req: &Req, lines: &[&str]) {
    respond(session, req, 200, &set_cookies(lines));
}

pub fn reconnect(session: &mut Session, now: u64) -> Req {
    session.reconnect(now);
    one(session, now)
}

pub fn redirect_probe(start_url: &str, lines: &[&str], target: &str) -> Req {
    let mut session = open(start_url, Config::default());
    let first = one(&mut session, 0);
    let mut headers = set_cookies(lines);
    headers.push(location(target));
    respond(&mut session, &first, 302, &headers);
    one(&mut session, 0)
}

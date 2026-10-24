use alloc::string::{String, ToString};
use alloc::vec::Vec;

use url::Url;

use super::{CookieJar, legacy::CookieJar as LegacyJar};

const SECOND_US: u64 = 1_000_000;
const MINUTE_US: u64 = 60 * SECOND_US;

struct Case {
    origin: &'static str,
    response: &'static str,
    line: &'static str,
    probe: &'static str,
}

const fn case(
    origin: &'static str,
    response: &'static str,
    line: &'static str,
    probe: &'static str,
) -> Case {
    Case {
        origin,
        response,
        line,
        probe,
    }
}

const EXPIRES_COMMA: &str = "sid=1; Path=/; Expires=Wed, 21 Oct 2037 07:28:00 GMT";
const EXPIRES_PAST: &str = "sid=1; Path=/; Expires=Thu, 01 Jan 1970 00:00:01 GMT";

const CASES: &[Case] = &[
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Path=/",
        "http://example.com/x",
    ),
    case(
        "http://example.com/",
        "http://example.com/live/a.ts",
        "a=1",
        "http://example.com/live/b.ts",
    ),
    case(
        "http://example.com/",
        "http://example.com/live/a.ts",
        "a=1",
        "http://example.com/other",
    ),
    case(
        "http://example.com/",
        "http://example.com/live/",
        "a=1",
        "http://example.com/live",
    ),
    case(
        "http://example.com/",
        "http://example.com/live/",
        "a=1",
        "http://example.com/live/x/y",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b/c",
        "a=1",
        "http://example.com/a/b/d",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b/c",
        "a=1",
        "http://example.com/a/x",
    ),
    case(
        "http://example.com/",
        "http://example.com//a/b",
        "a=1",
        "http://example.com//a/z",
    ),
    case(
        "http://example.com/",
        "http://example.com/a//",
        "a=1",
        "http://example.com/a//x",
    ),
    case(
        "http://example.com/",
        "http://example.com/a",
        "a=1; Path=",
        "http://example.com/zzz",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b",
        "a=1; Path=rel",
        "http://example.com/a/c",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b",
        "a=1; Path=/a",
        "http://example.com/a",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b",
        "a=1; Path=/a",
        "http://example.com/ab",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b",
        "a=1; Path=/a",
        "http://example.com/a/b/c",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b",
        "a=1; Path=/a/",
        "http://example.com/a",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b",
        "a=1; Path=/a/",
        "http://example.com/a/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Path=/Case",
        "http://example.com/case",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Path=/Case",
        "http://example.com/Case",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Path=/%41",
        "http://example.com/%41",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Path=/é",
        "http://example.com/é",
    ),
    case(
        "http://example.com/",
        "http://example.com/?q=1",
        "a=1",
        "http://example.com/x?q=1",
    ),
    case(
        "http://example.com/",
        "http://example.com/a/b?x=/y/z",
        "a=1",
        "http://example.com/a/q",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        EXPIRES_COMMA,
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        EXPIRES_PAST,
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "sid=1; Expires=Wed, 21 Oct 2037 07:28:00 GMT; Path=/p",
        "http://example.com/p",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "sid=1; Max-Age=60; Path=/",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "sid=1; Max-Age=0; Path=/",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "sid=1; Max-Age=-5",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "sid=1; Max-Age=abc",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "sid=1; Secure",
        "http://example.com/",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "sid=1; Secure",
        "https://example.com/",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "sid=1; Secure; HttpOnly; SameSite=Lax",
        "https://example.com/x",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "__Secure-s=1; Secure",
        "https://example.com/",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "__Secure-s=1",
        "https://example.com/",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "__Host-h=1; Secure; Path=/",
        "https://example.com/",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "__Host-h=1; Secure; Path=/; Domain=example.com",
        "https://example.com/",
    ),
    case(
        "https://example.com/",
        "https://example.com/",
        "__Host-h=1; Secure; Path=/x",
        "https://example.com/x",
    ),
    case(
        "http://example.com/",
        "http://other.example.com/",
        "a=1",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com:8080/",
        "a=1",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "https://example.com/",
        "a=1",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1",
        "http://example.com:8080/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1",
        "http://other.com/",
    ),
    case(
        "http://example.com:8080/",
        "http://example.com:8080/",
        "a=1",
        "http://example.com:8080/x",
    ),
    case(
        "http://example.com:8080/",
        "http://example.com:8080/",
        "a=1",
        "http://example.com:9090/x",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=example.com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=.example.com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=EXAMPLE.COM",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=Example.Com.",
        "http://example.com/",
    ),
    case(
        "http://www.example.com/",
        "http://www.example.com/",
        "a=1; Domain=example.com",
        "http://www.example.com/",
    ),
    case(
        "http://www.example.com/",
        "http://www.example.com/",
        "a=1; Domain=.example.com",
        "http://www.example.com/p",
    ),
    case(
        "http://a.b.example.com/",
        "http://a.b.example.com/",
        "a=1; Domain=b.example.com",
        "http://a.b.example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=www.example.com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=other.com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=ample.com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=.com",
        "http://example.com/",
    ),
    case(
        "http://example.co.uk/",
        "http://example.co.uk/",
        "a=1; Domain=co.uk",
        "http://example.co.uk/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=.",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=exa mple.com",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "a=1; Domain=xn--invalid-.com",
        "http://example.com/",
    ),
    case(
        "http://127.0.0.1/",
        "http://127.0.0.1/",
        "a=1",
        "http://127.0.0.1/",
    ),
    case(
        "http://127.0.0.1/",
        "http://127.0.0.1/",
        "a=1; Domain=127.0.0.1",
        "http://127.0.0.1/",
    ),
    case(
        "http://127.0.0.1/",
        "http://127.0.0.1/",
        "a=1; Domain=.0.0.1",
        "http://127.0.0.1/",
    ),
    case(
        "http://127.0.0.1/",
        "http://127.0.0.1/",
        "a=1; Domain=0.0.1",
        "http://127.0.0.1/",
    ),
    case(
        "http://[::1]:8080/",
        "http://[::1]:8080/",
        "a=1",
        "http://[::1]:8080/",
    ),
    case(
        "http://[::1]:8080/",
        "http://[::1]:8080/",
        "a=1; Domain=::1",
        "http://[::1]:8080/",
    ),
    case(
        "http://localhost/",
        "http://localhost/",
        "a=1; Domain=localhost",
        "http://localhost/",
    ),
    case(
        "http://localhost/",
        "http://localhost/",
        "a=1",
        "http://localhost/",
    ),
    case(
        "http://bücher.example/",
        "http://bücher.example/",
        "a=1; Domain=bücher.example",
        "http://bücher.example/",
    ),
    case(
        "http://bücher.example/",
        "http://bücher.example/",
        "a=1; Domain=xn--bcher-kva.example",
        "http://bücher.example/",
    ),
    case(
        "rtsp://cam.example/",
        "rtsp://cam.example/",
        "a=1",
        "rtsp://cam.example/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "bad line without equals",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        "=novalue",
        "http://example.com/",
    ),
    case(
        "http://example.com/",
        "http://example.com/",
        " sp = v ; Path=/",
        "http://example.com/",
    ),
];

fn parse(url: &str) -> Url {
    Url::parse(url).unwrap()
}

fn headers(line: &str) -> Vec<(String, String)> {
    alloc::vec![("Set-Cookie".to_string(), line.to_string())]
}

fn run(c: &Case) -> (Option<String>, Option<String>) {
    let origin = parse(c.origin);
    let response = parse(c.response);
    let probe = parse(c.probe);
    let mut old = LegacyJar::new(origin.origin());
    let mut new = CookieJar::new(&origin);
    old.receive(&response, &headers(c.line), 0);
    new.receive(&response, &headers(c.line), 0);
    (
        old.header_for(&probe, MINUTE_US / 2),
        new.header_for(&probe, MINUTE_US / 2),
    )
}

struct Decision {
    line: &'static str,
    origin: &'static str,
    old: Option<&'static str>,
    new: Option<&'static str>,
}

const DECIDED: &[Decision] = &[
    Decision {
        line: "a=1; Domain=Example.Com.",
        origin: "http://example.com/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=www.example.com",
        origin: "http://example.com/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=other.com",
        origin: "http://example.com/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=ample.com",
        origin: "http://example.com/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=exa mple.com",
        origin: "http://example.com/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=xn--invalid-.com",
        origin: "http://example.com/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=.0.0.1",
        origin: "http://127.0.0.1/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=0.0.1",
        origin: "http://127.0.0.1/",
        old: Some("a=1"),
        new: None,
    },
    Decision {
        line: "a=1; Domain=::1",
        origin: "http://[::1]:8080/",
        old: Some("a=1"),
        new: None,
    },
];

#[test]
fn table_has_at_least_sixty_cases() {
    assert!(CASES.len() >= 60, "{}", CASES.len());
}

#[test]
fn old_and_new_agree_except_for_decided_differences() {
    let mut undecided = Vec::new();
    for c in CASES {
        let (old, new) = run(c);
        if old == new {
            continue;
        }
        let decided = DECIDED.iter().any(|d| {
            d.line == c.line
                && d.origin == c.origin
                && d.old.map(str::to_string) == old
                && d.new.map(str::to_string) == new
        });
        if !decided {
            undecided.push((c.origin, c.line, old, new));
        }
    }
    assert!(undecided.is_empty(), "{undecided:#?}");
}

#[test]
fn every_decision_is_exercised_and_really_differs() {
    for d in DECIDED {
        let c = CASES
            .iter()
            .find(|c| c.line == d.line && c.origin == d.origin)
            .expect("decision names a case");
        let (old, new) = run(c);
        assert_eq!(old.as_deref(), d.old, "{}", d.line);
        assert_eq!(new.as_deref(), d.new, "{}", d.line);
        assert_ne!(old, new, "{}", d.line);
    }
}

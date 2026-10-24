use alloc::string::ToString;

use url::Url;

use super::CookieJar;

const SECOND_US: u64 = 1_000_000;
const FAR_CLOCK_US: u64 = u64::MAX / 2;
const CLOCKS: [u64; 3] = [0, SECOND_US * 86_400 * 365 * 40, FAR_CLOCK_US];
const PAST_EXPIRES: &str = "Expires=Thu, 01 Jan 1970 00:00:01 GMT";
const FUTURE_EXPIRES: &str = "Expires=Fri, 01 Jan 2100 00:00:00 GMT";

fn jar_with(line: &str, base: u64) -> (CookieJar, Url) {
    let url = Url::parse("http://example.com/").unwrap();
    let mut jar = CookieJar::new(&url);
    jar.receive(&url, &[("Set-Cookie".to_string(), line.to_string())], base);
    (jar, url)
}

#[test]
fn max_age_follows_the_injected_clock_at_any_base() {
    for base in CLOCKS {
        let (mut jar, url) = jar_with("a=1; Max-Age=60", base);
        assert_eq!(
            jar.header_for(&url, base + 59 * SECOND_US).as_deref(),
            Some("a=1"),
            "base {base}"
        );
        assert_eq!(
            jar.header_for(&url, base + 60 * SECOND_US),
            None,
            "base {base}"
        );
    }
}

#[test]
fn expires_is_ignored_whatever_the_wall_clock_or_session_clock_says() {
    for base in CLOCKS {
        for expires in [PAST_EXPIRES, FUTURE_EXPIRES] {
            let (mut jar, url) = jar_with(&alloc::format!("a=1; {expires}"), base);
            assert_eq!(
                jar.header_for(&url, base + 1000 * SECOND_US).as_deref(),
                Some("a=1"),
                "base {base} {expires}"
            );
        }
    }
}

#[test]
fn max_age_wins_over_a_far_future_expires() {
    for base in CLOCKS {
        let (mut jar, url) = jar_with(&alloc::format!("a=1; {FUTURE_EXPIRES}; Max-Age=1"), base);
        assert_eq!(jar.header_for(&url, base + SECOND_US), None, "base {base}");
    }
}

#[test]
fn max_age_zero_deletes_at_any_base() {
    for base in CLOCKS {
        let (mut jar, url) = jar_with("a=1", base);
        jar.receive(
            &url,
            &[("Set-Cookie".to_string(), "a=1; Max-Age=0".to_string())],
            base,
        );
        assert_eq!(jar.header_for(&url, base), None, "base {base}");
    }
}

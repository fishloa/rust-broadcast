//! Cookie jar scoped to the session origin (RFC 6265 sections 5.1.3, 5.1.4, 5.2, 5.3, 5.4).
//!
//! `Set-Cookie` parsing is the `cookie` crate's; domain-match and path-match are
//! `cookie_store`'s pure `CookieDomain`/`CookiePath`; expiry runs on the session's injected
//! microsecond clock. `Expires` is ignored; see `docs/cookies.md` for the documented deviations.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use cookie::Cookie;
use cookie_store::{CookieDomain, CookiePath};
use url::{Origin, Url};

const MAX_COOKIE_BYTES: usize = 4096;
const MAX_COOKIES: usize = 50;
const MAX_PATH_BYTES: usize = 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const SECURE_PREFIX: &str = "__secure-";
const HOST_PREFIX: &str = "__host-";
const MICROS_PER_SEC: u64 = 1_000_000;
const SET_COOKIE: &str = "set-cookie";
const ROOT_PATH: &str = "/";
const HTTPS: &str = "https";
const PAIR_SEPARATOR: &str = "; ";

#[derive(Clone)]
struct Stored {
    name: String,
    value: String,
    domain: CookieDomain,
    path: CookiePath,
    secure: bool,
    expires_us: Option<u64>,
}

impl Stored {
    fn expired_at(&self, now_us: u64) -> bool {
        self.expires_us.is_some_and(|at| now_us >= at)
    }
}

pub(crate) struct CookieJar {
    origin: Origin,
    host_only: CookieDomain,
    root: CookiePath,
    cookies: Vec<Stored>,
}

impl CookieJar {
    pub(crate) fn new(origin: &Url) -> Self {
        Self {
            origin: origin.origin(),
            host_only: CookieDomain::host_only(origin).unwrap_or(CookieDomain::NotPresent),
            root: CookiePath::new(ROOT_PATH, origin),
            cookies: Vec::new(),
        }
    }

    pub(crate) fn seed(&mut self, pairs: &[(String, String)]) {
        let https = matches!(&self.origin, Origin::Tuple(scheme, _, _) if scheme == HTTPS);
        for (name, value) in pairs {
            if name.len().saturating_add(value.len()) > MAX_COOKIE_BYTES {
                continue;
            }
            let prefixed = has_prefix(name, SECURE_PREFIX) || has_prefix(name, HOST_PREFIX);
            if prefixed && !https {
                continue;
            }
            self.insert(Stored {
                name: name.clone(),
                value: value.clone(),
                domain: self.host_only.clone(),
                path: self.root.clone(),
                secure: prefixed,
                expires_us: None,
            });
        }
    }

    pub(crate) fn receive(&mut self, url: &Url, headers: &[(String, String)], now_us: u64) {
        if url.origin() != self.origin {
            return;
        }
        for (_, line) in headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(SET_COOKIE))
        {
            self.receive_line(url, line, now_us);
        }
    }

    fn receive_line(&mut self, url: &Url, line: &str, now_us: u64) {
        let Ok(parsed) = Cookie::parse(line) else {
            return;
        };
        let (name, value) = parsed.name_value_trimmed();
        if name.len().saturating_add(value.len()) > MAX_COOKIE_BYTES {
            return;
        }
        let secure = parsed.secure().unwrap_or(false);
        if secure && url.scheme() != HTTPS {
            return;
        }
        if has_prefix(name, SECURE_PREFIX) && !secure {
            return;
        }
        if has_prefix(name, HOST_PREFIX)
            && !(secure && parsed.path() == Some(ROOT_PATH) && parsed.domain().is_none())
        {
            return;
        }
        let domain = match CookieDomain::try_from(&parsed) {
            Ok(domain @ CookieDomain::Suffix(_)) if domain.matches(url) => domain,
            Ok(CookieDomain::NotPresent | CookieDomain::Empty) => self.host_only.clone(),
            _ => return,
        };
        let path = CookiePath::new(parsed.path().unwrap_or_default(), url);
        if path.len() > MAX_PATH_BYTES {
            return;
        }
        let expires_us = match parsed.max_age() {
            Some(age) => {
                let secs = age.whole_seconds();
                if secs <= 0 {
                    self.remove(name, &path);
                    return;
                }
                let delta = u64::try_from(secs)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(MICROS_PER_SEC);
                Some(now_us.saturating_add(delta))
            }
            None => None,
        };
        self.insert(Stored {
            name: name.to_string(),
            value: value.to_string(),
            domain,
            path,
            secure,
            expires_us,
        });
    }

    fn remove(&mut self, name: &str, path: &CookiePath) {
        self.cookies
            .retain(|c| !(c.name == name && c.path == *path));
    }

    fn insert(&mut self, cookie: Stored) {
        if let Some(slot) = self
            .cookies
            .iter_mut()
            .find(|c| c.name == cookie.name && c.path == cookie.path)
        {
            *slot = cookie;
            return;
        }
        self.cookies.push(cookie);
        if self.cookies.len() > MAX_COOKIES {
            self.cookies.remove(0);
        }
    }

    pub(crate) fn header_for(&mut self, url: &Url, now_us: u64) -> Option<String> {
        if url.origin() != self.origin {
            return None;
        }
        self.cookies.retain(|c| !c.expired_at(now_us));
        let secure = url.scheme() == HTTPS;
        let mut matching: Vec<&Stored> = self
            .cookies
            .iter()
            .filter(|c| (secure || !c.secure) && c.domain.matches(url) && c.path.matches(url))
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| core::cmp::Reverse(c.path.len()));
        let mut header = String::new();
        for cookie in matching {
            let pair = format!("{}={}", cookie.name, cookie.value);
            let separator = if header.is_empty() {
                0
            } else {
                PAIR_SEPARATOR.len()
            };
            if header
                .len()
                .saturating_add(separator)
                .saturating_add(pair.len())
                > MAX_HEADER_BYTES
            {
                continue;
            }
            if separator > 0 {
                header.push_str(PAIR_SEPARATOR);
            }
            header.push_str(&pair);
        }
        (!header.is_empty()).then_some(header)
    }

    pub(crate) fn export(&self, now_us: u64) -> Vec<(String, String)> {
        self.cookies
            .iter()
            .filter(|c| !c.expired_at(now_us))
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect()
    }
}

impl core::fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CookieJar")
            .field("cookies", &self.cookies.len())
            .finish_non_exhaustive()
    }
}

fn has_prefix(name: &str, prefix: &str) -> bool {
    name.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

#[cfg(test)]
mod clock;
#[cfg(test)]
mod differential;
#[cfg(test)]
mod legacy;
#[cfg(test)]
mod sibling;

//! Validation of caller-supplied request headers.

use http::{HeaderName, HeaderValue};

use crate::Credentials;
use crate::error::Error;

const COOKIE_NAME_SPECIALS: &str = "!#$%&'*+-.^_`|~";

const RESERVED: [&str; 7] = [
    "authorization",
    "cookie",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "range",
];

pub(crate) fn is_reserved(name: &str) -> bool {
    RESERVED.iter().any(|r| name.eq_ignore_ascii_case(r))
}

pub(crate) fn validate(headers: &[(String, String)]) -> Result<(), Error> {
    for (name, value) in headers {
        let invalid = || Error::InvalidHeader { name: name.clone() };
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?;
        HeaderValue::from_str(value).map_err(|_| invalid())?;
        if is_reserved(name) {
            return Err(Error::ReservedHeader { name: name.clone() });
        }
    }
    Ok(())
}

pub(crate) fn validate_credentials(creds: &Credentials) -> Result<(), Error> {
    let clean = |s: &str| !s.chars().any(char::is_control);
    let valid = match creds {
        Credentials::Bearer { token } => {
            clean(token) && HeaderValue::from_str(&format!("Bearer {token}")).is_ok()
        }
        Credentials::Basic { username, password } | Credentials::Digest { username, password } => {
            clean(username) && clean(password)
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidCredentials)
    }
}

fn is_cookie_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || COOKIE_NAME_SPECIALS.contains(c)
}

pub(crate) fn validate_cookies(cookies: &[(String, String)]) -> Result<(), Error> {
    for (name, value) in cookies {
        let bad_name = name.is_empty() || !name.chars().all(is_cookie_name_char);
        let bad_value = value.contains(';') || HeaderValue::from_str(value).is_err();
        if bad_name || bad_value {
            return Err(Error::InvalidHeader { name: name.clone() });
        }
    }
    Ok(())
}

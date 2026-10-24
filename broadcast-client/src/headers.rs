//! Validation of caller-supplied request headers.

use http::{HeaderName, HeaderValue};

use crate::error::Error;

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

pub(crate) fn validate_cookies(cookies: &[(String, String)]) -> Result<(), Error> {
    for (name, value) in cookies {
        let bad_name = name.is_empty()
            || name
                .chars()
                .any(|c| c == '=' || c == ';' || c.is_whitespace());
        let bad_value = value.contains(';') || HeaderValue::from_str(value).is_err();
        if bad_name || bad_value {
            return Err(Error::InvalidHeader { name: name.clone() });
        }
    }
    Ok(())
}

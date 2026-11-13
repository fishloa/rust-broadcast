//! URL userinfo (RFC 3986 §3.2.1) as [`Credentials`], and its removal.
//!
//! A `scheme://user:pass@host/` URL carries credentials percent-encoded
//! (RFC 3986 §2.1). [`credentials_from_url`] decodes them and
//! [`strip_userinfo`] yields the same URL without them, safe to log or put on
//! a request line.

use url::Url;

use crate::{Credentials, Error, Result};

/// Credentials carried in `url`'s userinfo, percent-decoded to UTF-8.
///
/// `Ok(None)` when the URL has no userinfo at all (the common case: connect
/// without auth). An empty username with a password (`scheme://:pw@host/`) is
/// credentials, not "no credentials": RFC 3986 §3.2.1 makes the userinfo
/// `user ":" password` with an empty user legal, and RFC 7617 §2 (`user-id` may
/// be empty) and RFC 7616 §3.4 (`username=""`) both carry it. A missing
/// password decodes as the empty string. The error never echoes the offending
/// component, which is a still-encoded credential.
pub fn credentials_from_url(url: &Url) -> Result<Option<Credentials>> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let username = decode(url.username())?;
    let password = match url.password() {
        Some(p) => decode(p)?,
        None => String::new(),
    };
    Ok(Some(Credentials::new(username, password)))
}

/// A copy of `url` with its username and password removed.
pub fn strip_userinfo(url: &Url) -> Result<Url> {
    let mut clean = url.clone();
    clean
        .set_username("")
        .map_err(|()| Error::UserinfoNotRemovable("username"))?;
    clean
        .set_password(None)
        .map_err(|()| Error::UserinfoNotRemovable("password"))?;
    Ok(clean)
}

fn decode(s: &str) -> Result<String> {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|e| Error::InvalidUserinfo(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_encoded_userinfo() {
        let url = Url::parse("rtsp://us%40er:p%3Aw@host/s").unwrap();
        assert_eq!(
            credentials_from_url(&url).unwrap().unwrap(),
            Credentials::new("us@er", "p:w")
        );
    }

    /// RFC 3986 §3.2.1 / RFC 7617 §2: an empty user with a password is
    /// credentials (it was reported as "none" and the password silently
    /// dropped).
    #[test]
    fn an_empty_username_with_a_password_is_credentials() {
        let url = Url::parse("rtsp://:pw@host/s").unwrap();
        assert_eq!(
            credentials_from_url(&url).unwrap().unwrap(),
            Credentials::new("", "pw")
        );
    }

    #[test]
    fn no_username_is_none() {
        let url = Url::parse("http://host/s").unwrap();
        assert!(credentials_from_url(&url).unwrap().is_none());
    }

    #[test]
    fn missing_password_is_empty() {
        let url = Url::parse("http://user@host/s").unwrap();
        assert_eq!(
            credentials_from_url(&url).unwrap().unwrap(),
            Credentials::new("user", "")
        );
    }

    #[test]
    fn invalid_utf8_is_rejected_without_echoing_the_secret() {
        let url = Url::parse("http://user:%FFsecret@host/s").unwrap();
        let err = credentials_from_url(&url).unwrap_err();
        assert!(matches!(err, Error::InvalidUserinfo(_)));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn strip_removes_userinfo() {
        let url = Url::parse("http://user:secret@host/s").unwrap();
        assert_eq!(strip_userinfo(&url).unwrap().as_str(), "http://host/s");
    }

    #[test]
    fn strip_rejects_a_cannot_be_a_base_url() {
        let url = Url::parse("mailto:user@host").unwrap();
        assert!(matches!(
            strip_userinfo(&url),
            Err(Error::UserinfoNotRemovable("username"))
        ));
    }
}

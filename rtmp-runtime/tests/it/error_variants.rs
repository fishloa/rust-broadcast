#![cfg(feature = "tokio")]

use rtmp_runtime::target::{RtmpTarget, RtmpUrlError, Scheme};

#[test]
fn a_hostless_base_url_is_no_host() {
    assert_eq!(
        RtmpTarget::from_parts("rtmp:live", "live", "key").unwrap_err(),
        RtmpUrlError::NoHost
    );
}

/// #2767: a caller decides on variants, never on message text. The URL
/// parser's own error is kept typed instead of stringified, and the scheme
/// is named, so `http` is recognisable without parsing a Display string.
#[test]
fn a_malformed_url_and_a_wrong_scheme_are_typed_variants() {
    assert!(matches!(
        RtmpTarget::parse("not a url"),
        Err(RtmpUrlError::Parse(url::ParseError::RelativeUrlWithoutBase))
    ));
    assert_eq!(
        RtmpTarget::parse("http://h/live/key").unwrap_err(),
        RtmpUrlError::Scheme(Scheme::Other("http".to_string()))
    );
    assert_eq!(
        RtmpTarget::parse("ftp://h/live/key")
            .unwrap_err()
            .to_string(),
        "unsupported scheme ftp (only rtmp and rtmps)"
    );
}

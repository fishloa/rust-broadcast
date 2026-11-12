//! The `srt://` URL table (issue #2369): every shape the grammar admits and
//! every one it refuses, pinned in one place, so the parser that moved here
//! from `multimux`'s SRT push reproduces the accepted set exactly.

use srt_runtime::url::{DEFAULT_PORT, MAX_LATENCY_MS, SrtMode, SrtUrl, UrlRejection};

#[test]
fn a_bare_host_takes_the_default_port() {
    let url = SrtUrl::parse("srt://example.com").expect("bare host");
    assert_eq!(url.host, "example.com");
    assert_eq!(url.port, DEFAULT_PORT);
    assert_eq!(url.stream_id, None);
    assert_eq!(url.latency_ms, None);
    assert_eq!(url.mode, SrtMode::Caller);
    assert_eq!(url.passphrase, None);
}

#[test]
fn a_scheme_less_authority_is_accepted() {
    let url = SrtUrl::parse("example.com:9001").expect("scheme-less host:port");
    assert_eq!(url.dial_addr(), "example.com:9001");
}

#[test]
fn bracketed_ipv6_is_accepted_and_bare_ipv6_is_ambiguous() {
    let url = SrtUrl::parse("srt://[::1]:9000?latency=120").expect("bracketed v6");
    assert_eq!(url.dial_addr(), "[::1]:9000");
    for bare in ["::1", "::1:9000", "srt://::1"] {
        assert_eq!(
            SrtUrl::parse(bare),
            Err(UrlRejection::Authority),
            "{bare} is ambiguous"
        );
    }
}

#[test]
fn a_userinfo_prefix_is_refused_not_dropped() {
    assert_eq!(
        SrtUrl::parse("srt://user:key@host:9000"),
        Err(UrlRejection::Authority)
    );
    assert_eq!(
        SrtUrl::parse("srt://user@host:9000"),
        Err(UrlRejection::Authority)
    );
}

#[test]
fn a_streamid_keeps_its_whole_value_encoded_or_not() {
    let literal = SrtUrl::parse("srt://h:9000?streamid=#!::r=live/cam,m=publish")
        .expect("unencoded streamid");
    assert_eq!(
        literal.stream_id.as_deref(),
        Some("#!::r=live/cam,m=publish")
    );
    let encoded = SrtUrl::parse("srt://h:9000?streamid=%23!::r=live%2Fcam%2Cm=publish")
        .expect("encoded streamid");
    assert_eq!(encoded.stream_id, literal.stream_id);
    let stray = SrtUrl::parse("srt://h:9000?streamid=100%").expect("stray percent");
    assert_eq!(stray.stream_id.as_deref(), Some("100%"));
}

#[test]
fn the_latency_range_is_the_drafts() {
    for ms in [0u16, 200, MAX_LATENCY_MS] {
        let url = SrtUrl::parse(&format!("srt://h?latency={ms}")).expect("in range");
        assert_eq!(url.latency_ms, Some(ms));
    }
    let over = MAX_LATENCY_MS + 1;
    assert_eq!(
        SrtUrl::parse(&format!("srt://h?latency={over}")),
        Err(UrlRejection::LatencyTooLarge)
    );
    assert_eq!(
        SrtUrl::parse("srt://h?latency=notanumber"),
        Err(UrlRejection::LatencyNotANumber)
    );
    assert_eq!(
        SrtUrl::parse("srt://h?latency=99999"),
        Err(UrlRejection::LatencyNotANumber)
    );
}

#[test]
fn mode_and_passphrase_are_parsed_as_values() {
    assert_eq!(
        SrtUrl::parse("srt://h?mode=caller").expect("caller").mode,
        SrtMode::Caller
    );
    assert_eq!(
        SrtUrl::parse("srt://h?mode=listener")
            .expect("listener")
            .mode,
        SrtMode::Listener
    );
    assert_eq!(
        SrtUrl::parse("srt://h?mode=rendezvous"),
        Err(UrlRejection::Mode)
    );
    assert_eq!(
        SrtUrl::parse("srt://h?passphrase=secret")
            .expect("passphrase")
            .passphrase
            .as_deref(),
        Some("secret")
    );
}

#[test]
fn unknown_keys_slashes_and_empty_pairs_are_ignored() {
    let url = SrtUrl::parse("srt://h:9000/?unknownkey=value&").expect("trailing slash, empty pair");
    assert_eq!(url.dial_addr(), "h:9000");
    assert!(SrtUrl::parse("srt://h:9000?").is_ok());
}

#[test]
fn an_empty_authority_is_refused() {
    for bad in ["", "srt://", ":9000", "host:notaport", "host:99999"] {
        assert_eq!(SrtUrl::parse(bad), Err(UrlRejection::Authority), "{bad}");
    }
}

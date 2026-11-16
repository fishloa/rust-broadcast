//! A playlist reference is only fetched over `http`/`https`, and an `https`
//! playlist may not name an absolute `http:` resource (audit 20, a20-42). The
//! guarded resolver in `broadcast_common::url_ref` refuses both through the
//! existing `Error::UnresolvableReference`, so a `file:`/`ftp:`/`data:` key or
//! segment, or a TLS downgrade, never reaches the client's fetch actions.

use hls_runtime::client::{Action, Error, HlsClient, HlsClientConfig, Output};
use url::Url;

fn client(url: &str) -> HlsClient {
    HlsClient::new(HlsClientConfig::new(Url::parse(url).expect("URL")))
}

fn fetched_urls(client: &mut HlsClient) -> Vec<String> {
    std::iter::from_fn(|| client.poll())
        .filter_map(|a| match a {
            Action::FetchResource { url, .. } | Action::FetchKey { url, .. } => {
                Some(url.as_str().to_string())
            }
            _ => None,
        })
        .collect()
}

fn outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

const PLAYLIST_TAIL: &str =
    "#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.m4s\n#EXT-X-ENDLIST\n";

#[test]
fn a_non_http_segment_scheme_is_never_fetched() {
    let playlist = format!("#EXTM3U\n#EXT-X-VERSION:3\n{PLAYLIST_TAIL}");
    let playlist = playlist.replace("seg0.m4s", "ftp://cdn.example/seg0.m4s");
    let mut client = client("http://origin/v/media.m3u8");
    let _ = fetched_urls(&mut client);
    client.on_playlist(playlist.as_bytes()).expect("parses");
    assert_eq!(fetched_urls(&mut client), Vec::<String>::new());
    assert!(outputs(&mut client).is_empty());
    assert!(
        matches!(
            client.next_error(),
            Some(Error::UnresolvableReference { ref uri, .. })
                if uri == "ftp://cdn.example/seg0.m4s"
        ),
        "the non-HTTP reference is reported as unresolvable"
    );
}

#[test]
fn a_non_http_key_scheme_fails_the_playlist() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n\
                    #EXT-X-KEY:METHOD=AES-128,URI=\"file:///etc/passwd\"\n\
                    #EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXTINF:2.0,\nseg0.m4s\n#EXT-X-ENDLIST\n";
    let mut client = client("http://origin/v/media.m3u8");
    let _ = fetched_urls(&mut client);
    let err = client.on_playlist(playlist.as_bytes()).unwrap_err();
    assert!(
        matches!(err, Error::UnresolvableReference { .. }),
        "the non-HTTP key fails the playlist it is in: {err:?}"
    );
    let urls = fetched_urls(&mut client);
    assert!(!urls.iter().any(|u| u.contains("passwd")), "{urls:?}");
    assert!(!urls.iter().any(|u| u.contains("seg0")), "{urls:?}");
}

#[test]
fn an_https_playlist_refuses_an_http_segment() {
    let playlist = format!("#EXTM3U\n#EXT-X-VERSION:3\n{PLAYLIST_TAIL}");
    let playlist = playlist.replace("seg0.m4s", "http://cdn.example/seg0.m4s");
    let mut client = client("https://origin/v/media.m3u8");
    let _ = fetched_urls(&mut client);
    client.on_playlist(playlist.as_bytes()).expect("parses");
    assert_eq!(fetched_urls(&mut client), Vec::<String>::new());
    assert!(
        matches!(
            client.next_error(),
            Some(Error::UnresolvableReference {
                source: broadcast_common::url_ref::Error::SchemeDowngrade,
                ..
            })
        ),
        "an https playlist must not fetch an absolute http resource"
    );
}

#[test]
fn an_https_playlist_keeps_a_relative_segment_on_https() {
    let playlist = format!("#EXTM3U\n#EXT-X-VERSION:3\n{PLAYLIST_TAIL}");
    let mut client = client("https://origin/v/media.m3u8");
    let _ = fetched_urls(&mut client);
    client.on_playlist(playlist.as_bytes()).expect("parses");
    assert_eq!(
        fetched_urls(&mut client),
        vec!["https://origin/v/seg0.m4s".to_string()]
    );
}

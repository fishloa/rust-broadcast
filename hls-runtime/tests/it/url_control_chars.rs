//! A manifest reference carrying an ASCII control character is never fetched
//! and never sanitised (#2458): the WHATWG parser behind `url` DELETES a raw
//! CR/LF/TAB, so without the guard `seg\r\n1.m4s` would become a request to a
//! URL the manifest never named. The guarded resolver in
//! `broadcast-common::url_ref` rejects such a reference, and the client reports
//! it the way it already reports any other unresolvable reference.
//!
//! `broadcast_hls::MediaPlaylist::parse` splits on `\n` (a line is text up to
//! the next LF), so a reference carrying its own LF or CR arrives as one line
//! holding both halves (`seg\r` then `1.m4s`), which the parser refuses as a
//! segment URI with no `#EXTINF`. The reference actually resolved is therefore
//! the TAB form, and the remaining cases are covered through a master
//! playlist, a key URI and the segment URI text the client stores.

use hls_runtime::client::{Action, Error, HlsClient, Output};
use url::Url;

const URL: &str = "http://origin/v/media.m3u8";

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

#[test]
fn a_segment_reference_with_a_tab_is_never_fetched() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXTINF:2.0,\nseg\t1.m4s\n#EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(hls_runtime::client::HlsClientConfig::new(
        Url::parse(URL).expect("URL"),
    ));
    let _ = fetched_urls(&mut client);
    client.on_playlist(playlist.as_bytes()).unwrap();
    assert_eq!(fetched_urls(&mut client), Vec::<String>::new());
    assert!(outputs(&mut client).is_empty());
    assert!(
        matches!(
            client.next_error(),
            Some(Error::UnresolvableReference { ref uri, .. }) if uri == "seg\t1.m4s"
        ),
        "the bad reference is reported as such"
    );
}

#[test]
fn a_segment_reference_with_a_crlf_is_never_fetched() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXTINF:2.0,\nseg\r\n1.m4s\n#EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(hls_runtime::client::HlsClientConfig::new(
        Url::parse(URL).expect("URL"),
    ));
    let _ = fetched_urls(&mut client);
    let err = client.on_playlist(playlist.as_bytes()).unwrap_err();
    assert!(
        matches!(err, Error::PlaylistParse(_)),
        "the split line is refused by the parser: {err:?}"
    );
    assert_eq!(fetched_urls(&mut client), Vec::<String>::new());
    assert!(outputs(&mut client).is_empty());
}

#[test]
fn an_aes_key_reference_with_a_tab_is_never_fetched() {
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXT-X-KEY:METHOD=AES-128,URI=\"key\t1.bin\"\n#EXTINF:2.0,\nseg0.m4s\n\
                    #EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(hls_runtime::client::HlsClientConfig::new(
        Url::parse(URL).expect("URL"),
    ));
    let _ = fetched_urls(&mut client);
    let err = client.on_playlist(playlist.as_bytes()).unwrap_err();
    assert!(
        matches!(err, Error::UnresolvableReference { .. }),
        "the bad key reference fails the playlist it is in: {err:?}"
    );
    let urls = fetched_urls(&mut client);
    assert!(
        !urls.iter().any(|u| u == "http://origin/v/key1.bin"),
        "{urls:?}"
    );
    assert!(!urls.iter().any(|u| u.contains("seg0")), "{urls:?}");
}

#[test]
fn a_master_playlist_variant_with_a_tab_is_not_selected() {
    let playlist = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nseg\t1.m3u8\n\
                    #EXT-X-STREAM-INF:BANDWIDTH=400000\nlow.m3u8\n";
    let mut client = HlsClient::new(hls_runtime::client::HlsClientConfig::new(
        Url::parse(URL).expect("URL"),
    ));
    let _ = fetched_urls(&mut client);
    client.on_playlist(playlist.as_bytes()).unwrap();
    let urls = fetched_urls(&mut client);
    assert!(!urls.iter().any(|u| u.contains("seg")), "{urls:?}");
    assert!(
        urls.iter().all(|u| u == "http://origin/v/low.m3u8"),
        "{urls:?}"
    );
}

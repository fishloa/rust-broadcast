//! Golden of the HLS client's request URLs and actions (W1-R-low-b). Every row
//! here has no `..` segment, so the Task 11 `Url::join` rewrite must reproduce
//! it byte-for-byte except the three `#f` matrix rows Task 11 reviews (the old
//! builder appended a query pair after a fragment); the `..` rows (the defect)
//! are asserted in Task 11's unit tests, not in this golden.

use hls_runtime::client::{Action, BlockingReload, HlsClient};

const LIVE: &str = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=12.0\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-MEDIA-SEQUENCE:7\n\
#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXTINF:2.0,\nseg7.m4s\n#EXTINF:2.0,\n/abs/seg8.m4s\n#EXTINF:2.0,\nhttps://cdn.example/seg9.m4s\n\
#EXTINF:2.0,\n//cdn2.example/seg10.m4s\n#EXTINF:2.0,\nsub/seg11.m4s?token=a=b&x=y\n";

#[test]
fn client_actions_and_request_urls_match_golden() {
    let mut out = String::new();
    for base in [
        "http://h.example/live/stream.m3u8",
        "http://h.example:8080/live/stream.m3u8?_HLS_msn=3&k=v",
        "https://h.example/a/b/stream.m3u8#frag",
    ] {
        let mut c = HlsClient::new(base);
        out.push_str(&format!("base {base}\n"));
        out.push_str(&format!("  first {:?}\n", c.poll()));
        c.on_playlist(LIVE.as_bytes()).unwrap();
        while let Some(a) = c.poll() {
            out.push_str(&format!("  {a:?}\n"));
            if let Some(u) = a.playlist_request_url() {
                out.push_str(&format!("    request_url {u}\n"));
            }
        }
    }
    // request URL matrix (blocking x skip), independent of the engine
    for (blocking, skip) in [
        (None, false),
        (Some(BlockingReload { msn: 5, part: None }), false),
        (
            Some(BlockingReload {
                msn: 5,
                part: Some(2),
            }),
            true,
        ),
        (None, true),
    ] {
        for url in [
            "http://h/p.m3u8",
            "http://h/p.m3u8?a=1",
            "http://h/p.m3u8?a=1&b=2#f",
        ] {
            let a = Action::FetchPlaylist {
                url: url.into(),
                blocking,
                skip,
            };
            out.push_str(&format!(
                "matrix {url} blocking={blocking:?} skip={skip} -> {:?}\n",
                a.playlist_request_url()
            ));
        }
    }
    let path = format!(
        "{}/tests/golden/client_urls.golden",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(
        out,
        std::fs::read_to_string(&path).expect("golden"),
        "client golden differs"
    );
}

//! Byte-for-byte golden gate for broadcast-auth's wire output. The expected
//! files in `tests/golden/` were generated from unmodified `main` (commit in
//! `tests/golden/README.md`). `GOLDEN_BLESS=<dir>` writes instead of comparing.

use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use broadcast_auth::{Credentials, RequestContext, SignedUrlKeySet, Verifier, respond};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn keys() -> SignedUrlKeySet {
    SignedUrlKeySet::new([
        (
            "key-a".to_string(),
            b"01234567890123456789012345678901".to_vec(),
        ),
        (
            "kid_b.2-x".to_string(),
            b"abcdefghijabcdefghijabcdefghij01".to_vec(),
        ),
    ])
    .unwrap()
}

#[test]
fn signed_url_query_strings_match_golden() {
    let k = keys();
    let v4: IpAddr = "192.0.2.7".parse().unwrap();
    let v6: IpAddr = "2001:db8::1".parse().unwrap();
    let cases: [(&str, &str, u64, Option<IpAddr>); 5] = [
        ("key-a", "/live/stream.m3u8", 1_900_000_000, None),
        ("key-a", "/live/stream.m3u8", 1_900_000_000, Some(v4)),
        ("key-a", "/live/stream.m3u8", 1_900_000_000, Some(v6)),
        ("kid_b.2-x", "/vod/seg-1.m4s", 4_000_000_000, None),
        ("kid_b.2-x", "/vod/seg-1.m4s", 4_000_000_000, Some(v4)),
    ];
    let mut out = String::new();
    for (kid, path, exp, ip) in cases {
        let ip_text = ip.map_or_else(|| "-".to_string(), |i| i.to_string());
        let query = k.sign(kid, path, exp, ip).unwrap();
        out.push_str(&format!("{kid}\t{path}\t{exp}\t{ip_text}\t{query}\n"));
    }
    check("signed_url_sign.txt", &out);
}

/// The Digest nonce embeds a per-verifier random secret, so its hex is masked
/// to its length; everything else is byte-compared.
fn mask_nonce(challenge: &str) -> String {
    let start = challenge.find("nonce=\"").expect("digest nonce") + "nonce=\"".len();
    let len = challenge[start..].find('"').expect("closing quote");
    format!(
        "{}<nonce:{len} hex chars>{}",
        &challenge[..start],
        &challenge[start + len..]
    )
}

#[test]
fn challenges_match_golden() {
    let basic = Verifier::new(
        Credentials::Basic {
            username: "admin".into(),
            password: "12345".into(),
        },
        "cameras",
    );
    let digest = Verifier::new(
        Credentials::Digest {
            username: "admin".into(),
            password: "12345".into(),
        },
        "cameras",
    );
    let bearer = Verifier::new(Credentials::bearer("tok"), "cameras");
    let mut out = String::new();
    out.push_str(&format!("basic\t{}\n", basic.challenge()));
    out.push_str(&format!("digest\t{}\n", mask_nonce(&digest.challenge())));
    out.push_str(&format!("bearer\t{}\n", bearer.challenge()));
    check("challenges.txt", &out);
}

#[test]
fn client_authorization_matches_golden() {
    let ctx = RequestContext::new("GET", "/x");
    let basic = respond(
        "Basic realm=\"cameras\"",
        &ctx,
        Credentials::new("admin", "12345"),
    )
    .unwrap();
    let bearer = respond("", &ctx, Credentials::bearer("mytoken123")).unwrap();
    check(
        "client_authorization.txt",
        &format!("basic\t{basic}\nbearer\t{bearer}\n"),
    );
}

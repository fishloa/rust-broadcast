#![cfg(feature = "whip")]
//! W2a Task 8: ICE credentials are read at MEDIA level first, falling back
//! to session level (RFC 8839 §5.4) — a media section's own pair must win
//! over a stale session-level decoy.

const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96 97 98\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=rtpmap:97 H264/90000\r\n\
a=fmtp:97 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=rtpmap:98 VP8/90000\r\n";

/// Byte-for-byte golden of the WHIP SDP answer as rendered by
/// `sdp_types::Session::write` — pins the `o=`/`c=`/attribute line ORDERING
/// (which differs from main's hand-built text; see the CHANGELOG and
/// `tests/golden/README.md`). The offer, local address, ICE credentials,
/// fingerprint and candidate lines are all caller-supplied (deterministic), so
/// the golden does not depend on the random cert/candidate values.
/// `GOLDEN_BLESS=<dir>` writes instead.
#[test]
fn whip_answer_matches_golden() {
    let answer = multimux::source::whip::render_answer_for_test(
        OFFER,
        "127.0.0.1:54321".parse().unwrap(),
        "localufrag",
        "localpwdlocalpwdlocalpwd12",
        "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
         00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff",
        &["0 1 udp 2130706431 127.0.0.1 54321 typ host".to_string()],
    );
    let file = "whip_answer.golden";
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        std::fs::create_dir_all(&dir).expect("create golden dir");
        std::fs::write(std::path::Path::new(&dir).join(file), &answer).expect("write");
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file);
    let expected = std::fs::read_to_string(&path).expect("read golden");
    assert_eq!(
        answer, expected,
        "WHIP answer must match the golden byte-for-byte"
    );
}

/// CHARACTERISATION: a media-level offer parses its media-section pair.
#[test]
fn a_media_level_offer_parses_its_media_section_credentials() {
    let (ufrag, pwd) = multimux::source::whip::parse_offer_for_test(OFFER);
    assert_eq!(ufrag, "abcd");
    assert_eq!(pwd, "abcdefghijklmnopqrstuvwx");
}

/// CHARACTERISATION: session-only credentials resolve via the fallback.
#[test]
fn a_session_level_offer_still_parses_its_credentials() {
    let mut offer = String::with_capacity(OFFER.len());
    for line in OFFER.lines() {
        if line.starts_with("a=ice-ufrag:") || line.starts_with("a=ice-pwd:") {
            continue;
        }
        if line.starts_with("t=0 0") {
            offer.push_str("a=ice-ufrag:abcd\r\na=ice-pwd:abcdefghijklmnopqrstuvwx\r\n");
        }
        offer.push_str(line);
        offer.push_str("\r\n");
    }
    let (ufrag, pwd) = multimux::source::whip::parse_offer_for_test(&offer);
    assert_eq!(ufrag, "abcd");
    assert_eq!(pwd, "abcdefghijklmnopqrstuvwx");
}

/// BITING: a media section's own pair wins over a stale session-level decoy.
/// The pre-fix flat `sdp_attr_anywhere` scan took the FIRST match anywhere,
/// which is the session-level decoy — so this fails on old code.
#[test]
fn a_media_section_credentials_win_over_a_stale_session_level_pair() {
    let mut offer = String::with_capacity(OFFER.len());
    for line in OFFER.lines() {
        if line.starts_with("t=0 0") {
            offer.push_str("a=ice-ufrag:stale\r\na=ice-pwd:stalestalestalestalestale\r\n");
        }
        offer.push_str(line);
        offer.push_str("\r\n");
    }
    let (ufrag, pwd) = multimux::source::whip::parse_offer_for_test(&offer);
    assert_eq!(ufrag, "abcd", "media-level credentials must win");
    assert_eq!(
        pwd, "abcdefghijklmnopqrstuvwx",
        "media-level credentials must win"
    );
}

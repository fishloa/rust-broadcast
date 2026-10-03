//! `WWW-Authenticate` helper over `http_auth::parse_challenges` (replaces a
//! hand-rolled `split(',')` scan). `Session` and `Transport` live in
//! `session_header` / `transport` (owner decision (c)).

/// `stale=true` in any Digest challenge (RFC 7616 §3.3): parsed with the
/// RFC 7235 challenge grammar, so quoted commas and a leading `stale` are right.
pub(crate) fn challenge_is_stale(challenge: &str) -> bool {
    let Ok(challenges) = http_auth::parse_challenges(challenge) else {
        return false;
    };
    challenges
        .iter()
        .filter(|c| c.scheme.eq_ignore_ascii_case("digest"))
        .flat_map(|c| c.params.iter())
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("stale") && value.to_unescaped().eq_ignore_ascii_case("true")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_is_found_as_the_first_parameter_after_the_scheme() {
        // Old scan: first segment is "Digest stale=true" -> name "Digest stale" -> missed.
        assert!(challenge_is_stale(
            r#"Digest stale=true, realm="cam", nonce="n""#
        ));
    }

    #[test]
    fn stale_is_not_confused_by_a_comma_inside_a_quoted_realm() {
        assert!(!challenge_is_stale(
            r#"Digest realm="a, stale=true", nonce="n""#
        ));
        assert!(challenge_is_stale(
            r#"Digest realm="a,b", nonce="n", stale="TRUE""#
        ));
    }

    #[test]
    fn stale_matches_the_existing_cases() {
        assert!(challenge_is_stale(
            r#"Digest realm="cam",nonce="x",stale="True""#
        ));
        assert!(challenge_is_stale(r#"Digest realm="cam",STALE=true"#));
        assert!(!challenge_is_stale(r#"Digest realm="cam",nonce="x""#));
        assert!(!challenge_is_stale(r#"Digest realm="cam",stale=false"#));
        // A Basic challenge next to a Digest one: only Digest's params count.
        assert!(!challenge_is_stale(r#"Basic realm="x", stale=true"#));
    }
}

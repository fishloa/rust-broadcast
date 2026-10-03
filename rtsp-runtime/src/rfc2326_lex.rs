//! The single RFC 2326 §15.1 field-value lexer shared by the `Transport` (§12.39)
//! and `Session` (§12.37) header parsers. Owner decision (c): rtsp-runtime owns
//! these two header grammars because `rtsp-types` 0.1.3 mis-parses real-world
//! values (see "rtsp-types gaps" in `docs/transport-header.md`).
//!
//! Spec: `docs/rfc2326.md` §15.1 Base Syntax:
//!
//! ```text
//! LWS           = [CRLF] 1*( SP | HT )
//! tspecials     = "(" | ")" | "<" | ">" | "@" | "," | ";" | ":" | "\" | <">
//!               | "/" | "[" | "]" | "?" | "=" | "{" | "}" | SP | HT
//! token         = 1*<any CHAR except CTLs or tspecials>
//! quoted-string = ( <"> *(qdtext) <"> )      quoted-pair = "\" CHAR
//! ```
//!
//! Implied LWS (RFC 2068 §2.1) is allowed around every separator, so segments
//! are trimmed of SP/HT/CR/LF. Separator splitting never looks inside a
//! quoted-string. Everything above this module works on already-split segments.
//! This is the ONLY module of the crate allowed to tokenise header text by hand
//! (see `tests/no_handroll_guard.rs`).

/// Separator between transport-specs (§12.39 `1#transport-spec`).
pub(crate) const LIST_SEP: char = ',';
/// Separator between parameters (§12.39).
pub(crate) const PARAM_SEP: char = ';';
/// Separator between a parameter name and its value.
pub(crate) const VALUE_SEP: char = '=';
/// Separator between `transport/profile/lower-transport`.
pub(crate) const SLASH: char = '/';
/// Separator of a `lo-hi` range (`port`, `interleaved`, …).
pub(crate) const RANGE_SEP: char = '-';
const DQUOTE: char = '"';
const BACKSLASH: char = '\\';

/// Why a field value could not be lexed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LexError(pub(crate) String);

fn err<T>(msg: impl Into<String>) -> Result<T, LexError> {
    Err(LexError(msg.into()))
}

/// LWS characters (CRLF folding is treated as whitespace too).
fn is_lws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Trims implied LWS from both ends.
pub(crate) fn trim_lws(s: &str) -> &str {
    s.trim_matches(is_lws)
}

/// `token = 1*<any CHAR except CTLs or tspecials>` (§15.1).
pub(crate) fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii()
                && !c.is_ascii_control()
                && !matches!(
                    c,
                    '(' | ')'
                        | '<'
                        | '>'
                        | '@'
                        | ','
                        | ';'
                        | ':'
                        | '\\'
                        | '"'
                        | '/'
                        | '['
                        | ']'
                        | '?'
                        | '='
                        | '{'
                        | '}'
                        | ' '
                        | '\t'
                )
        })
}

/// Splits `s` at every `sep` that is outside a quoted-string (a quoted-pair
/// escapes the next character). Segments are returned untrimmed.
pub(crate) fn split_outside_quotes(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut in_quote, mut escaped) = (0, false, false);
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if in_quote && c == BACKSLASH {
            escaped = true;
        } else if c == DQUOTE {
            in_quote = !in_quote;
        } else if c == sep && !in_quote {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// A `name[=value]` parameter segment: the name is a token, the value (if any)
/// is everything after the first `=`, LWS-trimmed.
pub(crate) fn split_param(seg: &str) -> Result<(&str, Option<&str>), LexError> {
    let seg = trim_lws(seg);
    let (name, value) = match seg.split_once(VALUE_SEP) {
        Some((n, v)) => (trim_lws(n), Some(trim_lws(v))),
        None => (seg, None),
    };
    if !is_token(name) {
        return err(format!("invalid parameter name {name:?}"));
    }
    Ok((name, value))
}

/// Interprets a value: a quoted-string is unquoted (quoted-pairs resolved) and
/// must end at the closing quote; anything else is returned as written.
/// The flag says whether it was quoted.
pub(crate) fn unquote(v: &str) -> Result<(String, bool), LexError> {
    let v = trim_lws(v);
    if v.chars().any(|c| c.is_ascii_control()) {
        return err("control character in a header value");
    }
    let Some(rest) = v.strip_prefix(DQUOTE) else {
        return Ok((v.to_string(), false));
    };
    let mut out = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next() {
            None => return err("unterminated quoted-string"),
            Some(BACKSLASH) => match chars.next() {
                Some(c) => out.push(c),
                None => return err("dangling quoted-pair"),
            },
            Some(DQUOTE) => {
                return if trim_lws(chars.as_str()).is_empty() {
                    Ok((out, true))
                } else {
                    err("text after a quoted-string")
                };
            }
            Some(c) => out.push(c),
        }
    }
}

/// Canonical value form: as written when it is a safe bare value, otherwise a
/// quoted-string with quoted-pairs. Parsing the result gives back `v`. A control
/// character (qdtext excludes CTLs, §15.1) is an error: nothing unsafe is emitted.
pub(crate) fn emit_value(v: &str) -> Result<String, LexError> {
    if v.chars().any(|c| c.is_ascii_control()) {
        return err("control character in a header value");
    }
    let bare = !v.is_empty()
        && v.chars()
            .all(|c| !is_lws(c) && !matches!(c, ',' | ';' | '"' | '\\'))
        && v.trim_matches(is_lws) == v;
    if bare {
        return Ok(v.to_string());
    }
    emit_value_quoted(v)
}

/// Always a quoted-string (used for `mode`, whose grammar requires the quotes).
/// Control characters are rejected, never written.
pub(crate) fn emit_value_quoted(v: &str) -> Result<String, LexError> {
    if v.chars().any(|c| c.is_ascii_control()) {
        return err("control character in a header value");
    }
    let mut out = String::from(DQUOTE);
    for c in v.chars() {
        if c == DQUOTE || c == BACKSLASH {
            out.push(BACKSLASH);
        }
        out.push(c);
    }
    out.push(DQUOTE);
    Ok(out)
}

/// True if `s` holds a control character (CR, LF, ...). The only thing a
/// received session-id may not contain, and what makes echoing it unsafe.
pub(crate) fn has_ctl(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_control())
}

/// `1*<max_digits>DIGIT` into a `u32` (callers range-check against their type).
pub(crate) fn digits(s: &str, max_digits: usize, what: &str) -> Result<u32, LexError> {
    let s = trim_lws(s);
    if s.is_empty() || s.len() > max_digits || !s.bytes().all(|b| b.is_ascii_digit()) {
        return err(format!(
            "{what}: expected 1 to {max_digits} digits, got {s:?}"
        ));
    }
    s.parse::<u32>()
        .or_else(|_| err(format!("{what}: {s:?} out of range")))
}

/// Exactly `digits` hex digits (`ssrc = 8*8(HEX)`), as a `u32`.
pub(crate) fn hex_exact(s: &str, digits: usize, what: &str) -> Result<u32, LexError> {
    let s = trim_lws(s);
    if s.len() != digits || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return err(format!("{what} must be {digits} hex digits, got {s:?}"));
    }
    u32::from_str_radix(s, 16).or_else(|_| err(format!("{what}: bad hex {s:?}")))
}

/// `value [ "-" value ]` -> (lo, hi); a missing `hi` is `lo`.
pub(crate) fn range(
    v: &str,
    max_digits: usize,
    max: u32,
    what: &str,
) -> Result<(u32, u32), LexError> {
    let mut parts = v.split(RANGE_SEP);
    let lo = digits(parts.next().unwrap_or(""), max_digits, what)?;
    let hi = match parts.next() {
        Some(h) => digits(h, max_digits, what)?,
        None => lo,
    };
    if parts.next().is_some() {
        return err(format!("{what}: more than two range ends in {v:?}"));
    }
    if lo > max || hi > max {
        return err(format!("{what}: {v:?} exceeds {max}"));
    }
    Ok((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_respects_quoted_strings_and_quoted_pairs() {
        assert_eq!(
            split_outside_quotes(r#"a;b="x;y";c"#, ';'),
            vec!["a", r#"b="x;y""#, "c"]
        );
        assert_eq!(
            split_outside_quotes(r#"a="q\";x";b"#, ';'),
            vec![r#"a="q\";x""#, "b"]
        );
        assert_eq!(split_outside_quotes("", ','), vec![""]);
    }

    #[test]
    fn unquote_and_emit_round_trip() {
        for v in ["PLAY", "a b", "a,b", "q\"x", "back\\slash", "", " lead"] {
            assert_eq!(unquote(&emit_value(v).unwrap()).unwrap().0, v, "{v:?}");
        }
        assert!(unquote("\"open").is_err());
        assert!(emit_value("a\r\nb").is_err());
        assert!(emit_value_quoted("a\nb").is_err());
        assert!(unquote("\"a\r\nb\"").is_err());
        assert!(unquote("\"a\"junk").is_err());
        assert_eq!(unquote("  tok ").unwrap(), ("tok".into(), false));
    }

    #[test]
    fn tokens_follow_the_tspecials_list() {
        assert!(is_token("client_port") && is_token("3056-3057"));
        assert!(!is_token("") && !is_token("a;b") && !is_token("a b") && !is_token("a=b"));
    }

    #[test]
    fn ranges_check_digits_and_bounds() {
        assert_eq!(range("1-2", 5, 65535, "port").unwrap(), (1, 2));
        assert_eq!(range(" 7 ", 3, 255, "ch").unwrap(), (7, 7));
        assert!(range("65536", 5, 65535, "port").is_err());
        assert!(range("123456", 5, 65535, "port").is_err());
        assert!(range("1-2-3", 5, 65535, "port").is_err());
        assert!(range("a", 5, 65535, "port").is_err());
        assert!(range("-3", 5, 65535, "port").is_err());
    }
}

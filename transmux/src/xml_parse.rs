//! Minimal no_std XML tokenizer — dep-free, scoped to exactly what MPD
//! parsing and Smooth Streaming manifests need.
//!
//! Provides a hand-rolled, bounded, panic-free XML event stream
//! (`XmlTokenizer` + `XmlEvent`) for dash_parse and smooth_parse consumers.
//! Not a general-purpose XML parser: no DTD/CDATA support, and
//! unknown namespace-prefixed names are accepted with the prefix stripped
//! rather than resolved.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors raised while tokenizing/parsing XML.
///
/// These are decoupled from DASH-specific errors so a second manifest parser
/// (MS-SSTR, etc.) can reuse the tokenizer and convert these to its own error
/// type via `From`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum XmlError {
    /// The input ended before a well-formed construct was found.
    UnexpectedEof,
    /// A `<...>` tag, `<!--...-->` comment, `<?...?>` declaration, or
    /// `<!...>` markup declaration was never closed.
    UnterminatedTag {
        /// Byte offset (into the input) where the unterminated construct began.
        pos: usize,
    },
    /// An attribute inside a start tag was not well-formed
    /// (`name="value"`/`name='value'`, XML 1.0 §3.1).
    MalformedAttribute {
        /// Byte offset (into the input) of the offending attribute.
        pos: usize,
    },
    /// An end tag's name does not match the element currently open — a
    /// malformed nesting that would silently truncate the structure.
    MismatchedEndTag {
        /// The element name expected to close.
        expected: &'static str,
        /// The element name actually found in the closing tag.
        found: String,
    },
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlError::UnexpectedEof => {
                write!(f, "unexpected end of input while parsing XML")
            }
            XmlError::UnterminatedTag { pos } => {
                write!(
                    f,
                    "unterminated XML tag/comment/declaration at byte offset {pos}"
                )
            }
            XmlError::MalformedAttribute { pos } => {
                write!(f, "malformed XML attribute near byte offset {pos}")
            }
            XmlError::MismatchedEndTag { expected, found } => {
                if found.is_empty() {
                    write!(f, "expected closing tag </{expected}>, found none")
                } else {
                    write!(f, "expected closing tag </{expected}>, found </{found}>")
                }
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for XmlError {}

/// Crate-local result alias for the XML parser.
pub(crate) type Result<T> = core::result::Result<T, XmlError>;

// ---------------------------------------------------------------------------
// XML event stream
// ---------------------------------------------------------------------------

/// One tokenizer event. Text content, comments, processing instructions, and
/// markup declarations are consumed internally by [`XmlTokenizer::next_event`]
/// and never surfaced — this parser's grammar has no element with text
/// content it needs.
pub(crate) enum XmlEvent<'a> {
    /// A start tag, e.g. `<Period id="0">` or the self-closing `<S d="4" />`.
    Start {
        /// The element's local name (namespace prefix, if any, stripped).
        name: &'a str,
        /// Attribute name/value pairs, in document order (values unescaped).
        attrs: Vec<(String, String)>,
        /// Whether the tag was self-closing (`<Name .../>`).
        self_closing: bool,
    },
    /// An end tag, e.g. `</Period>`.
    End {
        /// The element's local name (namespace prefix, if any, stripped).
        name: &'a str,
    },
}

/// A minimal, bounded, panic-free XML tokenizer sufficient for MPD and
/// Smooth Streaming manifest parsing. Not a general-purpose XML parser:
/// no DTD/CDATA support, and unknown namespace-prefixed names are accepted
/// with the prefix simply stripped rather than resolved.
pub(crate) struct XmlTokenizer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> XmlTokenizer<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    /// The byte offset just past the most recently returned start tag — i.e.
    /// where the currently-open element's content begins. Used by
    /// [`text_content`] to read an element's character data, which the event
    /// stream itself consumes.
    pub(crate) fn content_start(&self) -> usize {
        self.pos
    }

    /// Return the next `Start`/`End` event, or `Ok(None)` at end of input.
    /// Skips leading/trailing text, `<?...?>` declarations, `<!--...-->`
    /// comments, and `<!...>` markup declarations internally.
    pub(crate) fn next_event(&mut self) -> Result<Option<XmlEvent<'a>>> {
        loop {
            let Some(rel) = self.input[self.pos..].find('<') else {
                return Ok(None);
            };
            self.pos += rel;
            let rest = &self.input[self.pos..];

            if let Some(after) = rest.strip_prefix("<?") {
                let end = after
                    .find("?>")
                    .ok_or(XmlError::UnterminatedTag { pos: self.pos })?;
                self.pos += 2 + end + 2;
                continue;
            }
            if let Some(after) = rest.strip_prefix("<!--") {
                let end = after
                    .find("-->")
                    .ok_or(XmlError::UnterminatedTag { pos: self.pos })?;
                self.pos += 4 + end + 3;
                continue;
            }
            if let Some(after) = rest.strip_prefix("<!") {
                let end = after
                    .find('>')
                    .ok_or(XmlError::UnterminatedTag { pos: self.pos })?;
                self.pos += 2 + end + 1;
                continue;
            }
            if let Some(after) = rest.strip_prefix("</") {
                let end = after
                    .find('>')
                    .ok_or(XmlError::UnterminatedTag { pos: self.pos })?;
                let name = strip_ns_prefix(after[..end].trim());
                self.pos += 2 + end + 1;
                return Ok(Some(XmlEvent::End { name }));
            }

            // The start tag ends at the first `>` **outside a quoted attribute
            // value**: XML 1.0 §2.4 only requires `<` and `&` to be escaped in
            // attribute values, so a `>` (e.g. in a DASH
            // `SegmentTemplate@media` or a `ContentProtection` scheme id) is
            // legal raw and must not terminate the tag (r04-W46).
            let end = find_tag_end(rest).ok_or(XmlError::UnterminatedTag { pos: self.pos })?;
            let mut body = &rest[1..end];
            let self_closing = body.trim_end().ends_with('/');
            if self_closing {
                body = body.trim_end();
                body = &body[..body.len() - 1];
            }
            let (name_raw, attrs_str) = split_name_attrs(body);
            let name = strip_ns_prefix(name_raw);
            let attrs = parse_attrs(attrs_str.trim())?;
            self.pos += end + 1;
            return Ok(Some(XmlEvent::Start {
                name,
                attrs,
                self_closing,
            }));
        }
    }
}

/// Strip a namespace prefix (`cenc:pssh` → `pssh`); names with no `:` are
/// returned unchanged.
fn strip_ns_prefix(name: &str) -> &str {
    match name.rfind(':') {
        Some(idx) => &name[idx + 1..],
        None => name,
    }
}

/// Byte offset of the `>` that ends a start tag, skipping any `>` inside a
/// single- or double-quoted attribute value (XML 1.0 §2.4). `s` starts at the
/// tag's `<`; the returned offset is relative to `s`.
fn find_tag_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// Split a start-tag body (everything between `<` and `>`, self-closing `/`
/// already stripped) into `(name, attrs_str)`.
fn split_name_attrs(body: &str) -> (&str, &str) {
    let trimmed = body.trim_start();
    match trimmed.find(|c: char| c.is_whitespace()) {
        Some(idx) => (&trimmed[..idx], trimmed[idx..].trim()),
        None => (trimmed.trim_end(), ""),
    }
}

fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Parse `name="value" name='value' ...` into owned, unescaped pairs. Bounds
/// every slice before taking it; returns [`XmlError::MalformedAttribute`]
/// rather than panicking on truncated/unquoted input.
pub(crate) fn parse_attrs(s: &str) -> Result<Vec<(String, String)>> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut attrs = Vec::new();
    while i < len {
        while i < len && is_ascii_ws(bytes[i]) {
            i += 1;
        }
        if i >= len {
            break;
        }
        let name_start = i;
        while i < len && bytes[i] != b'=' && !is_ascii_ws(bytes[i]) {
            i += 1;
        }
        let name_end = i;
        if name_end == name_start {
            return Err(XmlError::MalformedAttribute { pos: name_start });
        }
        while i < len && is_ascii_ws(bytes[i]) {
            i += 1;
        }
        if i >= len || bytes[i] != b'=' {
            return Err(XmlError::MalformedAttribute { pos: name_start });
        }
        i += 1;
        while i < len && is_ascii_ws(bytes[i]) {
            i += 1;
        }
        if i >= len || (bytes[i] != b'"' && bytes[i] != b'\'') {
            return Err(XmlError::MalformedAttribute { pos: name_start });
        }
        let quote = bytes[i];
        i += 1;
        let val_start = i;
        while i < len && bytes[i] != quote {
            i += 1;
        }
        if i >= len {
            return Err(XmlError::MalformedAttribute { pos: val_start });
        }
        let val_end = i;
        i += 1;
        let name = s[name_start..name_end].to_string();
        let value = unescape(&s[val_start..val_end]);
        attrs.push((name, value));
    }
    Ok(attrs)
}

/// Reverse XML writer-side escaping (XML 1.0 §2.4):
/// `&amp;`/`&lt;`/`&gt;`/`&quot;`/`&apos;` plus numeric character references
/// (`&#38;`/`&#x26;`) → their literal characters.
/// Unknown/malformed entities (no known name, or a missing `;`) are passed
/// through byte-for-byte rather than rejected.
///
/// Delegates to [`unescape_text`] — the same resolution rule for a text run and
/// an attribute value, so a numeric reference in a manifest attribute (a DASH
/// `SegmentTemplate@media` written as `a?x=1&#38;n=$Number$`) resolves rather
/// than staying literal (r04-W46).
pub(crate) fn unescape(s: &str) -> String {
    unescape_text(s)
}

/// Skip an already-open element's subtree, up to and including its matching
/// end tag. Depth-counted (not name-matched) — well-formed nesting is assumed,
/// which is enough to tolerate any element a parser doesn't model without
/// choking on it.
pub(crate) fn skip_element(tok: &mut XmlTokenizer<'_>) -> Result<()> {
    let mut depth: usize = 1;
    while depth > 0 {
        match tok.next_event()? {
            Some(XmlEvent::Start { self_closing, .. }) => {
                if !self_closing {
                    depth += 1;
                }
            }
            Some(XmlEvent::End { .. }) => depth -= 1,
            None => return Err(XmlError::UnexpectedEof),
        }
    }
    Ok(())
}

/// The character data of an already-open element (the text between its start
/// tag and its matching end tag), with XML entity references resolved.
///
/// [`XmlTokenizer::next_event`] consumes text internally, so this reads the
/// element's raw span out of `data` directly. A simple element is expected: any
/// nested markup makes this return `None` (the callers that use it all parse
/// text-only elements, e.g. an MPD `BaseURL`). Returns `None` for a
/// self-closing element, whose content is empty by definition.
pub(crate) fn text_content<'a>(
    tok: &mut XmlTokenizer<'a>,
    data: &'a str,
    name: &'static str,
    self_closing: bool,
) -> Result<Option<String>> {
    if self_closing {
        return Ok(None);
    }
    let start = tok.content_start();
    let end = data[start..]
        .find("</")
        .map(|rel| start + rel)
        .ok_or(XmlError::UnterminatedTag { pos: start })?;
    let raw = &data[start..end];
    if raw.contains('<') {
        return Ok(None);
    }
    // Consume the element's end tag so the caller's event stream stays aligned.
    match tok.next_event()? {
        Some(XmlEvent::End { name: found }) if found == name => {}
        Some(XmlEvent::End { name: found }) => {
            return Err(XmlError::MismatchedEndTag {
                expected: name,
                found: found.to_string(),
            });
        }
        _ => return Err(XmlError::UnexpectedEof),
    }
    Ok(Some(unescape_text(raw)))
}

/// Resolve the five predefined XML entities plus numeric character references
/// (`&#NN;`/`&#xNN;`) in a text run. An unknown or malformed reference is left
/// verbatim rather than dropped.
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp + 1..];
        // A reference candidate ends at the first `;` **or** the next `&`,
        // whichever comes first: `&amp;` inside `"a & b &amp; c"` must still
        // decode, so a bare `&` cannot make everything up to a later `;` one
        // entity name. Advancing past the `&` on that path keeps the scan linear.
        let semi = tail.find(';');
        let next_amp = tail.find('&');
        let end = match (semi, next_amp) {
            (Some(s), Some(a)) if a < s => a,
            (Some(s), _) => s,
            (None, Some(a)) => a,
            (None, None) => tail.len(),
        };
        if semi.is_none() && next_amp.is_none() {
            // No `;` and no further `&`: emit the lone `&` and the rest verbatim.
            out.push('&');
            out.push_str(tail);
            rest = "";
            break;
        }
        let entity = &tail[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => decode_numeric_entity(entity),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                // A decoded reference consumed its terminating `;`.
                rest = &tail[end + 1..];
            }
            None => {
                // Not a reference: emit the lone `&` and resume right after it,
                // leaving the candidate's text for the outer scan.
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `&#NN;` (decimal) / `&#xNN;` (hexadecimal) → the character it names, or
/// `None` for anything XML 1.0 does not permit.
///
/// The `Char` production (XML 1.0 §2.2) is
/// `#x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] | [#x10000-#x10FFFF]`,
/// so this rejects a zero code point, the control characters (except tab, LF
/// and CR), the surrogate range, and anything above `#x10FFFF` — a decoded
/// control character would be an illegal XML character, and `char::from_u32`
/// alone would accept `&#0;` as NUL. A leading `+` or `-` is also rejected
/// (not part of the production; `i32`-style parsing would silently accept it).
fn decode_numeric_entity(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix('#')?;
    if digits.starts_with(['+', '-']) {
        return None;
    }
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    // XML 1.0 §2.2 `Char`.
    let permitted = matches!(code, 0x9 | 0xA | 0xD)
        || (0x20..=0xD7FF).contains(&code)
        || (0xE000..=0xFFFD).contains(&code)
        || (0x10000..=0x10FFFF).contains(&code);
    if !permitted {
        return None;
    }
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_start_attrs(xml: &str) -> Vec<(String, String)> {
        let mut tok = XmlTokenizer::new(xml);
        loop {
            match tok.next_event().expect("tokenize") {
                Some(XmlEvent::Start { attrs, .. }) => return attrs,
                Some(_) => continue,
                None => panic!("no start tag"),
            }
        }
    }

    /// r04-W46: a `>` inside a quoted attribute value is legal raw (XML 1.0
    /// §2.4 requires only `<` and `&` to be escaped there) and must not
    /// terminate the start tag. Unfixed, the tag ended at the `>` in the value,
    /// so the attribute list was truncated and the element's real attributes
    /// (here `id`) were misparsed.
    #[test]
    fn greater_than_inside_attribute_value_is_not_a_tag_end() {
        let attrs = first_start_attrs(r#"<SegmentTemplate media="a?x=1&y=2>3" id="v"/>"#);
        assert_eq!(
            attrs,
            alloc::vec![
                ("media".to_string(), "a?x=1&y=2>3".to_string()),
                ("id".to_string(), "v".to_string()),
            ]
        );
    }

    /// A single-quoted value with `>` behaves the same.
    #[test]
    fn greater_than_inside_single_quoted_value() {
        let attrs = first_start_attrs("<X a='><' b='z'/>");
        assert_eq!(
            attrs,
            alloc::vec![
                ("a".to_string(), "><".to_string()),
                ("b".to_string(), "z".to_string()),
            ]
        );
    }

    /// Numeric character references in an attribute value resolve (r04-W46); an
    /// `&#38;` is the `&` a writer must use to keep a query string's separator.
    #[test]
    fn numeric_character_references_in_attribute_values() {
        let attrs = first_start_attrs(r#"<X media="a?x=1&#38;n=$Number$" hex="&#x26;"/>"#);
        assert_eq!(attrs[0].1, "a?x=1&n=$Number$");
        assert_eq!(attrs[1].1, "&");
    }

    /// The five named entities and a malformed reference (left verbatim) still
    /// behave as before.
    #[test]
    fn named_and_malformed_entities_in_attribute_values() {
        let attrs = first_start_attrs(r#"<X a="&lt;&gt;&amp;&quot;&apos;" b="&nope;"/>"#);
        assert_eq!(attrs[0].1, "<>&\"'");
        assert_eq!(attrs[1].1, "&nope;");
    }

    /// Numeric references outside XML 1.0's `Char` production are left
    /// verbatim, never decoded into an illegal character.
    #[test]
    fn numeric_entity_outside_the_char_production_is_left_verbatim() {
        // Zero (NUL), bare control chars, surrogates, above #x10FFFF, and a
        // leading '+' / '-' -- all invalid.
        for bad in [
            "&#0;",
            "&#x0;",
            "&#1;",
            "&#x1F;",
            "&#xD800;",
            "&#xDFFF;",
            "&#x110000;",
            "&#xFFFFFFFF;",
            "&#+38;",
            "&#-1;",
        ] {
            assert_eq!(unescape_text(bad), bad, "{bad} must be left verbatim");
            assert_eq!(unescape(bad), bad, "{bad} (attribute) must be verbatim");
        }
        // The permitted edges decode (XML 1.0 §2.2 Char).
        assert_eq!(unescape_text("&#x9;"), "\t");
        assert_eq!(unescape_text("&#xA;"), "\n");
        assert_eq!(unescape_text("&#xD;"), "\r");
        assert_eq!(unescape_text("&#x20;"), " ");
        assert_eq!(
            unescape_text("&#xD7FF;"),
            char::from_u32(0xD7FF).unwrap().to_string()
        );
        assert_eq!(
            unescape_text("&#xE000;"),
            char::from_u32(0xE000).unwrap().to_string()
        );
        assert_eq!(
            unescape_text("&#xFFFD;"),
            char::from_u32(0xFFFD).unwrap().to_string()
        );
        assert_eq!(
            unescape_text("&#x10000;"),
            char::from_u32(0x10000).unwrap().to_string()
        );
        assert_eq!(
            unescape_text("&#x10FFFF;"),
            char::from_u32(0x10FFFF).unwrap().to_string()
        );
    }

    /// A bare `&` must not swallow a later valid reference: the candidate for a
    /// reference ends at the first `;` **or** the next `&`, so
    /// `"a & b &amp; c"` decodes the `&amp;` while leaving the lone `&` alone.
    #[test]
    fn bare_ampersand_does_not_swallow_a_later_reference() {
        assert_eq!(unescape_text("a & b &amp; c"), "a & b & c");
        assert_eq!(unescape_text("x && amp;"), "x && amp;");
        assert_eq!(unescape_text("&&amp;"), "&&");
        // Trailing lone `&`.
        assert_eq!(unescape_text("abc&"), "abc&");
        assert_eq!(unescape_text("abc&def"), "abc&def");
        // The named and numeric forms still resolve.
        assert_eq!(unescape_text("&amp;&lt;&#65;"), "&<A");
        // Attribute values use the same rule.
        assert_eq!(unescape("a & b &amp; c"), "a & b & c");
    }

    /// `unescape_text` is linear: a long run of `&amp;` decodes in one pass, and
    /// a malformed run with no `;` does not rescan the tail.
    #[test]
    fn unescape_is_linear_over_a_long_run() {
        let many = "&amp;".repeat(20_000);
        assert_eq!(unescape_text(&many), "&".repeat(20_000));
        // No `;` anywhere: the whole tail is emitted once, verbatim.
        let malformed = "a&amp".repeat(20_000);
        let out = unescape_text(&malformed);
        assert!(out.starts_with("a&amp"));
        assert_eq!(out.matches("&amp").count(), 20_000);
    }

    /// Hostile input: an unterminated quoted value is a structured error, never
    /// a panic.
    #[test]
    fn unterminated_quote_is_error_not_panic() {
        let mut tok = XmlTokenizer::new(r#"<X a="unterminated"#);
        assert!(tok.next_event().is_err());
    }
}

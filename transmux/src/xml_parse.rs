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

            let end = rest
                .find('>')
                .ok_or(XmlError::UnterminatedTag { pos: self.pos })?;
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
/// `&amp;`/`&lt;`/`&gt;`/`&quot;`/`&apos;` → their literal characters.
/// Unknown/malformed entities (no known name, or a missing `;`) are passed
/// through byte-for-byte rather than rejected.
pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match tail.find(';') {
            Some(semi) => {
                let entity = &tail[1..semi];
                match entity {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ => {
                        out.push('&');
                        rest = &tail[1..];
                        continue;
                    }
                }
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
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
        let Some(semi) = tail.find(';') else {
            out.push('&');
            rest = tail;
            continue;
        };
        let entity = &tail[..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => decode_numeric_entity(entity),
        };
        match decoded {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

/// `&#NN;` (decimal) / `&#xNN;` (hexadecimal) → the character it names.
fn decode_numeric_entity(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

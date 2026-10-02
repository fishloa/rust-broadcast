//! Minimal indentation-aware XML element writer shared by the DASH MPD and
//! Smooth Streaming manifest renderers (one copy, audit r05-O6; they carried
//! verbatim duplicates). No external dependency.

use alloc::string::String;

/// A minimal, indentation-aware XML element writer.
///
/// Escapes attribute values per XML 1.0 §2.4; emits `<?xml ...?>` then nested
/// open/close/empty elements. Not a general-purpose serializer — just enough to
/// render the MPD structure this module produces.
pub(crate) struct XmlWriter {
    buf: String,
    depth: usize,
}

impl XmlWriter {
    pub(crate) fn new() -> Self {
        Self {
            buf: String::new(),
            depth: 0,
        }
    }

    pub(crate) fn declaration(&mut self) {
        self.buf
            .push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.buf.push_str("  ");
        }
    }

    fn attrs(&mut self, attrs: &[(&str, String)]) {
        for (k, v) in attrs {
            self.buf.push(' ');
            self.buf.push_str(k);
            self.buf.push_str("=\"");
            escape_into(&mut self.buf, v);
            self.buf.push('"');
        }
    }

    pub(crate) fn open(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.indent();
        self.buf.push('<');
        self.buf.push_str(name);
        self.attrs(attrs);
        self.buf.push_str(">\n");
        self.depth += 1;
    }

    pub(crate) fn empty(&mut self, name: &str, attrs: &[(&str, String)]) {
        self.indent();
        self.buf.push('<');
        self.buf.push_str(name);
        self.attrs(attrs);
        self.buf.push_str("/>\n");
    }

    pub(crate) fn close(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.indent();
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push_str(">\n");
    }

    /// Write a leaf element with escaped text content, on one line
    /// (`<name>text</name>`).
    pub(crate) fn text(&mut self, name: &str, text: &str) {
        self.indent();
        self.buf.push('<');
        self.buf.push_str(name);
        self.buf.push('>');
        escape_into(&mut self.buf, text);
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push_str(">\n");
    }

    pub(crate) fn finish(self) -> String {
        self.buf
    }
}

/// Escape a string for use in an XML attribute value (XML 1.0 §2.4).
fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
}

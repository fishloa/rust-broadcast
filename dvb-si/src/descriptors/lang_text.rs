//! Shared wire plumbing for the "language code + length-prefixed text"
//! entry loops: `multilingual_network_name` (§6.2.24), `multilingual_bouquet_name`
//! (§6.2.22), `multilingual_service_name` (§6.2.25), `multilingual_component`
//! (§6.2.23) and the AIT `application_name_descriptor` (TS 102 809 §5.3.5.6.2).
//!
//! Every one is a loop of `ISO_639_language_code (24) + text_length (8) +
//! text`, differing only in the field *names* (kept on each descriptor's own
//! public entry type) and, for the service-name descriptor, in carrying two
//! strings per entry. The five modules carried ~1,000 duplicated lines in
//! which the over-range check had drifted (some returned `InvalidDescriptor`,
//! some `FieldOverflow`) — audit r03-O1, #1141. The reader and writer below
//! are the one implementation; every serializer length goes through
//! `broadcast_common::len::fit_u8`.

use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};

/// `ISO_639_language_code` width, bytes.
pub(super) const LANG_LEN: usize = 3;
/// Width of the 8-bit `text_length` prefix, bytes.
pub(super) const LEN_FIELD: usize = 1;

/// Wire size of one length-prefixed text field.
pub(super) fn text_field_len(text: &DvbText<'_>) -> usize {
    LEN_FIELD + text.len()
}

/// Forward cursor over a descriptor body's entry loop.
pub(super) struct EntryReader<'a> {
    body: &'a [u8],
    pos: usize,
    tag: u8,
}

impl<'a> EntryReader<'a> {
    /// Start reading `body` at byte `pos` (past any fixed leading fields).
    pub(super) fn new(body: &'a [u8], pos: usize, tag: u8) -> Self {
        Self { body, pos, tag }
    }

    /// Whether any bytes remain to be read.
    pub(super) fn has_more(&self) -> bool {
        self.pos < self.body.len()
    }

    fn err(&self, reason: &'static str) -> Error {
        Error::InvalidDescriptor {
            tag: self.tag,
            reason,
        }
    }

    /// Read the language code, requiring the first `text_length` byte to be
    /// present too (the entry header).
    pub(super) fn lang(&mut self) -> Result<LangCode> {
        if self.pos + LANG_LEN + LEN_FIELD > self.body.len() {
            return Err(self.err("entry header runs past descriptor end"));
        }
        let b = self.body;
        let p = self.pos;
        self.pos += LANG_LEN;
        Ok(LangCode([b[p], b[p + 1], b[p + 2]]))
    }

    /// Read one length-prefixed text. `reserve` is the number of bytes that
    /// must still follow the text (e.g. the next `text_length` byte of a
    /// two-string entry); a text whose end plus `reserve` overruns the body
    /// is rejected with `overrun_reason`.
    pub(super) fn text(
        &mut self,
        overrun_reason: &'static str,
        reserve: usize,
    ) -> Result<DvbText<'a>> {
        // Checked indexing throughout: a missing length byte or an overrun is
        // an error, never a panic, whatever the caller's `lang()` history.
        let len = usize::from(
            *self
                .body
                .get(self.pos)
                .ok_or_else(|| self.err("entry header runs past descriptor end"))?,
        );
        let start = self.pos + LEN_FIELD;
        let end = start + len;
        let text = (end + reserve <= self.body.len())
            .then(|| self.body.get(start..end))
            .flatten()
            .ok_or_else(|| self.err(overrun_reason))?;
        self.pos = end;
        Ok(DvbText::new(text))
    }
}

/// Write `lang` at `pos`, returning the position after it.
pub(super) fn write_lang(buf: &mut [u8], pos: usize, lang: &LangCode) -> usize {
    buf[pos..pos + LANG_LEN].copy_from_slice(&lang.0);
    pos + LANG_LEN
}

/// Write one length-prefixed text at `pos`, returning the position after it.
/// The length goes through the checked `fit_u8` (a text over 255 bytes is
/// `FieldOverflow`, never a wrapped length).
pub(super) fn write_text(
    buf: &mut [u8],
    pos: usize,
    text: &DvbText<'_>,
    length_field: &'static str,
) -> Result<usize> {
    buf[pos] = broadcast_common::len::fit_u8(text.len(), length_field)?;
    let start = pos + LEN_FIELD;
    buf[start..start + text.len()].copy_from_slice(text.raw());
    Ok(start + text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text()` on an exhausted or truncated body is an error, not a panic.
    #[test]
    fn text_never_panics_on_short_bodies() {
        let mut r = EntryReader::new(&[], 0, 0x5C);
        assert!(r.text("overrun", 0).is_err());
        let mut r = EntryReader::new(&[5, b'a'], 0, 0x5C);
        assert!(r.text("overrun", 0).is_err());
        let mut r = EntryReader::new(&[1, b'a'], 0, 0x5C);
        assert!(r.text("overrun", 1).is_err());
        let mut r = EntryReader::new(&[1, b'a'], 0, 0x5C);
        assert_eq!(r.text("overrun", 0).unwrap().raw(), b"a");
    }
}

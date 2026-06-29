//! High-Level MMI objects — ETSI EN 50221 §8.6.5, Tables 46-51 (PDF pp. 47-50).
//!
//! - `text` (`9F 88 03` last / `9F 88 04` more, Table 46) — a text-char run; used
//!   standalone and as the `TEXT()` component of Menu / List.
//! - `enq` (`9F 88 07`, Table 47) — request a single user input (e.g. a PIN).
//! - `answ` (`9F 88 08`, Table 48) — the user input reply.
//! - `menu` (`9F 88 09` last / `9F 88 0A` more, Table 49) — title/sub/bottom +
//!   choices.
//! - `menu_answ` (`9F 88 0B`, Table 50) — the chosen `choice_ref`.
//! - `list` (`9F 88 0C` last / `9F 88 0D` more, Table 51) — same shape as Menu.

use crate::error::{Error, Result};
use crate::length;
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// `text()` object (Table 46): a run of `text_char` bytes (EN 300 468 Annex A).
///
/// The `_last` (`9F 88 03`) and `_more` (`9F 88 04`) tags share this one body;
/// [`Text::more`] selects which tag is written and is set from the parsed tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Text<'a> {
    /// `true` = `text_more` (more APDUs follow); `false` = `text_last`.
    pub more: bool,
    /// The `text_char` bytes (may be empty: a null text object).
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub text_chars: &'a [u8],
}

impl<'a> Text<'a> {
    /// The `apdu_tag` this text object serializes to, given [`Text::more`].
    #[must_use]
    pub fn tag(&self) -> ApduTag {
        if self.more {
            tag::TEXT_MORE
        } else {
            tag::TEXT_LAST
        }
    }

    /// Parse a nested `TEXT()` component at the front of `bytes`, returning the
    /// object and the number of bytes it consumed (tag + length + chars).
    pub(crate) fn parse_component(bytes: &'a [u8]) -> Result<(Self, usize)> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "text component tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let more = match t {
            tag::TEXT_LAST => false,
            tag::TEXT_MORE => true,
            _ => {
                return Err(Error::UnexpectedApduTag {
                    got: t.as_u24(),
                    expected: tag::TEXT_LAST.as_u24(),
                    what: "text component",
                })
            }
        };
        let (len_value, len_hdr) = length::decode(&bytes[3..])?;
        let body_start = 3 + len_hdr;
        let body_end = body_start + len_value;
        if bytes.len() < body_end {
            return Err(Error::LengthMismatch {
                what: "text component",
                declared: len_value,
                actual: bytes.len().saturating_sub(body_start),
            });
        }
        Ok((
            Self {
                more,
                text_chars: &bytes[body_start..body_end],
            },
            body_end,
        ))
    }

    /// Serialized length of this `TEXT()` component (tag + length + chars).
    pub(crate) fn component_len(&self) -> usize {
        super::apdu_len(self.text_chars.len())
    }

    /// Serialize this `TEXT()` component into the front of `buf`.
    pub(crate) fn serialize_component(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(self.tag(), self.text_chars.len(), buf)?;
        buf[pos..pos + self.text_chars.len()].copy_from_slice(self.text_chars);
        pos += self.text_chars.len();
        Ok(pos)
    }
}

impl<'a> Parse<'a> for Text<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (t, consumed) = Self::parse_component(bytes)?;
        // The whole APDU must be exactly this component.
        if consumed != bytes.len() {
            return Err(Error::LengthMismatch {
                what: "text",
                declared: consumed,
                actual: bytes.len(),
            });
        }
        Ok(t)
    }
}

impl Serialize for Text<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.component_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_component(buf)
    }
}

impl<'a> ApduDef<'a> for Text<'a> {
    const TAG: ApduTag = tag::TEXT_LAST;
    const NAME: &'static str = "TEXT";
}

/// `enq()` object (Table 47): request a single user input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Enq<'a> {
    /// `blind_answer` — when set, the user input is not displayed as typed.
    pub blind_answer: bool,
    /// `answer_text_length` — expected answer length (`0xFF` = unknown).
    pub answer_text_length: u8,
    /// The prompt `text_char` bytes (EN 300 468 Annex A).
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub text_chars: &'a [u8],
}

// reserved/blind_answer(1) + answer_text_length(1).
const ENQ_PREFIX: usize = 2;

impl<'a> Parse<'a> for Enq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::ENQ, "enq")?;
        if body.len() < ENQ_PREFIX {
            return Err(Error::BufferTooShort {
                need: ENQ_PREFIX,
                have: body.len(),
                what: "enq",
            });
        }
        Ok(Self {
            blind_answer: (body[0] & 0x01) != 0,
            answer_text_length: body[1],
            text_chars: &body[ENQ_PREFIX..],
        })
    }
}

impl Serialize for Enq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(ENQ_PREFIX + self.text_chars.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = ENQ_PREFIX + self.text_chars.len();
        let mut pos = super::write_apdu_header(tag::ENQ, body_len, buf)?;
        // reserved(7)='1111111', blind_answer(1).
        buf[pos] = 0xFE | u8::from(self.blind_answer);
        buf[pos + 1] = self.answer_text_length;
        pos += ENQ_PREFIX;
        buf[pos..pos + self.text_chars.len()].copy_from_slice(self.text_chars);
        pos += self.text_chars.len();
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for Enq<'a> {
    const TAG: ApduTag = tag::ENQ;
    const NAME: &'static str = "ENQ";
}

/// `answ_id` values (Table 48, p. 48).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AnswId {
    /// `00` — the user wishes to abort the dialogue.
    Cancel,
    /// `01` — the object contains the user input.
    Answer,
    /// Any other value (reserved).
    Reserved(u8),
}

impl AnswId {
    /// Decode an `answ_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Cancel,
            0x01 => Self::Answer,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Cancel => 0x00,
            Self::Answer => 0x01,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Answer => "answer",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(AnswId, Reserved);

/// `answ()` object (Table 48): the user input reply.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Answ<'a> {
    /// `answ_id`.
    pub answ_id: AnswId,
    /// The answer `text_char` bytes — present only when `answ_id == answer`.
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub text_chars: &'a [u8],
}

impl<'a> Parse<'a> for Answ<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::ANSW, "answ")?;
        let id_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "answ answ_id",
        })?;
        let answ_id = AnswId::from_u8(id_byte);
        let text_chars = if answ_id == AnswId::Answer {
            &body[1..]
        } else {
            &body[..0]
        };
        Ok(Self {
            answ_id,
            text_chars,
        })
    }
}

impl Serialize for Answ<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let body = 1 + if self.answ_id == AnswId::Answer {
            self.text_chars.len()
        } else {
            0
        };
        super::apdu_len(body)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let with_text = self.answ_id == AnswId::Answer;
        let body_len = 1 + if with_text { self.text_chars.len() } else { 0 };
        let mut pos = super::write_apdu_header(tag::ANSW, body_len, buf)?;
        buf[pos] = self.answ_id.to_u8();
        pos += 1;
        if with_text {
            buf[pos..pos + self.text_chars.len()].copy_from_slice(self.text_chars);
            pos += self.text_chars.len();
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for Answ<'a> {
    const TAG: ApduTag = tag::ANSW;
    const NAME: &'static str = "ANSW";
}

/// `menu()` object (Table 49) and `list()` object (Table 51) share this layout:
/// `choice_nb` / `item_nb`, three header TEXT()s, then the choice/item TEXT()s.
///
/// [`Menu::more`] selects the `_last` vs `_more` tag and is set from the parsed
/// tag. `0xFF` for the count means "count not carried; read to end of object".
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Menu<'a> {
    /// `true` = `_more` tag (more APDUs follow); `false` = `_last`.
    pub more: bool,
    /// `choice_nb` / `item_nb` — `0xFF` means the count is not carried on the wire.
    pub choice_nb: u8,
    /// Title TEXT().
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub title: Text<'a>,
    /// Sub-title TEXT().
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub subtitle: Text<'a>,
    /// Bottom-line TEXT().
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub bottom: Text<'a>,
    /// The per-choice / per-item TEXT()s, in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub choices: Vec<Text<'a>>,
}

/// `list()` object (Table 51) — byte-identical to `menu()` except for its tag
/// pair (`9F 88 0C` last / `9F 88 0D` more). Wraps a [`Menu`]; the inner
/// `Menu::more` selects `_last` vs `_more`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct List<'a>(
    /// The shared menu/list body.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub Menu<'a>,
);

impl<'a> Parse<'a> for List<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Menu::parse_list(bytes).map(List)
    }
}

impl Serialize for List<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.0.list_serialized_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.0.serialize_list(buf)
    }
}

impl<'a> ApduDef<'a> for List<'a> {
    const TAG: ApduTag = tag::LIST_LAST;
    const NAME: &'static str = "LIST";
}

impl<'a> Menu<'a> {
    fn parse_with_tag(bytes: &'a [u8], expected: ApduTag, what: &'static str) -> Result<Self> {
        let more = expected == tag::MENU_MORE || expected == tag::LIST_MORE;
        let body = super::parse_apdu_header(bytes, expected, what)?;
        let choice_nb = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what,
        })?;
        let mut pos = 1usize;
        let (title, n) = Text::parse_component(&body[pos..])?;
        pos += n;
        let (subtitle, n) = Text::parse_component(&body[pos..])?;
        pos += n;
        let (bottom, n) = Text::parse_component(&body[pos..])?;
        pos += n;
        let mut choices = Vec::new();
        while pos < body.len() {
            let (c, n) = Text::parse_component(&body[pos..])?;
            pos += n;
            choices.push(c);
        }
        Ok(Self {
            more,
            choice_nb,
            title,
            subtitle,
            bottom,
            choices,
        })
    }

    fn body_len(&self) -> usize {
        let mut n = 1
            + self.title.component_len()
            + self.subtitle.component_len()
            + self.bottom.component_len();
        for c in &self.choices {
            n += c.component_len();
        }
        n
    }

    fn serialize_with_tag(&self, t: ApduTag, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        let mut pos = super::write_apdu_header(t, body_len, buf)?;
        buf[pos] = self.choice_nb;
        pos += 1;
        pos += self.title.serialize_component(&mut buf[pos..])?;
        pos += self.subtitle.serialize_component(&mut buf[pos..])?;
        pos += self.bottom.serialize_component(&mut buf[pos..])?;
        for c in &self.choices {
            pos += c.serialize_component(&mut buf[pos..])?;
        }
        Ok(pos)
    }

    /// The menu `apdu_tag` for this object given [`Menu::more`].
    #[must_use]
    pub fn menu_tag(&self) -> ApduTag {
        if self.more {
            tag::MENU_MORE
        } else {
            tag::MENU_LAST
        }
    }

    /// The list `apdu_tag` for this object given [`Menu::more`] (for use as a
    /// [`List`]).
    #[must_use]
    pub fn list_tag(&self) -> ApduTag {
        if self.more {
            tag::LIST_MORE
        } else {
            tag::LIST_LAST
        }
    }

    /// Parse a `list()` object (Tables 51) from a complete APDU.
    pub fn parse_list(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "list tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let expected = if t == tag::LIST_MORE {
            tag::LIST_MORE
        } else {
            tag::LIST_LAST
        };
        Self::parse_with_tag(bytes, expected, "list")
    }

    /// Serialized length of this object as a `list()` (identical to a menu).
    #[must_use]
    pub fn list_serialized_len(&self) -> usize {
        super::apdu_len(self.body_len())
    }

    /// Serialize this object as a `list()` into `buf`.
    pub fn serialize_list(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_with_tag(self.list_tag(), buf)
    }
}

impl<'a> Parse<'a> for Menu<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "menu tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let expected = if t == tag::MENU_MORE {
            tag::MENU_MORE
        } else {
            tag::MENU_LAST
        };
        Self::parse_with_tag(bytes, expected, "menu")
    }
}

impl Serialize for Menu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(self.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_with_tag(self.menu_tag(), buf)
    }
}

impl<'a> ApduDef<'a> for Menu<'a> {
    const TAG: ApduTag = tag::MENU_LAST;
    const NAME: &'static str = "MENU";
}

/// `menu_answ()` object (Table 50): the chosen `choice_ref` (also used to close a
/// list). `00` = the user cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MenuAnsw {
    /// `choice_ref` — `01` = first choice, …; `00` = cancelled.
    pub choice_ref: u8,
}

impl<'a> Parse<'a> for MenuAnsw {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::MENU_ANSW, "menu_answ")?;
        let choice_ref = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "menu_answ choice_ref",
        })?;
        Ok(Self { choice_ref })
    }
}

impl Serialize for MenuAnsw {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::MENU_ANSW, 1, buf)?;
        buf[pos] = self.choice_ref;
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for MenuAnsw {
    const TAG: ApduTag = tag::MENU_ANSW;
    const NAME: &'static str = "MENU_ANSW";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(more: bool, s: &[u8]) -> Text<'_> {
        Text {
            more,
            text_chars: s,
        }
    }

    #[test]
    fn text_round_trips_and_more_bites() {
        let t = text(false, b"HELLO");
        let bytes = t.to_bytes();
        assert_eq!(
            bytes,
            [0x9F, 0x88, 0x03, 0x05, b'H', b'E', b'L', b'L', b'O']
        );
        assert_eq!(Text::parse(&bytes).unwrap(), t);
        // flipping `more` flips the tag byte.
        let mut other = t.clone();
        other.more = true;
        let ob = other.to_bytes();
        assert_eq!(ob[2], 0x04);
        assert_ne!(bytes, ob);
    }

    #[test]
    fn text_empty_is_null_object() {
        let t = text(false, b"");
        let bytes = t.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x03, 0x00]);
        assert_eq!(Text::parse(&bytes).unwrap(), t);
    }

    #[test]
    fn enq_pin_example_round_trips() {
        // Spec worked example: enq "PLEASE TYPE YOUR PIN CODE", blind_answer=1.
        let prompt = b"PLEASE TYPE YOUR PIN CODE";
        let e = Enq {
            blind_answer: true,
            answer_text_length: 4,
            text_chars: prompt,
        };
        let bytes = e.to_bytes();
        // tag 9F8807, body = reserved|blind(0xFF) + answer_text_length(0x04) + prompt
        assert_eq!(&bytes[..3], &[0x9F, 0x88, 0x07]);
        assert_eq!(bytes[4], 0xFF); // reserved(7)=1 + blind_answer=1
        assert_eq!(bytes[5], 0x04);
        let parsed = Enq::parse(&bytes).unwrap();
        assert_eq!(parsed, e);
        assert!(parsed.blind_answer);
        // bite: clear blind_answer changes the byte.
        let mut other = e.clone();
        other.blind_answer = false;
        let ob = other.to_bytes();
        assert_eq!(ob[4], 0xFE);
        assert_ne!(bytes, ob);
    }

    #[test]
    fn answ_answer_and_cancel() {
        let a = Answ {
            answ_id: AnswId::Answer,
            text_chars: b"1234",
        };
        let bytes = a.to_bytes();
        assert_eq!(
            bytes,
            [0x9F, 0x88, 0x08, 0x05, 0x01, b'1', b'2', b'3', b'4']
        );
        assert_eq!(Answ::parse(&bytes).unwrap(), a);

        // cancel carries no text (even if text_chars set, it is dropped).
        let c = Answ {
            answ_id: AnswId::Cancel,
            text_chars: &[],
        };
        let cb = c.to_bytes();
        assert_eq!(cb, [0x9F, 0x88, 0x08, 0x01, 0x00]);
        assert_eq!(Answ::parse(&cb).unwrap(), c);
        assert_eq!(c.answ_id.name(), "cancel");

        // bite
        let mut other = a.clone();
        other.text_chars = b"9999";
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn menu_two_choice_example_round_trips_and_bites() {
        // Spec worked example: menu "DO YOU WANT TO BUY?" / "JURASSIC PARK"
        // with two choices.
        let m = Menu {
            more: false,
            choice_nb: 2,
            title: text(false, b"DO YOU WANT TO BUY?"),
            subtitle: text(false, b"JURASSIC PARK"),
            bottom: text(false, b""),
            choices: alloc::vec![text(false, b"YES"), text(false, b"NO")],
        };
        let bytes = m.to_bytes();
        assert_eq!(&bytes[..3], &[0x9F, 0x88, 0x09]);
        let parsed = Menu::parse(&bytes).unwrap();
        assert_eq!(parsed, m);
        assert_eq!(parsed.choices.len(), 2);
        assert_eq!(parsed.title.text_chars, b"DO YOU WANT TO BUY?");

        // hand-compute the body length: choice_nb(1) + title(4+19) + sub(4+13)
        // + bottom(4+0) + YES(4+3) + NO(4+2) = 1+23+17+4+7+6 = 58.
        assert_eq!(bytes[3], 58);

        // bite: mutate choice_nb.
        let mut other = m.clone();
        other.choice_nb = 0xFF;
        assert_ne!(bytes, other.to_bytes());

        // more bite: flipping `more` flips the tag.
        let mut more = m.clone();
        more.more = true;
        let mb = more.to_bytes();
        assert_eq!(mb[2], 0x0A);
        assert_ne!(bytes, mb);
    }

    #[test]
    fn list_uses_list_tags_and_round_trips() {
        let l = List(Menu {
            more: false,
            choice_nb: 0xFF, // count not carried
            title: text(false, b"ENTITLEMENTS"),
            subtitle: text(false, b""),
            bottom: text(false, b""),
            choices: alloc::vec![text(false, b"A"), text(false, b"B"), text(false, b"C")],
        });
        let bytes = l.to_bytes();
        assert_eq!(&bytes[..3], &[0x9F, 0x88, 0x0C]);
        let parsed = List::parse(&bytes).unwrap();
        assert_eq!(parsed, l);
        assert_eq!(parsed.0.choices.len(), 3);

        // list more tag flips.
        let mut more = l.clone();
        more.0.more = true;
        let mb = more.to_bytes();
        assert_eq!(mb[2], 0x0D);
        assert_eq!(List::parse(&mb).unwrap(), more);
        assert_ne!(bytes, mb);
    }

    #[test]
    fn menu_answ_round_trips_and_bites() {
        let a = MenuAnsw { choice_ref: 0x02 };
        let bytes = a.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x0B, 0x01, 0x02]);
        assert_eq!(MenuAnsw::parse(&bytes).unwrap(), a);
        let other = MenuAnsw { choice_ref: 0x00 };
        assert_ne!(bytes, other.to_bytes());
    }
}

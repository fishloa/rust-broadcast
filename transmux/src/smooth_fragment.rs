//! Client-side inspection of Smooth Streaming documents and fragment responses: the pure,
//! sans-IO checks a Smooth *puller* needs before it hands bytes to [`crate::media::Fmp4Demux`].
//!
//! Smooth Streaming: MS-SSTR (`private/specs-verified/mpeg/transmux/smooth/ms-sstr.md`).
//!
//! - Manifest `<Protection>` element (the PlayReady/PIFF content-protection header) is found by
//!   [`manifest_declares_protection`] over a real `quick-xml` reader.
//! - Fragment Response (§2.2.4): `moof( mfhd traf( tfhd trun tfxd [tfrf] ) ) mdat`.
//!   [`fragment_state`] classifies a fetched fragment as clear, sample-encrypted
//!   (`senc`/`saiz`/`saio` or the PIFF sample-encryption `uuid` box in a `traf`, ISO/IEC 14496-12:2026
//!   §8.8.7/§8.8.8) or undeterminable; [`fragment_track_id`] reads the wire `tfhd@track_ID`
//!   (a manifest carries none) and [`fragment_tfxd`] the `tfxd` box (§2.2.4.4) with the
//!   fragment's absolute time and duration.
//!
//! The same walkers serve `multimux`'s Smooth-pull and `broadcast-client`'s Smooth source, so a
//! Smooth fragment is judged by one box walk.

use broadcast_common::xml_chars::{XmlBudget, namespace_of};
use quick_xml::NsReader;
use quick_xml::events::Event;

use crate::box_types::{box_iter, parse_box};
use crate::error::{Error, Result};
use crate::movie_fragment::MovieFragmentBox;
use crate::smooth::{TFXD_UUID, TfxdBox};
use crate::smooth_parse::default_limits;

const PROTECTION_ELEMENT: &str = "Protection";
const BOX_MOOF: [u8; 4] = *b"moof";
const BOX_TRAF: [u8; 4] = *b"traf";
const BOX_UUID: [u8; 4] = *b"uuid";
const BOX_SENC: [u8; 4] = *b"senc";
const BOX_SAIZ: [u8; 4] = *b"saiz";
const BOX_SAIO: [u8; 4] = *b"saio";
const BOX_HEADER_LEN: usize = 8;
const BOX_SIZE_LEN: usize = 4;

/// The PIFF "UUID Sample Encryption Box" extended type
/// (`A2394F52-5A9B-4F14-A244-6C427C648DF4`).
pub const PIFF_SAMPLE_ENCRYPTION_UUID: [u8; 16] = [
    0xA2, 0x39, 0x4F, 0x52, 0x5A, 0x9B, 0x4F, 0x14, 0xA2, 0x44, 0x6C, 0x42, 0x7C, 0x64, 0x8D, 0xF4,
];

/// How [`fragment_state`]'s box walk classified a fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FragmentState {
    /// No encryption box anywhere; safe to demux.
    Clear,
    /// A `senc`/`saiz`/`saio` (or PIFF `uuid`) box inside a `moof`/`traf`.
    Encrypted,
    /// A malformed or truncated `moof` (or a top-level box that does not parse): it cannot be
    /// proven clear and is not provably encrypted either.
    Undetermined,
}

impl FragmentState {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Encrypted => "encrypted",
            Self::Undetermined => "undetermined",
        }
    }
}

broadcast_common::impl_spec_display!(FragmentState);

/// Whether the manifest carries a `<Protection>` element (PlayReady/PIFF sample encryption)
/// anywhere, matched on a namespace-aware `quick-xml` pull loop under the same bounds as
/// [`crate::smooth_parse::SmoothManifest::parse`]: only an element in no namespace counts (a
/// Smooth manifest has none), so a `<ms:Protection>` or default-namespaced look-alike in a
/// foreign namespace, a comment, an attribute value or a longer name such as `<ProtectionFoo>`
/// does not.
///
/// A document the reader cannot get through reports `false`; the caller goes on to
/// [`crate::smooth_parse::SmoothManifest::parse`], which rejects the same input with its own
/// error, so nothing unreadable is ever played.
#[must_use]
pub fn manifest_declares_protection(xml: &str) -> bool {
    let mut budget = XmlBudget::new(default_limits());
    if budget.check_input(xml.len()).is_err() {
        return false;
    }
    let mut reader = NsReader::from_str(xml);
    loop {
        match reader.read_resolved_event() {
            Ok((ns, Event::Start(e))) => {
                if budget.open_element().is_err() {
                    return false;
                }
                if matches!(namespace_of(ns), Ok(None))
                    && e.local_name().as_ref() == PROTECTION_ELEMENT
                {
                    return true;
                }
            }
            Ok((ns, Event::Empty(e))) => {
                if budget.open_element().is_err() {
                    return false;
                }
                budget.close_element();
                if matches!(namespace_of(ns), Ok(None))
                    && e.local_name().as_ref() == PROTECTION_ELEMENT
                {
                    return true;
                }
            }
            Ok((_, Event::End(_))) => budget.close_element(),
            Ok((_, Event::Eof)) | Err(_) => return false,
            Ok(_) => {}
        }
    }
}

/// Classify a fetched fragment, distinguishing a genuinely encrypted one from a malformed or
/// truncated one so the caller's error is accurate.
///
/// Only the `moof` box and its `traf` children are inspected (`senc`/`saiz`/`saio` and the PIFF
/// `uuid` can only legitimately sit there); a byte scan of the whole fragment would trip on a
/// four-byte run in clear `mdat` payload. A fragment that cannot be walked to the end after a
/// `moof` was seen fails closed as [`FragmentState::Undetermined`].
#[must_use]
pub fn fragment_state(bytes: &[u8]) -> FragmentState {
    let mut offset = 0usize;
    let mut saw_moof = false;
    while offset + BOX_HEADER_LEN <= bytes.len() {
        let size = u32::from_be_bytes(
            bytes[offset..offset + BOX_SIZE_LEN]
                .try_into()
                .unwrap_or([0; BOX_SIZE_LEN]),
        );
        let box_type = &bytes[offset + BOX_SIZE_LEN..offset + BOX_HEADER_LEN];
        let is_moof = box_type == BOX_MOOF;
        if is_moof {
            saw_moof = true;
        }
        let size = size as usize;
        if size < BOX_HEADER_LEN || offset + size > bytes.len() {
            return if saw_moof {
                FragmentState::Undetermined
            } else {
                FragmentState::Clear
            };
        }
        if is_moof {
            match moof_is_encrypted(&bytes[offset + BOX_HEADER_LEN..offset + size]) {
                Some(true) => return FragmentState::Encrypted,
                None => return FragmentState::Undetermined,
                Some(false) => {}
            }
        }
        offset += size;
    }
    if saw_moof && offset < bytes.len() {
        FragmentState::Undetermined
    } else {
        FragmentState::Clear
    }
}

fn moof_is_encrypted(moof_body: &[u8]) -> Option<bool> {
    for result in box_iter(moof_body) {
        let Ok((child, _)) = result else {
            return None;
        };
        if !child.header.box_type.is(&BOX_TRAF) {
            continue;
        }
        for result in box_iter(child.body) {
            let Ok((grandchild, _)) = result else {
                return None;
            };
            let t = grandchild.header.box_type;
            if t.is(&BOX_SENC) || t.is(&BOX_SAIZ) || t.is(&BOX_SAIO) {
                return Some(true);
            }
            if t.is(&BOX_UUID)
                && grandchild.header.usertype.as_ref() == Some(&PIFF_SAMPLE_ENCRYPTION_UUID)
            {
                return Some(true);
            }
        }
    }
    Some(false)
}

/// The wire `tfhd@track_ID` of a fragment's first `traf`, read without any `moov`.
///
/// A Smooth manifest carries no track id, yet every fragment's `tfhd` must match a `trak` of the
/// init segment the client synthesises, so the id is taken from the first fragment fetched.
///
/// # Errors
/// [`Error::UnexpectedBox`] when the fragment has no `moof` or its `moof` no `traf`; the box
/// parser's own error for a malformed box.
pub fn fragment_track_id(fragment: &[u8]) -> Result<u32> {
    let mut offset = 0usize;
    while offset + BOX_HEADER_LEN <= fragment.len() {
        let (bx, consumed) = parse_box(&fragment[offset..])?;
        if bx.header.box_type.is(&BOX_MOOF) {
            let moof = MovieFragmentBox::parse_body(bx.body)?;
            let traf = moof
                .traf
                .first()
                .ok_or(Error::UnexpectedBox { expected: "traf" })?;
            return Ok(traf.tfhd.track_id.value());
        }
        if consumed == 0 {
            break;
        }
        offset += consumed;
    }
    Err(Error::UnexpectedBox { expected: "moof" })
}

/// The `tfxd` box (MS-SSTR 2.2.4.4) of a fragment's first `traf`, `None` when the fragment
/// carries none (a server may omit it; the request's own start time then stands in) or the
/// `moof` cannot be walked.
#[must_use]
pub fn fragment_tfxd(fragment: &[u8]) -> Option<TfxdBox> {
    for result in box_iter(fragment) {
        let (bx, _) = result.ok()?;
        if !bx.header.box_type.is(&BOX_MOOF) {
            continue;
        }
        for child in box_iter(bx.body) {
            let (traf, _) = child.ok()?;
            if !traf.header.box_type.is(&BOX_TRAF) {
                continue;
            }
            for grandchild in box_iter(traf.body) {
                let (g, _) = grandchild.ok()?;
                if g.header.box_type.is(&BOX_UUID) && g.header.usertype.as_ref() == Some(&TFXD_UUID)
                {
                    return TfxdBox::parse_body(g.body).ok();
                }
            }
            return None;
        }
        return None;
    }
    None
}

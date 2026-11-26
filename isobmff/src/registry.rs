//! Runtime registration of private ISO BMFF boxes and sample entries.
//!
//! [`BoxRegistry`] lets a crate outside this one type a box that this crate
//! does not model, without editing it: register a type implementing [`BoxDef`]
//! (its four-CC or `uuid` extended type, an optional parent scope, and the
//! usual `Parse`/`Serialize` pair), hand the registry to
//! [`ReaderConfig::with_registry`](crate::reader::ReaderConfig::with_registry),
//! and a matching `stsd` entry or child box comes back typed instead of as an
//! opaque blob. Writing needs no registry: [`SampleEntry::from_registered`](crate::writer::SampleEntry::from_registered)
//! and [`RegisteredEntry::try_to_bytes`](RegisteredEntry::try_to_bytes) turn the
//! value back into the box bytes a [`SampleEntry::Raw`](crate::writer::SampleEntry::Raw) or an opaque child
//! carries.
//!
//! Modelled on `dvb_si::descriptors::DescriptorRegistry`: registration is keyed
//! by the wire identity (four-CC, or `uuid` extended type) plus an optional
//! parent four-CC, a scoped registration wins over an unscoped one, and an
//! unregistered box stays an opaque box that writes back byte-identically.
//!
//! Registration is a `fn` pointer per entry and the parsed value is a boxed
//! trait object with a clone hook, so a [`BoxRegistry`] is `Clone` and can sit
//! in a [`ReaderConfig`](crate::reader::ReaderConfig) whose value is cloned
//! freely.
//!
//! ISO/IEC 14496-12:2026 §4.2 (box header, `uuid` extended type),
//! §8.5.2 (sample description).
//!
//! # Example
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use isobmff::registry::{BoxDef, BoxRegistry};
//! use isobmff::Error;
//!
//! #[derive(Debug, Clone, PartialEq)]
//! struct Vendor(u8);
//!
//! impl<'a> Parse<'a> for Vendor {
//!     type Error = Error;
//!     fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
//!         let (item, _) = isobmff::parse_box(bytes)?;
//!         item.body.first().map(|&b| Self(b)).ok_or(Error::InvalidInput("empty vendor box"))
//!     }
//! }
//!
//! impl Serialize for Vendor {
//!     type Error = Error;
//!     fn serialized_len(&self) -> usize {
//!         8 + 1
//!     }
//!     fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Error> {
//!         let header = isobmff::box_types::write_box_header(buf, b"vend", 9)?;
//!         buf[header] = self.0;
//!         Ok(9)
//!     }
//! }
//!
//! impl BoxDef for Vendor {
//!     const FOURCC: [u8; 4] = *b"vend";
//! }
//!
//! let mut registry = BoxRegistry::new();
//! registry.register::<Vendor>().expect("register");
//! ```

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use broadcast_core::{Parse, Serialize};

use crate::box_types::{BOX_TYPE_SIZE, BoxIter, UUID, UUID_TYPE_SIZE, parse_box};
use crate::error::{Error, Result};

/// The parsed value of a registered box, cloneable and byte-comparable.
///
/// The value is held as a boxed [`RegisteredBox`]; [`Clone`] re-boxes it
/// through the type's own clone hook and [`PartialEq`] compares its wire
/// bytes, so a track table holding one still derives both as it always did.
pub struct RegisteredEntry(Box<dyn RegisteredBox>);

impl RegisteredEntry {
    fn wrap(value: Box<dyn RegisteredBox>) -> Self {
        Self(value)
    }

    /// The four-CC of the box this value is.
    #[must_use]
    pub fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        self.0.fourcc()
    }

    /// The value as `&dyn Any`, for [`RegisteredEntry::downcast_ref`].
    #[must_use]
    pub fn as_any(&self) -> &dyn Any {
        self.0.as_any()
    }

    /// The value as its concrete registered type, when `T` is that type.
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// The box as the object-safe trait object the writer takes.
    #[must_use]
    pub fn as_registered(&self) -> &dyn RegisteredBox {
        &*self.0
    }

    /// Number of bytes the box occupies, header included.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        self.0.serialized_len()
    }

    /// Write the complete box, header included.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.0.serialize_into(buf)
    }

    /// The complete box bytes, header included.
    pub fn try_to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; self.0.serialized_len()];
        let written = self.0.serialize_into(&mut out)?;
        out.truncate(written);
        Ok(out)
    }
}

impl Clone for RegisteredEntry {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl PartialEq for RegisteredEntry {
    fn eq(&self, other: &Self) -> bool {
        self.fourcc() == other.fourcc() && self.try_to_bytes().ok() == other.try_to_bytes().ok()
    }
}

impl Eq for RegisteredEntry {}

impl fmt::Debug for RegisteredEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A type a caller can register: its wire identity plus the symmetric
/// [`Parse`]/[`Serialize`] pair every box in this crate implements.
///
/// [`BoxDef::UUID`] makes the box a `uuid` box, keyed by its 16-byte extended
/// type; the `type` field of such a box is `uuid` and [`BoxDef::FOURCC`] is
/// ignored for dispatch. [`BoxDef::PARENT`] scopes the registration to one
/// parent box (a `udta`-only atom, say); an unscoped registration matches in
/// every parent, and a scoped one wins over it.
///
/// `Parse` receives the complete box bytes, header included, as every parser in
/// this crate does.
pub trait BoxDef:
    for<'a> Parse<'a, Error = Error>
    + Serialize<Error = Error>
    + Clone
    + fmt::Debug
    + Send
    + Sync
    + 'static
{
    /// The box's four-CC.
    const FOURCC: [u8; BOX_TYPE_SIZE];
    /// The `uuid` extended type, when the box is a `uuid` box (§4.2).
    const UUID: Option<[u8; UUID_TYPE_SIZE]> = None;
    /// The parent box this registration is scoped to, when it is scoped.
    const PARENT: Option<[u8; BOX_TYPE_SIZE]> = None;
}

/// Object-safe face of a registered box value.
///
/// Implemented for every [`BoxDef`]; a caller never writes this by hand.
/// `Send + Sync`, so a reader holding registered values can move between
/// threads and one registry can serve many readers.
pub trait RegisteredBox: fmt::Debug + Any + Send + Sync {
    /// The four-CC of the box.
    fn fourcc(&self) -> [u8; BOX_TYPE_SIZE];
    /// The value as `&dyn Any`.
    fn as_any(&self) -> &dyn Any;
    /// A boxed copy of the value.
    fn clone_box(&self) -> Box<dyn RegisteredBox>;
    /// Number of bytes the box occupies, header included.
    fn serialized_len(&self) -> usize;
    /// Write the complete box, header included.
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize>;
}

impl<T: BoxDef> RegisteredBox for T {
    fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        T::FOURCC
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn RegisteredBox> {
        Box::new(self.clone())
    }

    fn serialized_len(&self) -> usize {
        Serialize::serialized_len(self)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        Serialize::serialize_into(self, buf)
    }
}

/// The wire identity a registration is keyed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BoxKey {
    /// A box addressed by its four-CC.
    FourCc([u8; BOX_TYPE_SIZE]),
    /// A `uuid` box addressed by its extended type (§4.2).
    Uuid([u8; UUID_TYPE_SIZE]),
}

/// The parse closure type one registration holds.
type ParseFn = fn(&[u8]) -> Result<Box<dyn RegisteredBox>>;

#[derive(Clone)]
struct Registration {
    parse: ParseFn,
}

/// One registry entry's key: the optional parent scope plus the box identity.
type RegistryKey = (Option<[u8; BOX_TYPE_SIZE]>, BoxKey);

/// A runtime table of box types registered from outside this crate.
///
/// See the [module docs](self). An empty registry leaves every read exactly as
/// it was: unknown box types come back as their opaque bytes.
#[derive(Clone, Default)]
pub struct BoxRegistry {
    entries: BTreeMap<RegistryKey, Registration>,
}

impl BoxRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `T` for its [`BoxDef::FOURCC`] / [`BoxDef::UUID`].
    ///
    /// A key that is already registered, or a four-CC this crate types itself,
    /// is [`Error::BoxTypeRegistered`]; use
    /// [`register_override`](BoxRegistry::register_override) to replace it
    /// deliberately.
    pub fn register<T: BoxDef>(&mut self) -> Result<&mut Self> {
        self.insert::<T>(false)
    }

    /// Register `T`, replacing an existing registration or this crate's own
    /// type for the same key.
    pub fn register_override<T: BoxDef>(&mut self) -> Result<&mut Self> {
        self.insert::<T>(true)
    }

    /// Whether no type is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of registered types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Parse the box in `bytes` when a registration matches it.
    ///
    /// `parent` names the enclosing box's four-CC, so a scoped registration can
    /// match; pass `None` at the top level. A matching scoped registration wins
    /// over an unscoped one. `None` means no registration matched and the
    /// caller should keep its own handling (an opaque box).
    pub fn parse(
        &self,
        parent: Option<[u8; BOX_TYPE_SIZE]>,
        bytes: &[u8],
    ) -> Result<Option<RegisteredEntry>> {
        let Some(key) = box_key(bytes)? else {
            return Ok(None);
        };
        let entry = parent
            .and_then(|scope| self.entries.get(&(Some(scope), key)))
            .or_else(|| self.entries.get(&(None, key)));
        match entry {
            Some(registration) => Ok(Some(RegisteredEntry::wrap((registration.parse)(bytes)?))),
            None => Ok(None),
        }
    }

    /// Walk `payload` (the children of the box named `parent`) and parse every
    /// child a registration matches.
    ///
    /// For a caller that holds a box this crate leaves opaque (a `udta`'s
    /// vendor atoms, a top-level `uuid`) and wants its own children typed.
    #[must_use]
    pub fn parse_children<'r, 'a>(
        &'r self,
        parent: [u8; BOX_TYPE_SIZE],
        payload: &'a [u8],
    ) -> ParseChildren<'r, 'a> {
        ParseChildren {
            registry: self,
            parent,
            boxes: BoxIter::new(payload),
        }
    }

    fn insert<T: BoxDef>(&mut self, override_existing: bool) -> Result<&mut Self> {
        let key = registration_key::<T>();
        let fourcc = T::FOURCC;
        if !override_existing {
            if self.entries.contains_key(&(T::PARENT, key)) {
                return Err(Error::BoxTypeRegistered {
                    reason: "already registered; use register_override to replace it",
                });
            }
            if T::UUID.is_none() && crate_types(&fourcc) {
                return Err(Error::BoxTypeRegistered {
                    reason: "this crate types that four-CC; use register_override to replace it",
                });
            }
        }
        self.entries.insert(
            (T::PARENT, key),
            Registration {
                parse: parse_entry::<T>,
            },
        );
        Ok(self)
    }
}

impl fmt::Debug for BoxRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxRegistry")
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// The registry key one type registers under.
fn registration_key<T: BoxDef>() -> BoxKey {
    match T::UUID {
        Some(usertype) => BoxKey::Uuid(usertype),
        None => BoxKey::FourCc(T::FOURCC),
    }
}

/// The parse closure a registration stores: the one place a concrete type is
/// erased to [`RegisteredBox`].
fn parse_entry<T: BoxDef>(bytes: &[u8]) -> Result<Box<dyn RegisteredBox>> {
    Ok(Box::new(<T as Parse>::parse(bytes)?))
}

/// The wire key of the box in `bytes`, or `None` when no registration could
/// name it.
fn box_key(bytes: &[u8]) -> Result<Option<BoxKey>> {
    let (item, _) = parse_box(bytes)?;
    if !item.header.box_type.is(&UUID) {
        return Ok(Some(BoxKey::FourCc(item.four_cc())));
    }
    match item.header.usertype {
        Some(usertype) => Ok(Some(BoxKey::Uuid(usertype))),
        None => Err(Error::UuidBufferTooShort {
            need: crate::box_types::UUID_TYPE_SIZE,
            have: 0,
        }),
    }
}

/// One child box a [`BoxRegistry::parse_children`] walk yielded.
#[derive(Debug)]
pub struct ChildBox<'a> {
    /// Offset of the box's first header byte inside the walked payload.
    pub offset: usize,
    /// The box's four-CC.
    pub fourcc: [u8; BOX_TYPE_SIZE],
    /// The complete box bytes, header included.
    pub bytes: &'a [u8],
    /// The registered parse, when a registration matched this child.
    pub registered: Option<RegisteredEntry>,
}

/// The children of one box, each paired with its registered parse when a
/// registration matches.
#[derive(Debug)]
pub struct ParseChildren<'r, 'a> {
    registry: &'r BoxRegistry,
    parent: [u8; BOX_TYPE_SIZE],
    boxes: BoxIter<'a>,
}

impl<'a> Iterator for ParseChildren<'_, 'a> {
    type Item = Result<ChildBox<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.boxes.next()?;
        Some(item.and_then(|item| {
            let registered = self.registry.parse(Some(self.parent), item.bytes)?;
            Ok(ChildBox {
                offset: item.start,
                fourcc: item.four_cc(),
                bytes: item.bytes,
                registered,
            })
        }))
    }
}

/// Whether this crate already has a typed parse for `fourcc`.
///
/// A registration over one of these is refused unless
/// [`BoxRegistry::register_override`] is used: replacing the crate's own parse
/// silently would change what every reader of the file sees.
fn crate_types(fourcc: &[u8; BOX_TYPE_SIZE]) -> bool {
    matches!(
        fourcc,
        // Top level and movie boxes.
        b"ftyp"
            | b"styp"
            | b"moov"
            | b"mvhd"
            | b"trak"
            | b"tkhd"
            | b"tref"
            | b"edts"
            | b"elst"
            | b"mdia"
            | b"mdhd"
            | b"hdlr"
            | b"minf"
            | b"vmhd"
            | b"smhd"
            | b"nmhd"
            | b"sthd"
            | b"dinf"
            | b"dref"
            | b"url "
            | b"urn "
            | b"stbl"
            | b"stsd"
            | b"stts"
            | b"ctts"
            | b"cslg"
            | b"stsc"
            | b"stsz"
            | b"stz2"
            | b"sdtp"
            | b"stco"
            | b"co64"
            | b"stss"
            | b"sgpd"
            | b"sbgp"
            | b"mvex"
            | b"trex"
            | b"mehd"
            // Movie fragments.
            | b"moof"
            | b"mfhd"
            | b"traf"
            | b"tfhd"
            | b"tfdt"
            | b"trun"
            // Indexes.
            | b"sidx"
            | b"mfra"
            | b"tfra"
            | b"mfro"
            // CENC.
            | b"sinf"
            | b"frma"
            | b"schm"
            | b"schi"
            | b"tenc"
            | b"senc"
            | b"saiz"
            | b"saio"
            | b"pssh"
            // Metadata.
            | b"udta"
            | b"meta"
            | b"ilst"
            | b"keys"
            | b"data"
            | b"mean"
            | b"name"
            | b"chpl"
            | b"loci"
            | b"free"
            | b"skip"
            | b"mdat"
            | b"emsg"
            | b"prft"
            | b"iods"
            // Visual sample entries and their configuration records.
            | b"avc1"
            | b"avc3"
            | b"avcC"
            | b"avcp"
            | b"hvc1"
            | b"hev1"
            | b"hvcC"
            | b"vvc1"
            | b"vvcC"
            | b"av01"
            | b"av1C"
            | b"vp09"
            | b"vpcC"
            | b"mp4v"
            | b"avs3"
            | b"av3c"
            | b"avst"
            | b"av2c"
            | b"la2c"
            | b"lav2"
            | b"lav3"
            | b"lavc"
            | b"lrap"
            | b"telg"
            // Audio sample entries and their configuration records.
            | b"mp4a"
            | b"esds"
            | b"ac-3"
            | b"dac3"
            | b"ec-3"
            | b"dec3"
            | b"ac-4"
            | b"dac4"
            | b"Opus"
            | b"dOps"
            | b"fLaC"
            | b"dfLa"
            | b"dtsc"
            | b"dtsh"
            | b"dtsl"
            | b"dtse"
            | b"ddts"
            | b"mha1"
            | b"mha2"
            | b"mhm1"
            | b"mhm2"
            | b"mhaC"
            | b"av3a"
            | b"dca3"
            // Protected sample entries.
            | b"encv"
            | b"enca"
            // Subtitle and metadata sample entries.
            | b"stpp"
            | b"wvtt"
            | b"tmcd"
            // Visual extensions.
            | b"colr"
            | b"pasp"
            | b"clli"
            | b"mdcv"
            | b"dvcC"
            | b"dvvC"
            | b"dva1"
            | b"btrt"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_types::{BOX_HEADER_MIN_SIZE, write_box_header};

    #[derive(Debug, Clone, PartialEq)]
    struct VendorBox {
        value: u8,
    }

    impl<'a> Parse<'a> for VendorBox {
        type Error = Error;
        fn parse(bytes: &'a [u8]) -> Result<Self> {
            let (item, _) = parse_box(bytes)?;
            item.body
                .first()
                .map(|&value| Self { value })
                .ok_or(Error::InvalidInput("empty vendor box"))
        }
    }

    impl Serialize for VendorBox {
        type Error = Error;
        fn serialized_len(&self) -> usize {
            BOX_HEADER_MIN_SIZE + 1
        }
        fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
            let mut c = write_box_header(buf, b"vend", Serialize::serialized_len(self))?;
            buf[c] = self.value;
            c += 1;
            Ok(c)
        }
    }

    impl BoxDef for VendorBox {
        const FOURCC: [u8; BOX_TYPE_SIZE] = *b"vend";
    }

    #[derive(Debug, Clone, PartialEq)]
    struct ScopedBox {
        value: u8,
    }

    impl<'a> Parse<'a> for ScopedBox {
        type Error = Error;
        fn parse(bytes: &'a [u8]) -> Result<Self> {
            Ok(Self {
                value: parse_box(bytes)?.0.body.first().copied().unwrap_or(0),
            })
        }
    }

    impl Serialize for ScopedBox {
        type Error = Error;
        fn serialized_len(&self) -> usize {
            BOX_HEADER_MIN_SIZE + 1
        }
        fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
            let mut c = write_box_header(buf, b"vend", Serialize::serialized_len(self))?;
            buf[c] = self.value;
            c += 1;
            Ok(c)
        }
    }

    impl BoxDef for ScopedBox {
        const FOURCC: [u8; BOX_TYPE_SIZE] = *b"vend";
        const PARENT: Option<[u8; BOX_TYPE_SIZE]> = Some(*b"udta");
    }

    fn vend(value: u8) -> Vec<u8> {
        VendorBox { value }.try_to_bytes().expect("bytes")
    }

    #[test]
    fn a_registered_box_parses_typed_and_round_trips() {
        let mut registry = BoxRegistry::new();
        registry.register::<VendorBox>().expect("register");
        let bytes = vend(7);
        let entry = registry
            .parse(None, &bytes)
            .expect("parse")
            .expect("registered");
        assert_eq!(entry.fourcc(), *b"vend");
        assert_eq!(
            entry.downcast_ref::<VendorBox>(),
            Some(&VendorBox { value: 7 })
        );
        assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);
    }

    #[test]
    fn an_unregistered_box_is_not_parsed() {
        let registry = BoxRegistry::new();
        assert!(registry.is_empty());
        assert!(registry.parse(None, &vend(1)).expect("parse").is_none());
    }

    #[test]
    fn a_second_registration_of_one_key_needs_override() {
        let mut registry = BoxRegistry::new();
        registry.register::<VendorBox>().expect("register");
        assert!(matches!(
            registry.register::<VendorBox>(),
            Err(Error::BoxTypeRegistered { .. })
        ));
        registry.register_override::<VendorBox>().expect("override");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_box_type_the_crate_types_needs_override() {
        #[derive(Debug, Clone)]
        struct FakeMoov;

        impl<'a> Parse<'a> for FakeMoov {
            type Error = Error;
            fn parse(_bytes: &'a [u8]) -> Result<Self> {
                Ok(Self)
            }
        }

        impl Serialize for FakeMoov {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                BOX_HEADER_MIN_SIZE
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                write_box_header(buf, b"moov", Serialize::serialized_len(self))
            }
        }

        impl BoxDef for FakeMoov {
            const FOURCC: [u8; BOX_TYPE_SIZE] = *b"moov";
        }

        let mut registry = BoxRegistry::new();
        assert!(matches!(
            registry.register::<FakeMoov>(),
            Err(Error::BoxTypeRegistered { .. })
        ));
        registry.register_override::<FakeMoov>().expect("override");
    }

    #[test]
    fn a_scoped_registration_wins_for_its_parent_only() {
        let mut registry = BoxRegistry::new();
        registry.register::<VendorBox>().expect("register");
        registry.register::<ScopedBox>().expect("register scoped");
        let bytes = vend(9);

        let scoped = registry
            .parse(Some(*b"udta"), &bytes)
            .expect("parse")
            .expect("registered");
        assert_eq!(
            scoped.downcast_ref::<ScopedBox>(),
            Some(&ScopedBox { value: 9 })
        );

        let other = registry
            .parse(Some(*b"trak"), &bytes)
            .expect("parse")
            .expect("registered");
        assert_eq!(
            other.downcast_ref::<VendorBox>(),
            Some(&VendorBox { value: 9 })
        );
    }

    #[test]
    fn parse_children_walks_a_udta_payload() {
        let mut registry = BoxRegistry::new();
        registry.register::<ScopedBox>().expect("register");
        let mut payload = vend(4);
        payload.extend_from_slice(&vend(5));
        let children: Vec<ChildBox<'_>> = registry
            .parse_children(*b"udta", &payload)
            .collect::<Result<Vec<_>>>()
            .expect("walk");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].offset, 0);
        assert_eq!(children[1].offset, 9);
        assert_eq!(
            children[1]
                .registered
                .as_ref()
                .and_then(|entry| entry.downcast_ref::<ScopedBox>()),
            Some(&ScopedBox { value: 5 })
        );
    }

    #[test]
    fn a_uuid_box_is_keyed_by_its_extended_type() {
        #[derive(Debug, Clone, PartialEq)]
        struct UuidBox;

        impl<'a> Parse<'a> for UuidBox {
            type Error = Error;
            fn parse(_bytes: &'a [u8]) -> Result<Self> {
                Ok(Self)
            }
        }

        impl Serialize for UuidBox {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                BOX_HEADER_MIN_SIZE + crate::box_types::UUID_TYPE_SIZE
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let need = <Self as Serialize>::serialized_len(self);
                let mut c = write_box_header(buf, b"uuid", need)?;
                buf[c..c + crate::box_types::UUID_TYPE_SIZE].copy_from_slice(&[3u8; 16]);
                c += crate::box_types::UUID_TYPE_SIZE;
                Ok(c)
            }
        }

        impl BoxDef for UuidBox {
            const FOURCC: [u8; BOX_TYPE_SIZE] = *b"uuid";
            const UUID: Option<[u8; UUID_TYPE_SIZE]> = Some([3u8; UUID_TYPE_SIZE]);
        }

        let mut registry = BoxRegistry::new();
        registry.register::<UuidBox>().expect("register");
        let bytes = UuidBox.try_to_bytes().expect("bytes");
        assert!(registry.parse(None, &bytes).expect("parse").is_some());
    }
}

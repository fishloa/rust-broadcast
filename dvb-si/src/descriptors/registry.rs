//! Runtime descriptor registry — open registration of client private tags.
//!
//! [`DescriptorRegistry`] is a runtime-configurable walk engine that mirrors
//! the semantics of the free [`crate::descriptors::parse_loop`] but allows
//! clients to register their own private descriptor types.  Registered custom
//! parsers win over built-in dispatch; the 0x83 logical_channel built-in is
//! opt-in via [`DescriptorRegistry::with_logical_channel`].
//!
//! # PDS-scoped registration
//!
//! DVB private descriptor tags (`0x80..=0xFE`) are ambiguous without a
//! preceding [`private_data_specifier_descriptor`][crate::descriptors::private_data_specifier]
//! (tag `0x5F`) that scopes them.  Use [`register_for_pds`][DescriptorRegistry::register_for_pds]
//! to register a type that is only dispatched when the active
//! `private_data_specifier` matches.  A PDS-scoped registration takes precedence
//! over a PDS-agnostic [`register`][DescriptorRegistry::register] of the same tag when that
//! PDS is active.
//!
//! # Owned types only
//!
//! Registered types must be `'static` (i.e. owned — no borrowed slices).
//! This is required because the parsed value is heap-allocated as a
//! `Box<dyn DescriptorObject>` whose concrete type is erased; `dyn Any`
//! downcast demands `'static`.  If your wire layout contains borrowed bytes,
//! copy them into a `Vec<u8>` in the struct.
//!
//! # Example
//!
//! ```rust,no_run
//! use dvb_si::descriptors::{DescriptorRegistry, AnyDescriptor};
//! use dvb_si::traits::DescriptorDef;
//! use broadcast_common::Parse;
//!
//! // A registered type must be `serde::Serialize` only when the `serde`
//! // feature is on (that is what `DescriptorObject` requires there).
//! #[derive(Debug)]
//! #[cfg_attr(feature = "serde", derive(serde::Serialize))]
//! struct MyPrivate { x: u8 }
//!
//! impl<'a> Parse<'a> for MyPrivate {
//!     type Error = dvb_si::Error;
//!     fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
//!         if bytes.len() < 3 {
//!             return Err(dvb_si::Error::BufferTooShort {
//!                 need: 3, have: bytes.len(), what: "MyPrivate",
//!             });
//!         }
//!         Ok(Self { x: bytes[2] })
//!     }
//! }
//!
//! impl<'a> DescriptorDef<'a> for MyPrivate {
//!     const TAG: u8 = 0xA7;
//!     const NAME: &'static str = "MY_PRIVATE";
//! }
//!
//! let mut reg = DescriptorRegistry::new();
//! reg.register::<MyPrivate>().with_logical_channel();
//!
//! let bytes = [0xA7, 0x01, 0x42u8];
//! let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
//! if let AnyDescriptor::Other { tag, ref value } = items[0] {
//!     assert_eq!(tag, 0xA7);
//!     assert_eq!(value.downcast_ref::<MyPrivate>().unwrap().x, 0x42);
//! }
//! ```

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::any::Any;

use crate::descriptors::any::AnyDescriptor;
use crate::descriptors::private_data_specifier::PDS_NORDIG;

// ---------------------------------------------------------------------------
// DescriptorObject trait
// ---------------------------------------------------------------------------

/// Object-safe face of a runtime-registered descriptor value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(not(feature = "serde"))]
pub trait DescriptorObject: core::fmt::Debug + Any + Send + Sync {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// Object-safe face of a runtime-registered descriptor value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(feature = "serde")]
pub trait DescriptorObject: core::fmt::Debug + Any + Send + Sync + erased_serde::Serialize {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

// Blanket impl — no-serde arm.
#[cfg(not(feature = "serde"))]
impl<T> DescriptorObject for T
where
    T: core::fmt::Debug + Any + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Blanket impl — serde arm.
#[cfg(feature = "serde")]
impl<T> DescriptorObject for T
where
    T: core::fmt::Debug + Any + Send + Sync + serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Downcast helpers ON THE TRAIT OBJECT (not the blanket).
//
// The blanket `impl<T> DescriptorObject for T` also covers `Box<dyn
// DescriptorObject>` itself whenever the box satisfies the bounds — it does
// under `--no-default-features`, where the bound is just `Debug + Any + Send +
// Sync`. So `the_box.as_any()` resolves to the *box's* impl and reports the
// box's `TypeId`, not the inner value's — a silent downcast failure. (Under
// `serde` the extra `serde::Serialize` bound excludes the box, which is why the
// footgun only bites without default features.) Calling through `dyn
// DescriptorObject` (which `Box` derefs to) always hits the inner value, so
// always downcast via these methods rather than `the_box.as_any()`.
impl dyn DescriptorObject {
    /// Downcast a registered descriptor to its concrete type `T`.
    ///
    /// Works for `Box<dyn DescriptorObject>` (it derefs to the trait object)
    /// under every feature configuration.
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// `true` if the registered descriptor's concrete type is `T`.
    #[must_use]
    pub fn is<T: Any>(&self) -> bool {
        self.as_any().is::<T>()
    }
}

// ---------------------------------------------------------------------------
// Erased serialisation helper (serde-gated)
// ---------------------------------------------------------------------------

/// `serialize_with` helper used on [`AnyDescriptor::Other`]'s `value` field.
///
/// Delegates to [`erased_serde::serialize`] so the concrete type's
/// `serde::Serialize` impl is invoked through the trait object.
///
/// The `&Box<T>` is required by serde's `serialize_with` codegen — the field
/// type is `Box<dyn DescriptorObject>` so serde passes `&Box<dyn DescriptorObject>`.
#[cfg(feature = "serde")]
#[allow(clippy::borrowed_box)]
pub(crate) fn serialize_erased<S: serde::Serializer>(
    v: &Box<dyn DescriptorObject>,
    s: S,
) -> Result<S::Ok, S::Error> {
    erased_serde::serialize(&**v, s)
}

// ---------------------------------------------------------------------------
// Internal parse closure type
// ---------------------------------------------------------------------------

/// A heap-allocated parse closure that takes a full descriptor (header + body)
/// and returns an owned, type-erased descriptor value.
pub(crate) type CustomParse =
    Box<dyn for<'a> Fn(&'a [u8]) -> crate::Result<Box<dyn DescriptorObject>> + Send + Sync>;

// ---------------------------------------------------------------------------
// DescriptorRegistry
// ---------------------------------------------------------------------------

/// Runtime-configurable descriptor registry.
///
/// By default the registry has no custom parsers and 0x83 logical_channel is
/// disabled (it is a private tag that requires `private_data_specifier`
/// context).  Use [`register`][Self::register] and
/// [`with_logical_channel`][Self::with_logical_channel] to opt in.
///
/// Walk a byte slice with [`parse_loop`][Self::parse_loop]; it returns a lazy
/// [`RegistryIter`] with identical truncation/fuse/error-continue semantics to
/// the free [`crate::descriptors::parse_loop`].
///
/// # Precedence (per entry)
///
/// 1. PDS-scoped custom parser (if the current `private_data_specifier` matches
///    a [`register_for_pds`][Self::register_for_pds] entry) →
///    [`AnyDescriptor::Other`]
/// 2. PDS-agnostic custom-registered parser (tag in the [`custom`][Self::register]
///    map) → [`AnyDescriptor::Other`]
/// 3. PDS-scoped logical-channel opt-in (tag 0x83 + active PDS in
///    [`with_logical_channel_for_pds`][Self::with_logical_channel_for_pds])
///    → [`AnyDescriptor::LogicalChannel`]
/// 4. Logical-channel opt-in (tag 0x83 + [`with_logical_channel`][Self::with_logical_channel]
///    enabled) → [`AnyDescriptor::LogicalChannel`]
/// 5. Built-in dispatch (internal `AnyDescriptor::dispatch`) → typed variant
/// 6. Unknown → [`AnyDescriptor::Unknown`]
#[derive(Default)]
pub struct DescriptorRegistry {
    custom: BTreeMap<(Option<u32>, u8), CustomParse>,
    logical_channel: bool,
    logical_channel_pds: alloc::collections::BTreeSet<u32>,
}

impl DescriptorRegistry {
    /// Create an empty registry (built-in dispatch only; 0x83 disabled).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an owned custom descriptor type for its
    /// [`DescriptorDef::TAG`][crate::traits::DescriptorDef::TAG].
    ///
    /// # Owned types only
    ///
    /// `T` must be `'static` — no borrowed slices.  The registered value is
    /// type-erased as `Box<dyn DescriptorObject>`; `dyn Any` downcast requires
    /// the concrete type to be `'static`.
    ///
    /// Registering a type whose `TAG` is already used by a built-in **overrides**
    /// the built-in for that tag.
    ///
    /// Re-registering the same tag replaces the prior custom parser (last wins).
    /// A failing custom parse surfaces the client's `Parse::Error` unwrapped —
    /// embed identifying context (type/tag) in your error's `what`/`reason` fields.
    ///
    /// The registration is PDS-agnostic: it matches the tag regardless of which
    /// `private_data_specifier` (if any) is active in the loop.  A
    /// PDS-scoped registration via [`register_for_pds`][Self::register_for_pds]
    /// takes precedence over this when the matching PDS is active.
    pub fn register<T>(&mut self) -> &mut Self
    where
        T: for<'a> crate::traits::DescriptorDef<'a> + DescriptorObject + 'static,
    {
        let tag = <T as crate::traits::DescriptorDef<'static>>::TAG;
        self.custom.insert(
            (None, tag),
            Box::new(|b| {
                Ok(Box::new(<T as broadcast_common::Parse>::parse(b)?)
                    as Box<dyn DescriptorObject>)
            }),
        );
        self
    }

    /// Register an owned custom descriptor type scoped to a specific
    /// `private_data_specifier` value.
    ///
    /// The type is only dispatched when a preceding
    /// [`private_data_specifier_descriptor`][crate::descriptors::private_data_specifier]
    /// (tag `0x5F`) in the same descriptor loop has set the active PDS to
    /// `pds`.  A PDS-scoped registration takes precedence over a PDS-agnostic
    /// [`register`][Self::register] of the same tag when that PDS is active.
    ///
    /// Re-registering the same `(pds, tag)` pair replaces the prior custom
    /// parser (last wins).
    pub fn register_for_pds<T>(&mut self, pds: u32) -> &mut Self
    where
        T: for<'a> crate::traits::DescriptorDef<'a> + DescriptorObject + 'static,
    {
        let tag = <T as crate::traits::DescriptorDef<'static>>::TAG;
        self.custom.insert(
            (Some(pds), tag),
            Box::new(|b| {
                Ok(Box::new(<T as broadcast_common::Parse>::parse(b)?)
                    as Box<dyn DescriptorObject>)
            }),
        );
        self
    }

    /// Enable the 0x83 logical_channel built-in.
    ///
    /// By default 0x83 is not auto-dispatched because it is a private tag
    /// whose semantics depend on a `private_data_specifier` context.  Call
    /// this when you know the loop is from an EACEM/NorDig/D-Book stream.
    pub fn with_logical_channel(&mut self) -> &mut Self {
        self.logical_channel = true;
        self
    }

    /// Enable the 0x83 logical_channel built-in ONLY when the active
    /// `private_data_specifier` (set by a preceding 0x5F descriptor in the
    /// same loop) matches `pds`.
    ///
    /// This is a safer opt-in than [`with_logical_channel`][Self::with_logical_channel]:
    /// tag 0x83 under a different (or absent) PDS stays `Unknown`.
    ///
    /// Common values are [`crate::descriptors::PDS_EACEM`] and
    /// [`crate::descriptors::PDS_NORDIG`].  A per-PDS opt-in takes precedence
    /// over the global [`with_logical_channel`][Self::with_logical_channel]
    /// opt-in when the matching PDS is active.
    pub fn with_logical_channel_for_pds(&mut self, pds: u32) -> &mut Self {
        self.logical_channel_pds.insert(pds);
        self
    }

    /// Register both NorDig logical channel descriptors (v1 tag 0x83 and
    /// v2 tag 0x87) for PDS_NORDIG (0x00000029). See
    /// [`register_for_pds`][Self::register_for_pds].
    pub fn with_nordig_lcn(&mut self) -> &mut Self {
        self.register_for_pds::<crate::descriptors::nordig::NordigLogicalChannelV1>(PDS_NORDIG);
        self.register_for_pds::<crate::descriptors::nordig::NordigLogicalChannelV2>(PDS_NORDIG);
        self
    }

    /// Register the AVS3_audio_descriptor (tag 0xD2, T/AI 109.7 clause 8.1.3.4)
    /// for `private_data_specifier` [`crate::descriptors::PDS_AVSA`] ('AVSA',
    /// `0x41565341`) - the specifier EN 300 468 V1.20.1 Annex O / TS 101 154
    /// V2.10.1 section 4.1.8.35 require to precede it. Under any other (or
    /// no) specifier tag 0xD2 stays `Unknown`.
    pub fn with_avs3_audio(&mut self) -> &mut Self {
        self.register_for_pds::<crate::descriptors::avs3_audio::Avs3AudioDescriptor>(
            crate::descriptors::PDS_AVSA,
        );
        self
    }

    /// Register the AVS3_video_descriptor (tag 0xD1, T/AI 109.6 clause 9.3.2 /
    /// GY/T 420 clause 7.3.3.2) for [`crate::descriptors::PDS_AVSV`] ('AVSV',
    /// `0x41565356`). The scope is established either by a preceding
    /// `private_data_specifier_descriptor` (ETSI TS 101 154 V2.10.1 section
    /// 4.1.8) or by a `registration_descriptor` whose `format_identifier` is
    /// 'AVSV' (the Chinese documents' form). Otherwise tag 0xD1 stays `Unknown`.
    pub fn with_avs3_video(&mut self) -> &mut Self {
        self.register_for_pds::<crate::descriptors::avs3_video::Avs3VideoDescriptor>(
            crate::descriptors::PDS_AVSV,
        );
        self
    }

    /// Register the AVS2_video_descriptor (tag 0x40, GY/T 420-2025 clause 7.2.3)
    /// for 'AVSV' (private_data_specifier or `registration_descriptor`
    /// `format_identifier`, GY/T 420 clause 7.2.2). Tag 0x40 stays the DVB
    /// `network_name_descriptor` outside that scope.
    pub fn with_avs2_video(&mut self) -> &mut Self {
        self.register_for_pds::<crate::descriptors::avs2_video::Avs2VideoDescriptor>(
            crate::descriptors::PDS_AVSV,
        );
        self
    }

    /// Register the AVS2_audio_descriptor (tag 0xD3, T/AI 109.7-2024 clause
    /// 8.2.3.4) for 'AVSA' (private_data_specifier or `registration_descriptor`
    /// `format_identifier`, clause 8.2.3.3).
    pub fn with_avs2_audio(&mut self) -> &mut Self {
        self.register_for_pds::<crate::descriptors::avs2_audio::Avs2AudioDescriptor>(
            crate::descriptors::PDS_AVSA,
        );
        self
    }

    /// Register the older T/UWA 012.2-2023 AVS3 video descriptor (tag 62,
    /// clause 5.4.1). The standard gives it no private_data_specifier, so this
    /// is a PDS-agnostic registration of tag 0x3E.
    pub fn with_tuwa_avs3_video(&mut self) -> &mut Self {
        self.register::<crate::descriptors::tuwa_avs3_video::TuwaAvs3VideoDescriptor>()
    }

    /// Register the China `caption_service_descriptor` (tag 0x86, GY/T 270-2013
    /// clause 6.4 Table 8). The standard gives it no private_data_specifier, so
    /// this is a PDS-agnostic registration of tag 0x86: enable it only for
    /// Chinese DTV streams (tag 0x86 is the ATSC caption_service_descriptor
    /// elsewhere, with a different layout).
    pub fn with_china_caption_service(&mut self) -> &mut Self {
        self.register::<crate::descriptors::china_caption_service::ChinaCaptionServiceDescriptor>()
    }

    /// Lazily walk a raw descriptor loop using this registry's configuration.
    ///
    /// Semantics mirror [`crate::descriptors::parse_loop`]: per-descriptor
    /// parse errors yield `Err` and iteration continues; a truncated final
    /// header or body yields one `Err` then fuses.
    ///
    /// A `private_data_specifier_descriptor` (tag `0x5F`) in the loop
    /// automatically updates the iterator's PDS context, scoping subsequent
    /// private-tag dispatch.
    #[must_use]
    pub fn parse_loop<'r, 'a>(&'r self, bytes: &'a [u8]) -> RegistryIter<'r, 'a> {
        RegistryIter {
            registry: self,
            bytes,
            pos: 0,
            fused: false,
            current_pds: None,
        }
    }

    /// Like [`parse_loop`](Self::parse_loop) but starting inside the scope an
    /// enclosing loop already opened (`inherited`, from
    /// [`RegistryIter::avs_scope`]).
    ///
    /// T/AI 109.7 clause 8.1.3 and GY/T 420 clauses 7.2.2 / 7.3.3.1 / 7.4.3.1
    /// put the `registration_descriptor` under "Program and program element
    /// descriptors" and say only that "the PMT shall contain" it, without
    /// naming the `program_info` or the `ES_info` loop, while the AVS
    /// descriptors themselves sit in the ES_info loop (T/AI 109.7 clause
    /// 8.1.3.4, GY/T 420 clause 7.3.1 d). A registration in the `program_info`
    /// loop therefore has to scope the AVS descriptors of the elementary-stream
    /// loops that follow: walk `program_info` with `parse_loop`, take the scope
    /// with `avs_scope`, and walk each ES_info loop with this method.
    #[must_use]
    pub fn parse_loop_in_scope<'r, 'a>(
        &'r self,
        bytes: &'a [u8],
        inherited: Option<u32>,
    ) -> RegistryIter<'r, 'a> {
        RegistryIter {
            registry: self,
            bytes,
            pos: 0,
            fused: false,
            current_pds: inherited,
        }
    }
}

// ---------------------------------------------------------------------------
// RegistryIter
// ---------------------------------------------------------------------------

/// Lazy iterator over a raw descriptor loop, driven by a [`DescriptorRegistry`].
///
/// Returned by [`DescriptorRegistry::parse_loop`].
pub struct RegistryIter<'r, 'a> {
    registry: &'r DescriptorRegistry,
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
    current_pds: Option<u32>,
}

/// Shared precedence ladder for both [`RegistryIter`] and [`ExtRegistryIter`].
///
/// Returns the [`AnyDescriptor`] that results from applying the six-step
/// precedence: PDS-scoped custom → PDS-agnostic custom → PDS-scoped
/// logical-channel → logical-channel opt-in → built-in dispatch → Unknown.
pub(crate) fn dispatch_entry<'a>(
    registry: &DescriptorRegistry,
    current_pds: Option<u32>,
    tag: u8,
    full: &'a [u8],
) -> crate::Result<AnyDescriptor<'a>> {
    if let Some(pds) = current_pds
        && let Some(parse_fn) = registry.custom.get(&(Some(pds), tag))
    {
        return parse_fn(full).map(|value| AnyDescriptor::Other { tag, value });
    }
    if let Some(parse_fn) = registry.custom.get(&(None, tag)) {
        return parse_fn(full).map(|value| AnyDescriptor::Other { tag, value });
    }
    if let Some(pds) = current_pds
        && tag == crate::descriptors::logical_channel::TAG
        && registry.logical_channel_pds.contains(&pds)
    {
        use broadcast_common::Parse;
        return crate::descriptors::logical_channel::LogicalChannelDescriptor::parse(full)
            .map(AnyDescriptor::LogicalChannel);
    }
    if registry.logical_channel && tag == crate::descriptors::logical_channel::TAG {
        use broadcast_common::Parse;
        return crate::descriptors::logical_channel::LogicalChannelDescriptor::parse(full)
            .map(AnyDescriptor::LogicalChannel);
    }
    if let Some(res) = AnyDescriptor::dispatch(tag, full) {
        return res;
    }
    Ok(AnyDescriptor::Unknown {
        tag,
        body: &full[2..],
    })
}

fn update_pds(current: &mut Option<u32>, tag: u8, full: &[u8]) {
    // The Chinese AVS3 documents (T/AI 109.6/109.7, GY/T 420) scope their
    // descriptors with a registration_descriptor whose format_identifier is the
    // same four ASCII bytes as the DVB private_data_specifier ('AVSV'/'AVSA'),
    // so such a registration opens the same scope. Any other registration
    // leaves the context untouched.
    if tag == crate::descriptors::registration::TAG {
        use broadcast_common::Parse;
        if let Ok(reg) = crate::descriptors::registration::RegistrationDescriptor::parse(full) {
            let id = u32::from_be_bytes(reg.format_identifier);
            if id == crate::descriptors::PDS_AVSA || id == crate::descriptors::PDS_AVSV {
                *current = Some(id);
            }
        }
    }
    if tag == crate::descriptors::private_data_specifier::TAG {
        use broadcast_common::Parse;
        if let Ok(pds) =
            crate::descriptors::private_data_specifier::PrivateDataSpecifierDescriptor::parse(full)
        {
            *current = Some(pds.private_data_specifier);
        }
    }
}

impl RegistryIter<'_, '_> {
    /// The AVS scope ('AVSV' / 'AVSA') the entries walked so far opened, via a
    /// `private_data_specifier_descriptor` or a `registration_descriptor`
    /// `format_identifier`; `None` when no AVS scope is open. Feed it to
    /// [`DescriptorRegistry::parse_loop_in_scope`] for the elementary-stream
    /// loops that follow a `program_info` loop.
    #[must_use]
    pub fn avs_scope(&self) -> Option<u32> {
        self.current_pds
            .filter(|p| *p == crate::descriptors::PDS_AVSA || *p == crate::descriptors::PDS_AVSV)
    }
}

impl<'a> Iterator for RegistryIter<'_, 'a> {
    type Item = crate::Result<AnyDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (tag, full) = match crate::descriptors::any::next_loop_entry(
            self.bytes,
            &mut self.pos,
            &mut self.fused,
        )? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        update_pds(&mut self.current_pds, tag, full);

        Some(dispatch_entry(self.registry, self.current_pds, tag, full))
    }
}

impl core::iter::FusedIterator for RegistryIter<'_, '_> {}

// ---------------------------------------------------------------------------
// ExtIterItem — item type for iter_with_extensions
// ---------------------------------------------------------------------------

/// Item produced by [`DescriptorLoop::iter_with_extensions`](super::any::DescriptorLoop::iter_with_extensions).
///
/// Extends [`AnyDescriptor`] with a third arm for custom-registered extension
/// bodies whose `descriptor_tag_extension` is recognised by an
/// [`ExtensionRegistry`](super::extension::registry::ExtensionRegistry).
/// The `value` field can be downcast to the concrete type via
/// `downcast_ref` on the value (see [`ExtensionObject`](super::extension::registry::ExtensionObject))
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum ExtIterItem<'a> {
    /// A regular descriptor (including built-in extension descriptors).
    Descriptor(AnyDescriptor<'a>),
    /// A custom-registered extension body (tag `0x7F`, known `descriptor_tag_extension`).
    CustomExtension {
        /// The `descriptor_tag_extension` byte.
        tag_extension: u8,
        /// The parsed, type-erased extension body value. Call `downcast_ref`
        /// on it (see [`ExtensionObject`](super::extension::registry::ExtensionObject)) to recover the concrete type.
        #[cfg_attr(
            feature = "serde",
            serde(serialize_with = "super::extension::registry::serialize_erased")
        )]
        value: Box<dyn super::extension::registry::ExtensionObject>,
    },
}

// ---------------------------------------------------------------------------
// ExtRegistryIter — iterator for iter_with_extensions
// ---------------------------------------------------------------------------

/// Lazy iterator over a raw descriptor loop, driven by both a
/// [`DescriptorRegistry`] and an
/// [`ExtensionRegistry`](super::extension::registry::ExtensionRegistry).
///
/// Returned by [`DescriptorLoop::iter_with_extensions`](super::any::DescriptorLoop::iter_with_extensions).
pub struct ExtRegistryIter<'r, 'a> {
    desc_reg: &'r DescriptorRegistry,
    ext_reg: &'r super::extension::registry::ExtensionRegistry,
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
    current_pds: Option<u32>,
}

impl<'r, 'a> ExtRegistryIter<'r, 'a> {
    pub(crate) fn new(
        desc_reg: &'r DescriptorRegistry,
        ext_reg: &'r super::extension::registry::ExtensionRegistry,
        bytes: &'a [u8],
    ) -> Self {
        Self {
            desc_reg,
            ext_reg,
            bytes,
            pos: 0,
            fused: false,
            current_pds: None,
        }
    }
}

impl<'a> Iterator for ExtRegistryIter<'_, 'a> {
    type Item = crate::Result<ExtIterItem<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (tag, full) = match crate::descriptors::any::next_loop_entry(
            self.bytes,
            &mut self.pos,
            &mut self.fused,
        )? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        update_pds(&mut self.current_pds, tag, full);

        let len = full.len() - 2;
        if tag == crate::descriptors::extension::TAG && len >= 1 {
            let tag_extension = full[2];
            if self.ext_reg.has_custom(tag_extension) {
                return Some(match self.ext_reg.parse_body(tag_extension, &full[3..]) {
                    Ok(super::extension::registry::RegisteredExtension::Custom {
                        tag_extension,
                        value,
                    }) => Ok(ExtIterItem::CustomExtension {
                        tag_extension,
                        value,
                    }),
                    Ok(super::extension::registry::RegisteredExtension::Builtin(d)) => {
                        Ok(ExtIterItem::Descriptor(AnyDescriptor::Extension(d)))
                    }
                    Err(e) => Err(e),
                });
            }
        }

        Some(
            dispatch_entry(self.desc_reg, self.current_pds, tag, full).map(ExtIterItem::Descriptor),
        )
    }
}

impl core::iter::FusedIterator for ExtRegistryIter<'_, '_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::private_data_specifier;
    use crate::descriptors::private_data_specifier::{PDS_EACEM, PDS_NORDIG};
    use crate::traits::DescriptorDef;

    #[test]
    fn pds_constants_match_the_register() {
        // Pin the public consts to the vendored TS 101 162 register
        // (registries/tsPDS.names) so they can't silently drift.
        assert_eq!(
            private_data_specifier::private_data_specifier_name(PDS_EACEM),
            Some("EACEM/EICTA")
        );
        assert_eq!(
            private_data_specifier::private_data_specifier_name(PDS_NORDIG),
            Some("NorDig")
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct PdsEacem {
        v: u8,
    }

    impl<'a> broadcast_common::Parse<'a> for PdsEacem {
        type Error = crate::error::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.len() < 3 {
                return Err(crate::error::Error::BufferTooShort {
                    need: 3,
                    have: bytes.len(),
                    what: "PdsEacem",
                });
            }
            Ok(Self { v: bytes[2] })
        }
    }

    impl DescriptorDef<'_> for PdsEacem {
        const TAG: u8 = 0x83;
        const NAME: &'static str = "PDS_EACEM";
    }

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct PdsNordig {
        w: u8,
    }

    impl<'a> broadcast_common::Parse<'a> for PdsNordig {
        type Error = crate::error::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.len() < 3 {
                return Err(crate::error::Error::BufferTooShort {
                    need: 3,
                    have: bytes.len(),
                    what: "PdsNordig",
                });
            }
            Ok(Self { w: bytes[2] })
        }
    }

    impl DescriptorDef<'_> for PdsNordig {
        const TAG: u8 = 0x83;
        const NAME: &'static str = "PDS_NORDIG";
    }

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct PdsAgnostic {
        z: u8,
    }

    impl<'a> broadcast_common::Parse<'a> for PdsAgnostic {
        type Error = crate::error::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.len() < 3 {
                return Err(crate::error::Error::BufferTooShort {
                    need: 3,
                    have: bytes.len(),
                    what: "PdsAgnostic",
                });
            }
            Ok(Self { z: bytes[2] })
        }
    }

    impl DescriptorDef<'_> for PdsAgnostic {
        const TAG: u8 = 0x84;
        const NAME: &'static str = "PDS_AGNOSTIC";
    }

    fn pds_descriptor(pds: u32) -> Vec<u8> {
        let mut v = vec![private_data_specifier::TAG, 4];
        v.extend_from_slice(&pds.to_be_bytes());
        v
    }

    #[test]
    fn pds_scoped_same_tag_resolves_by_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.register_for_pds::<PdsEacem>(PDS_EACEM);
        reg.register_for_pds::<PdsNordig>(PDS_NORDIG);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_EACEM));
        bytes.extend_from_slice(&[0x83, 0x01, 0xAA]);

        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnyDescriptor::PrivateDataSpecifier(_)));
        match &items[1] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x83);
                let c = value.downcast_ref::<PdsEacem>().unwrap();
                assert_eq!(c.v, 0xAA);
            }
            other => panic!("expected Other (PdsEacem), got {other:?}"),
        }

        let mut bytes2 = Vec::new();
        bytes2.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        bytes2.extend_from_slice(&[0x83, 0x01, 0xBB]);

        let items2: Vec<_> = reg.parse_loop(&bytes2).collect::<Result<_, _>>().unwrap();
        match &items2[1] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x83);
                let c = value.downcast_ref::<PdsNordig>().unwrap();
                assert_eq!(c.w, 0xBB);
            }
            other => panic!("expected Other (PdsNordig), got {other:?}"),
        }
    }

    #[test]
    fn pds_scoped_does_not_match_wrong_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.register_for_pds::<PdsEacem>(PDS_EACEM);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        bytes.extend_from_slice(&[0x83, 0x01, 0xCC]);

        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnyDescriptor::PrivateDataSpecifier(_)));
        match &items[1] {
            AnyDescriptor::Unknown { tag, .. } => assert_eq!(*tag, 0x83),
            other => panic!("expected Unknown (wrong PDS), got {other:?}"),
        }
    }

    /// AVS2 descriptors (GY/T 420 / T/AI 109.7) and the T/UWA 012.2 tag-62
    /// descriptor are opt-in and scoped like the AVS3 ones.
    #[test]
    fn avs2_and_tuwa_descriptors_are_scoped() {
        use crate::descriptors::avs2_audio::Avs2AudioDescriptor;
        use crate::descriptors::avs2_video::Avs2VideoDescriptor;
        use crate::descriptors::private_data_specifier::{PDS_AVSA, PDS_AVSV};
        use crate::descriptors::tuwa_avs3_video::TuwaAvs3VideoDescriptor;
        let mut reg = DescriptorRegistry::new();
        reg.with_avs2_video()
            .with_avs2_audio()
            .with_tuwa_avs3_video();

        let v = [0x40, 5, 0x20, 0x42, 0x00, 0xA9, 0x5F];
        // Unscoped tag 0x40 is still the DVB network_name_descriptor.
        let items: Vec<_> = reg.parse_loop(&v).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::NetworkName(_)));
        // Scoped by an 'AVSV' registration_descriptor it is the AVS2 descriptor.
        let mut b = vec![0x05, 4, b'A', b'V', b'S', b'V'];
        b.extend_from_slice(&v);
        let items: Vec<_> = reg.parse_loop(&b).collect::<Result<_, _>>().unwrap();
        match &items[1] {
            AnyDescriptor::Other { tag: 0x40, value } => {
                assert!(value.downcast_ref::<Avs2VideoDescriptor>().is_some());
            }
            other => panic!("expected Avs2Video, got {other:?}"),
        }
        // ... or by a private_data_specifier.
        let mut b = pds_descriptor(PDS_AVSV);
        b.extend_from_slice(&v);
        let items: Vec<_> = reg.parse_loop(&b).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[1], AnyDescriptor::Other { tag: 0x40, .. }));

        let a = [0xD3, 2, 6, 0x20];
        let mut b = vec![0x05, 4, b'A', b'V', b'S', b'A'];
        b.extend_from_slice(&a);
        let items: Vec<_> = reg.parse_loop(&b).collect::<Result<_, _>>().unwrap();
        match &items[1] {
            AnyDescriptor::Other { tag: 0xD3, value } => {
                assert!(value.downcast_ref::<Avs2AudioDescriptor>().is_some());
            }
            other => panic!("expected Avs2Audio, got {other:?}"),
        }
        let mut b = pds_descriptor(PDS_AVSA);
        b.extend_from_slice(&a);
        assert!(matches!(
            reg.parse_loop(&b).collect::<Result<Vec<_>, _>>().unwrap()[1],
            AnyDescriptor::Other { tag: 0xD3, .. }
        ));
        // Without scope tag 0xD3 stays Unknown.
        let items: Vec<_> = reg.parse_loop(&a).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::Unknown { tag: 0xD3, .. }));

        // T/UWA 012.2 tag 62: PDS-agnostic opt-in.
        let t = [62, 7, 0x22, 0x42, 0xAA, 0x4F, 9, 14, 9];
        let items: Vec<_> = reg.parse_loop(&t).collect::<Result<_, _>>().unwrap();
        match &items[0] {
            AnyDescriptor::Other { tag: 62, value } => {
                assert!(value.downcast_ref::<TuwaAvs3VideoDescriptor>().is_some());
            }
            other => panic!("expected TuwaAvs3Video, got {other:?}"),
        }
        let plain = DescriptorRegistry::new();
        let items: Vec<_> = plain.parse_loop(&t).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::Unknown { tag: 62, .. }));
    }

    /// The specs say "the PMT shall contain" the 'AVSV'/'AVSA'
    /// registration_descriptor without naming the loop, and put the AVS
    /// descriptor in the ES_info loop: a registration in `program_info` must
    /// scope the ES_info loops that follow (`avs_scope` + `parse_loop_in_scope`).
    #[test]
    fn program_info_registration_scopes_es_info_descriptors() {
        use crate::descriptors::avs3_video::Avs3VideoDescriptor;
        use crate::descriptors::private_data_specifier::PDS_AVSV;
        let program_info = [0x05, 4, b'A', b'V', b'S', b'V'];
        // ES_info of the stream_type 0xD4 entry: just the AVS3 video descriptor.
        let es_info = [0xD1, 8, 0x22, 0x42, 0xAA, 0x4B, 9, 14, 9, 0xFF];
        let mut reg = DescriptorRegistry::new();
        reg.with_avs3_video();

        // Per-loop scoping alone never sees the registration: Unknown.
        let items: Vec<_> = reg.parse_loop(&es_info).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::Unknown { tag: 0xD1, .. }));

        // Walk program_info, carry the scope into the ES_info walk.
        let mut pi = reg.parse_loop(&program_info);
        assert_eq!(pi.avs_scope(), None, "no entry consumed yet");
        for item in pi.by_ref() {
            item.unwrap();
        }
        let scope = pi.avs_scope();
        assert_eq!(scope, Some(PDS_AVSV));
        let items: Vec<_> = reg
            .parse_loop_in_scope(&es_info, scope)
            .collect::<Result<_, _>>()
            .unwrap();
        match &items[0] {
            AnyDescriptor::Other { tag: 0xD1, value } => {
                assert!(value.downcast_ref::<Avs3VideoDescriptor>().is_some());
            }
            other => panic!("expected Avs3Video via inherited scope, got {other:?}"),
        }
        // A non-AVS program_info registration opens no scope to inherit.
        let hevc = [0x05, 4, b'H', b'E', b'V', b'C'];
        let mut pi = reg.parse_loop(&hevc);
        for item in pi.by_ref() {
            item.unwrap();
        }
        assert_eq!(pi.avs_scope(), None);
    }

    /// AVS3_video_descriptor (tag 0xD1): scoped by a 'AVSV' private_data_specifier
    /// (TS 101 154) or a 'AVSV' registration_descriptor (T/AI 109.6 / GY/T 420).
    #[test]
    fn avs3_video_descriptor_scopes() {
        use crate::descriptors::avs3_video::Avs3VideoDescriptor;
        use crate::descriptors::private_data_specifier::PDS_AVSV;
        let desc = [0xD1, 8, 0x22, 0x42, 0xAA, 0x4B, 9, 14, 9, 0xFF];
        let mut reg = DescriptorRegistry::new();
        reg.with_avs3_video();
        let check = |bytes: &[u8], idx: usize| {
            let items: Vec<_> = reg.parse_loop(bytes).collect::<Result<_, _>>().unwrap();
            match &items[idx] {
                AnyDescriptor::Other { tag, value } => {
                    assert_eq!(*tag, 0xD1);
                    assert_eq!(
                        value
                            .downcast_ref::<Avs3VideoDescriptor>()
                            .unwrap()
                            .profile_id,
                        0x22
                    );
                }
                other => panic!("expected Other, got {other:?}"),
            }
        };
        let mut pds = pds_descriptor(PDS_AVSV);
        pds.extend_from_slice(&desc);
        check(&pds, 1);
        let mut registration = vec![0x05, 4, b'A', b'V', b'S', b'V'];
        registration.extend_from_slice(&desc);
        check(&registration, 1);
        // No scope, or a different registration ('AC-3'), leaves it Unknown.
        let items: Vec<_> = reg.parse_loop(&desc).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::Unknown { tag: 0xD1, .. }));
        let mut other = vec![0x05, 4, b'A', b'C', b'-', b'3'];
        other.extend_from_slice(&desc);
        let items: Vec<_> = reg.parse_loop(&other).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[1], AnyDescriptor::Unknown { tag: 0xD1, .. }));
        // The audio descriptor also accepts the 'AVSA' registration form.
        let adesc = [0xD2, 2, 0x72, 0x45];
        let mut areg = DescriptorRegistry::new();
        areg.with_avs3_audio();
        let mut b = vec![0x05, 4, b'A', b'V', b'S', b'A'];
        b.extend_from_slice(&adesc);
        let items: Vec<_> = areg.parse_loop(&b).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[1], AnyDescriptor::Other { tag: 0xD2, .. }));
    }

    /// T/AI 109.7 clause 8.1.3.4 + TS 101 154 V2.10.1 4.1.8.35: tag 0xD2 is the
    /// AVS3_audio_descriptor only under private_data_specifier 'AVSA'.
    #[test]
    fn avs3_audio_descriptor_is_pds_scoped() {
        use crate::descriptors::avs3_audio::Avs3AudioDescriptor;
        use crate::descriptors::private_data_specifier::PDS_AVSA;
        let desc = [0xD2, 6, 0x03, 0xAA, 0x95, 0x04, 0x00, 0x6A];
        let mut reg = DescriptorRegistry::new();
        reg.with_avs3_audio();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_AVSA));
        bytes.extend_from_slice(&desc);
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        match &items[1] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0xD2);
                let d = value.downcast_ref::<Avs3AudioDescriptor>().unwrap();
                assert_eq!(d.sampling_frequency_index, 3);
            }
            other => panic!("expected Other (Avs3AudioDescriptor), got {other:?}"),
        }

        // Without the 'AVSA' specifier (or under another) the tag stays Unknown.
        let items: Vec<_> = reg.parse_loop(&desc).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::Unknown { tag: 0xD2, .. }));
        let mut other = Vec::new();
        other.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        other.extend_from_slice(&desc);
        let items: Vec<_> = reg.parse_loop(&other).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[1], AnyDescriptor::Unknown { tag: 0xD2, .. }));
    }

    /// GY/T 270-2013 Table 8: tag 0x86 is the China caption_service_descriptor
    /// once `with_china_caption_service` is enabled, and stays Unknown otherwise.
    #[test]
    fn china_caption_service_is_opt_in() {
        use crate::descriptors::china_caption_service::ChinaCaptionServiceDescriptor;
        let desc = [
            0x86, 9, 0xE1, b'c', b'h', b'i', 0xC1, 0xC2, 0xFF, 0xE1, 0x00,
        ];
        let plain = DescriptorRegistry::new();
        let items: Vec<_> = plain.parse_loop(&desc).collect::<Result<_, _>>().unwrap();
        assert!(matches!(items[0], AnyDescriptor::Unknown { tag: 0x86, .. }));
        let mut reg = DescriptorRegistry::new();
        reg.with_china_caption_service();
        let items: Vec<_> = reg.parse_loop(&desc).collect::<Result<_, _>>().unwrap();
        match &items[0] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x86);
                let d = value
                    .downcast_ref::<ChinaCaptionServiceDescriptor>()
                    .unwrap();
                assert_eq!(d.services.len(), 1);
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn pds_agnostic_matches_without_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.register::<PdsAgnostic>();

        let bytes = [0x84, 0x01, 0xDD];
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 1);
        match &items[0] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x84);
                let c = value.downcast_ref::<PdsAgnostic>().unwrap();
                assert_eq!(c.z, 0xDD);
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn pds_scoped_takes_precedence_over_agnostic() {
        // Two parsers compete for the SAME tag 0x83: one PDS-agnostic, one
        // scoped to EACEM. With no PDS active the agnostic one wins; once an
        // EACEM private_data_specifier appears, the scoped one takes over.
        #[derive(Debug, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct Agnostic83 {
            a: u8,
        }

        impl<'a> broadcast_common::Parse<'a> for Agnostic83 {
            type Error = crate::error::Error;
            fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
                if bytes.len() < 3 {
                    return Err(crate::error::Error::BufferTooShort {
                        need: 3,
                        have: bytes.len(),
                        what: "Agnostic83",
                    });
                }
                Ok(Self { a: bytes[2] })
            }
        }

        impl DescriptorDef<'_> for Agnostic83 {
            const TAG: u8 = 0x83;
            const NAME: &'static str = "AGNOSTIC_83";
        }

        let mut reg = DescriptorRegistry::new();
        reg.register::<Agnostic83>();
        reg.register_for_pds::<PdsEacem>(PDS_EACEM);

        // No PDS → agnostic wins
        let items: Vec<_> = reg
            .parse_loop(&[0x83, 0x01, 0xEE])
            .collect::<Result<_, _>>()
            .unwrap();
        match &items[0] {
            AnyDescriptor::Other { value, .. } => {
                assert!(value.downcast_ref::<Agnostic83>().is_some());
                assert!(value.downcast_ref::<PdsEacem>().is_none());
            }
            other => panic!("expected Other, got {other:?}"),
        }

        // With PDS EACEM → PDS-scoped wins
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_EACEM));
        bytes.extend_from_slice(&[0x83, 0x01, 0xFF]);
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        match &items[1] {
            AnyDescriptor::Other { value, .. } => {
                assert!(value.downcast_ref::<PdsEacem>().is_some());
                assert!(value.downcast_ref::<Agnostic83>().is_none());
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    fn logical_channel_descriptor(service_id: u16, visible: bool, lcn: u16) -> Vec<u8> {
        // One entry: tag=0x83, len=4, entry(4 bytes)
        let mut v = vec![crate::descriptors::logical_channel::TAG, 4];
        v.extend_from_slice(&service_id.to_be_bytes());
        let flags = (u8::from(visible) << 7) | ((lcn >> 8) as u8 & 0x03);
        v.push(flags);
        v.push((lcn & 0xFF) as u8);
        v
    }

    #[test]
    fn logical_channel_pds_scoped_matches_correct_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.with_logical_channel_for_pds(PDS_EACEM);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_EACEM));
        bytes.extend_from_slice(&logical_channel_descriptor(0x1234, true, 101));

        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnyDescriptor::PrivateDataSpecifier(_)));
        match &items[1] {
            AnyDescriptor::LogicalChannel(lc) => {
                assert_eq!(lc.entries.len(), 1);
                assert_eq!(lc.entries[0].service_id, 0x1234);
                assert!(lc.entries[0].visible_service);
                assert_eq!(lc.entries[0].logical_channel_number, 101);
            }
            other => panic!("expected LogicalChannel, got {other:?}"),
        }
    }

    #[test]
    fn logical_channel_pds_scoped_rejects_no_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.with_logical_channel_for_pds(PDS_EACEM);

        let bytes = logical_channel_descriptor(0x1234, true, 101);
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            AnyDescriptor::Unknown { tag: 0x83, .. }
        ));
    }

    #[test]
    fn logical_channel_pds_scoped_rejects_wrong_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.with_logical_channel_for_pds(PDS_EACEM);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        bytes.extend_from_slice(&logical_channel_descriptor(0x1234, true, 101));

        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnyDescriptor::PrivateDataSpecifier(_)));
        assert!(matches!(
            &items[1],
            AnyDescriptor::Unknown { tag: 0x83, .. }
        ));
    }

    #[test]
    fn logical_channel_pds_scoped_multiple_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.with_logical_channel_for_pds(PDS_EACEM);
        reg.with_logical_channel_for_pds(PDS_NORDIG);

        // EACEM → LogicalChannel
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_EACEM));
        bytes.extend_from_slice(&logical_channel_descriptor(0x0001, true, 1));
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert!(matches!(&items[1], AnyDescriptor::LogicalChannel(_)));

        // NorDig → LogicalChannel
        let mut bytes2 = Vec::new();
        bytes2.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        bytes2.extend_from_slice(&logical_channel_descriptor(0x0002, false, 2));
        let items2: Vec<_> = reg.parse_loop(&bytes2).collect::<Result<_, _>>().unwrap();
        assert!(matches!(&items2[1], AnyDescriptor::LogicalChannel(_)));
    }

    #[test]
    fn iter_with_extensions_surfaces_custom_extension() {
        use crate::descriptors::any::{AnyDescriptor, DescriptorLoop};
        use crate::descriptors::extension::registry::ExtensionRegistry;

        #[derive(Debug, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct MyCustomExt {
            payload: Vec<u8>,
        }

        impl<'a> broadcast_common::Parse<'a> for MyCustomExt {
            type Error = crate::error::Error;
            fn parse(sel: &'a [u8]) -> crate::Result<Self> {
                Ok(Self {
                    payload: sel.to_vec(),
                })
            }
        }

        impl crate::descriptors::extension::ExtensionBodyDef<'_> for MyCustomExt {
            const TAG_EXTENSION: u8 = 0x42;
            const NAME: &'static str = "MY_CUSTOM_EXT";
        }

        let mut ext_reg = ExtensionRegistry::new();
        ext_reg.register::<MyCustomExt>();

        let desc_reg = DescriptorRegistry::new();

        // Build a descriptor loop with a short_event + a 0x7F extension with tag_extension 0x42
        let mut loop_bytes = vec![
            0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
        ];
        // extension: tag=0x7F, length=3, tag_extension=0x42, selector=[0xAB, 0xCD]
        loop_bytes.extend_from_slice(&[0x7F, 0x03, 0x42, 0xAB, 0xCD]);

        let dl = DescriptorLoop::new(&loop_bytes);
        let items: Vec<_> = dl
            .iter_with_extensions(&desc_reg, &ext_reg)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(items.len(), 2);
        // First item is a regular descriptor
        assert!(matches!(
            &items[0],
            ExtIterItem::Descriptor(AnyDescriptor::ShortEvent(_))
        ));
        // Second item surfaces the custom extension body (not Raw!)
        match &items[1] {
            ExtIterItem::CustomExtension {
                tag_extension,
                value,
            } => {
                assert_eq!(*tag_extension, 0x42);
                let concrete = value.downcast_ref::<MyCustomExt>().unwrap();
                assert_eq!(concrete.payload, &[0xAB, 0xCD]);
            }
            other => panic!("expected CustomExtension, got {other:?}"),
        }
    }
}

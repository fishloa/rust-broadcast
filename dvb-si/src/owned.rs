//! Own a parsed view past the input buffer's borrow (feature `yoke`).
//!
//! Tables and descriptor loops parse **zero-copy**, borrowing the input section
//! slice — [`Pmt<'a>`](crate::tables::pmt::Pmt), [`Sdt<'a>`](crate::tables::sdt::Sdt),
//! [`DescriptorLoop<'a>`](crate::descriptors::DescriptorLoop), and so on. That is
//! ideal for parse-and-discard, but a consumer that needs to **retain** a parsed
//! table — stash it in a struct field, a cache, or a `tokio::sync::watch`
//! channel — would otherwise have to re-parse on every access or maintain a
//! hand-written owned mirror type.
//!
//! [`Owned`] bundles the owned backing bytes (`Arc<[u8]>`) and the borrowing
//! view into one `'static`, cheaply-`Clone`, `Send + Sync` value via the
//! [`yoke`] crate. The view's lifetime disappears from your signatures, and no
//! re-parse happens on access.
//!
//! ```
//! use std::sync::Arc;
//! use dvb_si::owned::Owned;
//! use dvb_si::tables::pmt::Pmt;
//! use dvb_common::Parse;
//!
//! # let section: Vec<u8> = dvb_si::owned::doc::pmt_section();
//! // `section` is the complete PMT section bytes (e.g. straight off the demux).
//! let bytes: Arc<[u8]> = Arc::from(section);
//!
//! // Parse once, keep the result — no borrow of a local buffer escapes.
//! let owned: Owned<Pmt<'static>> =
//!     Owned::try_new(bytes, |b| Pmt::parse(b))?;
//!
//! // The owned value is 'static, so it can live in a struct field…
//! struct Cache { pmt: Owned<Pmt<'static>> }
//! let cache = Cache { pmt: owned };
//!
//! // …and move across a thread boundary, then read the typed view back out
//! // with no re-parse.
//! let handle = std::thread::spawn(move || {
//!     let pmt: &Pmt = cache.pmt.get();
//!     (pmt.program_number, pmt.streams.len())
//! });
//! let (program_number, stream_count) = handle.join().unwrap();
//! assert_eq!(program_number, 1);
//! assert_eq!(stream_count, 1);
//! # Ok::<(), dvb_si::error::Error>(())
//! ```

use std::sync::Arc;

use yoke::trait_hack::YokeTraitHack;
use yoke::{Yoke, Yokeable};

/// An owned, `'static`, `Send + Sync` bundle of (backing bytes, parsed view).
///
/// `Y` is the view type with its lifetime set to `'static` — e.g.
/// `Owned<Pmt<'static>>`, `Owned<Sdt<'static>>`,
/// `Owned<DescriptorLoop<'static>>`. The backing buffer is an `Arc<[u8]>`, so
/// [`Clone`] is a refcount bump and the view is shared, never re-parsed.
///
/// Construct one with [`Owned::try_new`] (fallible parse) or [`Owned::new`]
/// (infallible), then read the borrowing view with [`Owned::get`].
pub struct Owned<Y: for<'a> Yokeable<'a>> {
    yoke: Yoke<Y, Arc<[u8]>>,
}

impl<Y: for<'a> Yokeable<'a>> Owned<Y> {
    /// Parse `bytes` into a view and bundle the two into an [`Owned`].
    ///
    /// `parse` receives a borrow of the backing bytes and returns the borrowing
    /// view (or an error). The borrow does not escape `parse`: `yoke` re-binds
    /// the view's lifetime to the `Arc<[u8]>` it now co-owns.
    ///
    /// # Errors
    ///
    /// Returns `parse`'s error verbatim if parsing fails.
    pub fn try_new<F, E>(bytes: Arc<[u8]>, parse: F) -> Result<Self, E>
    where
        F: for<'a> FnOnce(&'a [u8]) -> Result<<Y as Yokeable<'a>>::Output, E>,
    {
        Ok(Self {
            yoke: Yoke::try_attach_to_cart(bytes, parse)?,
        })
    }

    /// Parse `bytes` into a view with an infallible parser and bundle them.
    pub fn new<F>(bytes: Arc<[u8]>, parse: F) -> Self
    where
        F: for<'a> FnOnce(&'a [u8]) -> <Y as Yokeable<'a>>::Output,
    {
        Self {
            yoke: Yoke::attach_to_cart(bytes, parse),
        }
    }

    /// Borrow the parsed view. No re-parse; this is a field read.
    #[must_use]
    pub fn get(&self) -> &<Y as Yokeable<'_>>::Output {
        self.yoke.get()
    }

    /// The backing section bytes the view borrows from.
    #[must_use]
    pub fn backing_bytes(&self) -> &[u8] {
        self.yoke.backing_cart()
    }
}

// Cloning is a refcount bump on the `Arc<[u8]>` plus a re-binding of the
// (already-parsed) view — no re-parse. The bound mirrors yoke's own `Clone`
// impl for `Yoke<Y, CloneableCart>`.
impl<Y: for<'a> Yokeable<'a>> Clone for Owned<Y>
where
    for<'a> YokeTraitHack<<Y as Yokeable<'a>>::Output>: Clone,
{
    fn clone(&self) -> Self {
        Self {
            yoke: self.yoke.clone(),
        }
    }
}

impl<Y> std::fmt::Debug for Owned<Y>
where
    Y: for<'a> Yokeable<'a>,
    for<'a> <Y as Yokeable<'a>>::Output: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Owned").field("view", self.get()).finish()
    }
}

#[doc(hidden)]
pub mod doc {
    //! Hidden helpers for the module-level doctest. Not part of the public API.

    /// Build a minimal one-stream PMT section via the serializer, so the
    /// doctest is self-contained and actually round-trips through the parser.
    #[must_use]
    pub fn pmt_section() -> Vec<u8> {
        use crate::tables::pmt::{Pmt, PmtStream};
        use dvb_common::Serialize;

        let pmt = Pmt {
            program_number: 1,
            version_number: 0,
            current_next_indicator: true,
            pcr_pid: 0x0100,
            program_info: crate::descriptors::DescriptorLoop::new(&[]),
            streams: vec![PmtStream {
                stream_type: 0x1B, // H.264 video
                elementary_pid: 0x0101,
                es_info: crate::descriptors::DescriptorLoop::new(&[]),
            }],
        };
        let mut section = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut section).unwrap();
        section
    }
}

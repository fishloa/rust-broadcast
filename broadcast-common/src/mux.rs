//! Container-mux vocabulary traits: [`Unpackage`] / [`Package`] and
//! [`Decrypt`] / [`Encrypt`].
//!
//! Where [`Parse`](crate::Parse) / [`Serialize`](crate::Serialize) are the
//! *wire-structure* contract (one concrete type ⇄ its bytes), these four traits
//! are the *container-mux* contract: they move between a packaged container
//! representation (an fMP4/CMAF segment, an HLS playlist, an MPEG-TS multiplex,
//! …) and an in-memory *media* intermediate representation (elementary tracks +
//! coded samples). They are deliberately abstract — the associated types name
//! the input/output/media/key material, and no concrete media or codec type
//! appears in `broadcast-common`. Concrete `Media` IR types and impls live in
//! the consuming crates (e.g. `transmux`).
//!
//! The pairs are inverses:
//!
//! - [`Unpackage`] ⇄ [`Package`] — demux a container into media, and mux media
//!   back into a container. Round-tripping `unpackage` then `package` should
//!   reproduce the same media structure.
//! - [`Decrypt`] ⇄ [`Encrypt`] — remove and (re)apply sample protection on an
//!   in-place media IR. Round-tripping `decrypt` then `encrypt` with the same
//!   key/config material should reproduce the same protected media.
//!
//! Each trait picks its own error type via `type Error`, mirroring the
//! [`Parse`](crate::Parse) / [`Serialize`](crate::Serialize) style, so
//! domain-specific error variants stay visible to the caller.

/// Demux a packaged container into an in-memory media representation.
///
/// The inverse of [`Package`]. `Self::Input` is the packaged form (typically a
/// byte slice or a stream of segments); `Self::Media` is the decoded elementary
/// representation (tracks + coded samples).
pub trait Unpackage {
    /// The packaged input this demuxer consumes (e.g. `&[u8]` of an fMP4 file).
    type Input;
    /// The in-memory media representation produced.
    type Media;
    /// The error type this implementer returns.
    type Error;

    /// Demux `input` into a [`Self::Media`], borrowing or owning as the
    /// implementer chooses. Returns `Err(Self::Error)` on any container
    /// violation or buffer underrun.
    fn unpackage(&mut self, input: Self::Input) -> Result<Self::Media, Self::Error>;
}

/// Mux an in-memory media representation into a packaged container.
///
/// The inverse of [`Unpackage`]. `Self::Output` is the packaged form produced
/// (e.g. a `Vec<u8>` of an fMP4 segment or a `String` playlist).
pub trait Package {
    /// The in-memory media representation this muxer consumes.
    type Media;
    /// The packaged output produced (e.g. `Vec<u8>` or `String`).
    type Output;
    /// The error type this implementer returns.
    type Error;

    /// Mux `media` into a [`Self::Output`]. Returns `Err(Self::Error)` on any
    /// constraint violation (e.g. an empty track list or an oversized field).
    fn package(&mut self, media: &Self::Media) -> Result<Self::Output, Self::Error>;
}

/// Remove sample protection from a media representation, in place.
///
/// The inverse of [`Encrypt`]. `Self::Keys` is the key material required to
/// unprotect the samples (e.g. per-key-ID content keys).
pub trait Decrypt {
    /// The in-memory media representation operated on in place.
    type Media;
    /// The key material required to unprotect samples.
    type Keys;
    /// The error type this implementer returns.
    type Error;

    /// Decrypt the protected samples in `media` in place using `keys`.
    fn decrypt(&self, media: &mut Self::Media, keys: &Self::Keys) -> Result<(), Self::Error>;
}

/// Apply sample protection to a media representation, in place.
///
/// The inverse of [`Decrypt`]. `Self::Config` describes the protection scheme
/// to apply (e.g. `cenc`/`cbcs` scheme + key IDs + IV material).
pub trait Encrypt {
    /// The in-memory media representation operated on in place.
    type Media;
    /// The protection scheme configuration to apply.
    type Config;
    /// The error type this implementer returns.
    type Error;

    /// Encrypt the samples in `media` in place per `cfg`.
    fn encrypt(&self, media: &mut Self::Media, cfg: &Self::Config) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    // A trivial media IR + a full set of impls, to prove the traits are usable
    // (object-safe method shapes, generic associated types resolve) and that
    // the inverse pairs round-trip on a hand-built value.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    struct Media {
        tracks: Vec<Vec<u8>>,
    }

    struct Codec;

    impl Unpackage for Codec {
        type Input = &'static [u8];
        type Media = Media;
        type Error = ();
        fn unpackage(&mut self, input: Self::Input) -> Result<Self::Media, Self::Error> {
            Ok(Media {
                tracks: input.iter().map(|b| alloc::vec![*b]).collect(),
            })
        }
    }

    impl Package for Codec {
        type Media = Media;
        type Output = Vec<u8>;
        type Error = ();
        fn package(&mut self, media: &Self::Media) -> Result<Self::Output, Self::Error> {
            Ok(media.tracks.iter().map(|t| t[0]).collect())
        }
    }

    impl Decrypt for Codec {
        type Media = Media;
        type Keys = u8;
        type Error = ();
        fn decrypt(&self, media: &mut Self::Media, keys: &Self::Keys) -> Result<(), Self::Error> {
            for t in &mut media.tracks {
                for b in t {
                    *b ^= *keys;
                }
            }
            Ok(())
        }
    }

    impl Encrypt for Codec {
        type Media = Media;
        type Config = u8;
        type Error = ();
        fn encrypt(&self, media: &mut Self::Media, cfg: &Self::Config) -> Result<(), Self::Error> {
            for t in &mut media.tracks {
                for b in t {
                    *b ^= *cfg;
                }
            }
            Ok(())
        }
    }

    #[test]
    fn traits_compile_and_round_trip() {
        let mut codec = Codec;
        let input: &'static [u8] = &[1, 2, 3];
        let media = codec.unpackage(input).unwrap();
        assert_eq!(media.tracks.len(), 3);
        // Package ⇄ Unpackage inverse on this trivial IR.
        assert_eq!(codec.package(&media).unwrap(), alloc::vec![1u8, 2, 3]);

        // Encrypt ⇄ Decrypt inverse (XOR with the same key is self-inverse).
        let mut m = media.clone();
        codec.encrypt(&mut m, &0xAA).unwrap();
        assert_ne!(m, media);
        codec.decrypt(&mut m, &0xAA).unwrap();
        assert_eq!(m, media);
    }
}

//! The object-safe trait one codec adapter implements.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::codec::CodecParameters;
use crate::descriptor::CodecDescriptor;
use crate::encoder_config::EncoderConfig;
use crate::error::CodecError;
use crate::traits::{Decoder, Encoder};

/// One adapter, offering any number of implementations.
///
/// A provider is a value an application registers with a
/// [`Registry`](crate::Registry); nothing registers itself. The trait is
/// object-safe, so a registry holds `Box<dyn CodecProvider>` and links any
/// number of adapters without naming one at its call sites.
pub trait CodecProvider {
    /// The provider's name.
    ///
    /// It is the provider side of a [`Preference`](crate::Preference), so it
    /// must be non-empty and made only of lower-case ASCII letters, digits,
    /// `-` and `_`. A [`Registry`](crate::Registry) refuses a provider whose
    /// name breaks that rule, and refuses two providers with the same name.
    fn name(&self) -> &str;

    /// Every implementation this provider offers, in the order the registry
    /// should try them. Providers are asked on every lookup, so the list may
    /// depend on the machine (a hardware encoder absent on this host is absent
    /// here too).
    fn descriptors(&self) -> Vec<CodecDescriptor>;

    /// Build a decoder for `parameters`.
    ///
    /// `implementation` is one of the names [`Self::descriptors`] lists for
    /// `parameters.codec()` in [`Direction::Decode`](crate::Direction::Decode).
    ///
    /// # Errors
    /// [`CodecError::Unsupported`] or [`CodecError::Refused`] when this
    /// implementation cannot serve the request: a registry in
    /// [`Fallback::NextCandidate`](crate::Fallback::NextCandidate) then tries
    /// the next candidate. [`CodecError::Failed`] stops every further attempt.
    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError>;

    /// Build an encoder from `config`.
    ///
    /// `implementation` is one of the names [`Self::descriptors`] lists for
    /// `config.codec()` in [`Direction::Encode`](crate::Direction::Encode).
    ///
    /// # Errors
    /// As [`Self::make_decoder`], plus [`CodecError::Refused`] naming any key
    /// in [`EncoderConfig::options`] this provider does not recognise. An
    /// unknown key is never ignored.
    fn make_encoder(
        &self,
        implementation: &str,
        config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError>;
}

/// The character in `name` that stops it being a provider or implementation
/// name, when one does.
pub(crate) fn offending_character(name: &str) -> Option<char> {
    name.chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
}

//! Assembling providers, and choosing among them.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::codec::{CodecId, CodecParameters};
use crate::descriptor::CodecDescriptor;
use crate::direction::Direction;
use crate::encoder_config::EncoderConfig;
use crate::error::CodecError;
use crate::fallback::Fallback;
use crate::preference::Preference;
use crate::provider::{CodecProvider, offending_character};
use crate::registry_error::RegistryError;
use crate::traits::{Decoder, Encoder};

/// The function [`Registry::select_with`] takes: every candidate for a codec
/// and direction, in registration order, in; the index of the one to try first
/// out.
pub type Selector = Box<dyn Fn(&[CodecDescriptor]) -> Option<usize>>;

/// The providers an application linked, and how to choose among them.
///
/// A registry is a value the application builds; there is no static registry
/// and no link-time self-registration. Every provider is added with
/// [`Self::with`], every choice with [`Self::prefer`] or
/// [`Self::select_with`], and the selection rule is stated once at the crate
/// root.
pub struct Registry {
    providers: Vec<Box<dyn CodecProvider>>,
    preferences: Vec<Preference>,
    fallback: Fallback,
    selector: Option<Selector>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let providers: Vec<&str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .collect();
        f.debug_struct("Registry")
            .field("providers", &providers)
            .field("preferences", &self.preferences)
            .field("fallback", &self.fallback)
            .field("selector", &self.selector.is_some())
            .finish()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// An empty registry with no preference and [`Fallback::Strict`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            providers: Vec::new(),
            preferences: Vec::new(),
            fallback: Fallback::Strict,
            selector: None,
        }
    }

    /// Register `provider`, which is then tried in the order it was added.
    ///
    /// # Errors
    /// [`RegistryError::InvalidName`] when the provider's name is empty or
    /// breaks the naming rule; [`RegistryError::DuplicateName`] when a provider
    /// with that name is already registered.
    pub fn with(mut self, provider: Box<dyn CodecProvider>) -> Result<Self, RegistryError> {
        let name = provider.name().to_string();
        if let Some(character) = offending_character(&name) {
            return Err(RegistryError::invalid_name(&name, Some(character)));
        }
        if name.is_empty() {
            return Err(RegistryError::invalid_name(&name, None));
        }
        if self.provider(&name).is_some() {
            return Err(RegistryError::duplicate_name(&name));
        }
        self.providers.push(provider);
        Ok(self)
    }

    /// Add `preference`, replacing an earlier one for the same codec and
    /// direction.
    #[must_use]
    pub fn prefer(mut self, preference: Preference) -> Self {
        self.preferences.retain(|existing| {
            !(existing.codec() == preference.codec()
                && existing.direction() == preference.direction())
        });
        self.preferences.push(preference);
        self
    }

    /// Set what happens when a candidate cannot serve.
    #[must_use]
    pub fn fallback(mut self, fallback: Fallback) -> Self {
        self.fallback = fallback;
        self
    }

    /// Set the function that picks the first candidate, given every candidate
    /// for the codec and direction in registration order.
    #[must_use]
    pub fn select_with(mut self, selector: Selector) -> Self {
        self.selector = Some(selector);
        self
    }

    /// Every implementation every provider offers, in registration order and,
    /// within a provider, in the order it lists them.
    #[must_use]
    pub fn list(&self) -> Vec<CodecDescriptor> {
        self.providers
            .iter()
            .flat_map(|provider| provider.descriptors())
            .collect()
    }

    /// Every candidate for `codec` in `direction`, in the order they will be
    /// tried.
    ///
    /// This reports the order alone: a preference that names something not
    /// registered is reported by [`Self::decoder`] and [`Self::encoder`], never
    /// here.
    #[must_use]
    pub fn candidates(&self, codec: CodecId, direction: Direction) -> Vec<CodecDescriptor> {
        self.arranged(self.registered(codec, direction), codec, direction)
    }

    /// Build a decoder for `parameters` from the first candidate that serves
    /// it.
    ///
    /// # Errors
    /// [`CodecError::Unsupported`] when the codec has no
    /// [`Direction::Decode`] candidate, when a matching preference names a
    /// provider or implementation that is not registered, or when every
    /// candidate was skipped under [`Fallback::NextCandidate`]; otherwise the
    /// candidate's own error.
    pub fn decoder(&self, parameters: &CodecParameters) -> Result<Box<dyn Decoder>, CodecError> {
        let codec = parameters.codec();
        let direction = Direction::Decode;
        let candidates = self.checked(codec, direction)?;
        self.build(&candidates, codec, direction, |provider, implementation| {
            provider.make_decoder(implementation, parameters)
        })
    }

    /// Build an encoder from `config`.
    ///
    /// # Errors
    /// As [`Self::decoder`], for [`Direction::Encode`].
    pub fn encoder(&self, config: &EncoderConfig) -> Result<Box<dyn Encoder>, CodecError> {
        let codec = config.codec();
        let direction = Direction::Encode;
        let candidates = self.checked(codec, direction)?;
        self.build(&candidates, codec, direction, |provider, implementation| {
            provider.make_encoder(implementation, config)
        })
    }

    fn provider(&self, name: &str) -> Option<&dyn CodecProvider> {
        self.providers
            .iter()
            .find(|provider| provider.name() == name)
            .map(|provider| provider.as_ref())
    }

    fn registered(&self, codec: CodecId, direction: Direction) -> Vec<CodecDescriptor> {
        self.providers
            .iter()
            .flat_map(|provider| provider.descriptors())
            .filter(|descriptor| descriptor.codec() == codec && descriptor.direction() == direction)
            .collect()
    }

    /// The preference governing `codec` in `direction`: the one naming that
    /// direction, else the one naming both.
    fn preference_for(&self, codec: CodecId, direction: Direction) -> Option<&Preference> {
        let names = |preference: &&Preference, wanted: Option<Direction>| {
            preference.codec() == codec && preference.direction() == wanted
        };
        self.preferences
            .iter()
            .find(|preference| names(preference, Some(direction)))
            .or_else(|| {
                self.preferences
                    .iter()
                    .find(|preference| names(preference, None))
            })
    }

    fn arranged(
        &self,
        mut candidates: Vec<CodecDescriptor>,
        codec: CodecId,
        direction: Direction,
    ) -> Vec<CodecDescriptor> {
        if candidates.is_empty() {
            return candidates;
        }
        if let Some(selector) = &self.selector
            && let Some(index) = selector(&candidates)
            && index < candidates.len()
        {
            candidates[..=index].rotate_right(1);
            return candidates;
        }
        let Some(preference) = self.preference_for(codec, direction) else {
            return candidates;
        };
        let (front, rest): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|descriptor| matches_preference(descriptor, preference));
        let mut ordered = front;
        ordered.extend(rest);
        ordered
    }

    fn checked(
        &self,
        codec: CodecId,
        direction: Direction,
    ) -> Result<Vec<CodecDescriptor>, CodecError> {
        let candidates = self.registered(codec, direction);
        if candidates.is_empty() {
            return Err(CodecError::unsupported(format!(
                "{} of {}",
                direction.name(),
                codec.name()
            )));
        }
        if let Some(preference) = self.preference_for(codec, direction) {
            self.check_registered(preference, &candidates, codec, direction)?;
        }
        Ok(self.arranged(candidates, codec, direction))
    }

    fn check_registered(
        &self,
        preference: &Preference,
        candidates: &[CodecDescriptor],
        codec: CodecId,
        direction: Direction,
    ) -> Result<(), CodecError> {
        let what = format!("{} of {}", direction.name(), codec.name());
        let provider = preference.provider();
        if self.provider(provider).is_none() {
            return Err(CodecError::unsupported(format!(
                "{what}: provider `{provider}` is not registered"
            )));
        }
        if candidates
            .iter()
            .any(|descriptor| matches_preference(descriptor, preference))
        {
            return Ok(());
        }
        Err(match preference.implementation() {
            Some(implementation) => CodecError::unsupported(format!(
                "{what}: implementation `{implementation}` of provider `{provider}` is not registered"
            )),
            None => {
                CodecError::unsupported(format!("{what}: provider `{provider}` offers nothing"))
            }
        })
    }

    fn build<T>(
        &self,
        candidates: &[CodecDescriptor],
        codec: CodecId,
        direction: Direction,
        mut make: impl FnMut(&dyn CodecProvider, &str) -> Result<T, CodecError>,
    ) -> Result<T, CodecError> {
        let mut tried: Vec<String> = Vec::new();
        for descriptor in candidates {
            let Some(provider) = self.provider(descriptor.provider()) else {
                return Err(CodecError::unsupported(format!(
                    "provider `{}` is not registered",
                    descriptor.provider()
                )));
            };
            match make(provider, descriptor.implementation()) {
                Ok(built) => return Ok(built),
                Err(error) if self.fallback == Fallback::Strict => return Err(error),
                Err(error @ (CodecError::Unsupported { .. } | CodecError::Refused { .. })) => {
                    tried.push(format!(
                        "{}:{} ({error})",
                        descriptor.provider(),
                        descriptor.implementation()
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Err(CodecError::unsupported(format!(
            "{} of {}: tried {}",
            direction.name(),
            codec.name(),
            tried.join(", ")
        )))
    }
}

/// Whether `descriptor` is one the preference names.
fn matches_preference(descriptor: &CodecDescriptor, preference: &Preference) -> bool {
    descriptor.provider() == preference.provider()
        && preference
            .implementation()
            .is_none_or(|implementation| descriptor.implementation() == implementation)
}

//! Assembling providers, and choosing among them.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::availability::Probe;
use crate::codec::{CodecId, CodecParameters};
use crate::descriptor::CodecDescriptor;
use crate::direction::Direction;
use crate::encoder_config::EncoderConfig;
use crate::error::CodecError;
use crate::fallback::Fallback;
use crate::inventory::InventoryEntry;
use crate::preference::Preference;
use crate::profile::CodecProfile;
use crate::provider::{CodecProvider, offending_character};
use crate::provider_info::ProviderInfo;
use crate::registry_error::RegistryError;
use crate::rejection::ProfileRejection;
use crate::traits::{Decoder, Encoder};
use crate::trim::TrimmedDecoder;

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

    /// Add `preference`, replacing an earlier one for the same codec, profile
    /// and direction.
    #[must_use]
    pub fn prefer(mut self, preference: Preference) -> Self {
        self.preferences.retain(|existing| {
            !(existing.codec() == preference.codec()
                && existing.profile() == preference.profile()
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

    /// Every registered provider, described by its own
    /// [`CodecProvider::info`], in registration order.
    #[must_use]
    pub fn providers(&self) -> Vec<ProviderInfo> {
        self.providers
            .iter()
            .map(|provider| provider.info())
            .collect()
    }

    /// One [`InventoryEntry`] per descriptor, in registration order and, within
    /// a provider, in the order it lists them.
    ///
    /// When `probe` is false no provider is asked to open anything and every
    /// entry is [`Availability::Unknown`](crate::Availability::Unknown), because
    /// opening a hardware codec has a cost. When it is true each descriptor is
    /// probed exactly once.
    #[must_use]
    pub fn inventory(&self, probe: bool) -> Vec<InventoryEntry> {
        let mut entries = Vec::new();
        for provider in &self.providers {
            let info = provider.info();
            for descriptor in provider.descriptors() {
                let result = if probe {
                    provider.probe(
                        descriptor.implementation(),
                        descriptor.direction(),
                        descriptor.codec(),
                    )
                } else {
                    Probe::unknown()
                };
                entries.push(InventoryEntry::new(descriptor, info.clone(), result));
            }
        }
        entries
    }

    /// Every profile any provider states for `codec` in `direction`, each with
    /// the names of the providers that state it.
    ///
    /// Profiles appear in the order they are first stated, and the names of one
    /// profile in provider registration order. A provider states its profiles
    /// through [`CodecProvider::profiles`], whose default reads its
    /// descriptors.
    #[must_use]
    pub fn profiles(
        &self,
        codec: CodecId,
        direction: Direction,
    ) -> Vec<(CodecProfile, Vec<String>)> {
        let mut attributed: Vec<(CodecProfile, Vec<String>)> = Vec::new();
        for provider in &self.providers {
            let name = provider.name();
            for profile in provider.profiles(codec, direction) {
                let index = match attributed.iter().position(|(stated, _)| *stated == profile) {
                    Some(index) => index,
                    None => {
                        attributed.push((profile, Vec::new()));
                        attributed.len() - 1
                    }
                };
                let names = &mut attributed[index].1;
                if !names.iter().any(|existing| existing == name) {
                    names.push(name.to_string());
                }
            }
        }
        attributed
    }

    /// Every candidate for `codec` in `direction`, in the order they will be
    /// tried.
    ///
    /// This reports the order alone: a preference that names something not
    /// registered is reported by [`Self::decoder`] and [`Self::encoder`], never
    /// here, and no profile clause applies because no stream is involved.
    #[must_use]
    pub fn candidates(&self, codec: CodecId, direction: Direction) -> Vec<CodecDescriptor> {
        self.resolved(codec, direction, None).kept
    }

    /// Every candidate for `parameters`, in the order they will be tried, with
    /// every candidate that states a profile list not containing the stream's
    /// profile removed.
    ///
    /// A candidate that states no profiles stays, and a stream that states no
    /// profile removes nothing. [`Self::rejected_for`] reports what was removed.
    #[must_use]
    pub fn candidates_for(&self, parameters: &CodecParameters) -> Vec<CodecDescriptor> {
        self.resolved(parameters.codec(), Direction::Decode, parameters.profile())
            .kept
    }

    /// The candidates [`Self::candidates_for`] removed from `parameters`, in
    /// their attempted order.
    #[must_use]
    pub fn rejected_for(&self, parameters: &CodecParameters) -> Vec<ProfileRejection> {
        self.resolved(parameters.codec(), Direction::Decode, parameters.profile())
            .removed
    }

    /// Every candidate for `config`, in the order they will be tried, with
    /// every candidate that states a profile list not containing the requested
    /// profile removed.
    #[must_use]
    pub fn encoder_candidates_for(&self, config: &EncoderConfig) -> Vec<CodecDescriptor> {
        self.resolved(config.codec(), Direction::Encode, config.profile())
            .kept
    }

    /// The candidates [`Self::encoder_candidates_for`] removed from `config`,
    /// in their attempted order.
    #[must_use]
    pub fn encoder_rejected_for(&self, config: &EncoderConfig) -> Vec<ProfileRejection> {
        self.resolved(config.codec(), Direction::Encode, config.profile())
            .removed
    }

    /// Build a decoder for `parameters` from the first candidate that serves
    /// it.
    ///
    /// # Errors
    /// [`CodecError::Unsupported`] when the codec has no
    /// [`Direction::Decode`] candidate, when a matching preference names a
    /// provider or implementation that is not registered, when the stream's
    /// profile rules out every candidate, when a preference or selector picked
    /// a candidate that does not state the stream's profile, or when every
    /// candidate was skipped under [`Fallback::NextCandidate`]; otherwise the
    /// candidate's own error.
    pub fn decoder(&self, parameters: &CodecParameters) -> Result<Box<dyn Decoder>, CodecError> {
        self.decoder_with(parameters).map(|(decoder, _)| decoder)
    }

    /// Build a decoder for `parameters` and apply the container's gapless trim
    /// unless the selected implementation says it applies the trim itself.
    ///
    /// The wrapper returns the decoder's frames with the initial and trailing
    /// padding removed (see [`TrimStage`](crate::TrimStage)); an implementation
    /// whose [`CodecDescriptor::applies_trim`] is true is returned unchanged.
    ///
    /// # Errors
    /// As [`Self::decoder`].
    pub fn trimmed_decoder(
        &self,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        let (decoder, descriptor) = self.decoder_with(parameters)?;
        if descriptor.applies_trim() {
            return Ok(decoder);
        }
        Ok(Box::new(TrimmedDecoder::new(decoder, parameters)))
    }

    fn decoder_with(
        &self,
        parameters: &CodecParameters,
    ) -> Result<(Box<dyn Decoder>, CodecDescriptor), CodecError> {
        let codec = parameters.codec();
        let direction = Direction::Decode;
        let candidates = self.checked(codec, direction, parameters.profile())?;
        self.build(&candidates, codec, direction, |provider, implementation| {
            provider.make_decoder(implementation, parameters)
        })
    }

    /// Build an encoder from `config`.
    ///
    /// # Errors
    /// As [`Self::decoder`], for [`Direction::Encode`], with the profile the
    /// configuration asks for in place of the stream's.
    pub fn encoder(&self, config: &EncoderConfig) -> Result<Box<dyn Encoder>, CodecError> {
        let codec = config.codec();
        let direction = Direction::Encode;
        let candidates = self.checked(codec, direction, config.profile())?;
        self.build(&candidates, codec, direction, |provider, implementation| {
            provider.make_encoder(implementation, config)
        })
        .map(|(encoder, _)| encoder)
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

    /// The preference governing `codec`, `direction` and `profile`: a
    /// preference naming the profile outranks one for the bare codec, and
    /// within one profile scope the one naming the direction outranks the one
    /// naming both.
    fn preference_for(
        &self,
        codec: CodecId,
        direction: Direction,
        profile: Option<CodecProfile>,
    ) -> Option<&Preference> {
        let find = |wanted_profile: Option<CodecProfile>, wanted_direction: Option<Direction>| {
            self.preferences.iter().find(|preference| {
                preference.codec() == codec
                    && preference.profile() == wanted_profile
                    && preference.direction() == wanted_direction
            })
        };
        if let Some(profile) = profile
            && let Some(preference) =
                find(Some(profile), Some(direction)).or_else(|| find(Some(profile), None))
        {
            return Some(preference);
        }
        find(None, Some(direction)).or_else(|| find(None, None))
    }

    /// The index a selector answers with, when it is in range.
    fn selector_index(&self, candidates: &[CodecDescriptor]) -> Option<usize> {
        let selector = self.selector.as_ref()?;
        selector(candidates).filter(|index| *index < candidates.len())
    }

    fn arrange(
        &self,
        mut candidates: Vec<CodecDescriptor>,
        picked: Option<usize>,
        preference: Option<&Preference>,
    ) -> Vec<CodecDescriptor> {
        if candidates.is_empty() {
            return candidates;
        }
        if let Some(index) = picked {
            candidates[..=index].rotate_right(1);
            return candidates;
        }
        let Some(preference) = preference else {
            return candidates;
        };
        let (front, rest): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|descriptor| matches_preference(descriptor, preference));
        let mut ordered = front;
        ordered.extend(rest);
        ordered
    }

    /// The candidates for a request, ordered and then filtered by its profile.
    fn resolved(
        &self,
        codec: CodecId,
        direction: Direction,
        profile: Option<CodecProfile>,
    ) -> Resolution {
        self.resolve(
            self.registered(codec, direction),
            self.preference_for(codec, direction, profile),
            profile,
        )
    }

    fn resolve(
        &self,
        registered: Vec<CodecDescriptor>,
        preference: Option<&Preference>,
        profile: Option<CodecProfile>,
    ) -> Resolution {
        let picked = self.selector_index(&registered);
        let chosen = picked
            .map(|index| (&registered[index], PickedBy::Selector))
            .or_else(|| {
                preference.and_then(|preference| {
                    registered
                        .iter()
                        .find(|descriptor| matches_preference(descriptor, preference))
                        .map(|descriptor| {
                            (
                                descriptor,
                                PickedBy::Preference(preference.provider().to_string()),
                            )
                        })
                })
            });
        let rejected_pick = match (chosen, profile) {
            (Some((descriptor, by)), Some(profile)) if !handles(descriptor, profile) => {
                Some(RejectedPick {
                    by,
                    provider: descriptor.provider().to_string(),
                    implementation: descriptor.implementation().to_string(),
                })
            }
            _ => None,
        };
        let (kept, removed) = split(self.arrange(registered, picked, preference), profile);
        Resolution {
            kept,
            removed,
            rejected_pick,
        }
    }

    fn checked(
        &self,
        codec: CodecId,
        direction: Direction,
        profile: Option<CodecProfile>,
    ) -> Result<Vec<CodecDescriptor>, CodecError> {
        let registered = self.registered(codec, direction);
        if registered.is_empty() {
            return Err(CodecError::unsupported(format!(
                "{} of {}",
                direction.name(),
                codec.name()
            )));
        }
        let preference = self.preference_for(codec, direction, profile);
        if let Some(preference) = preference {
            self.check_registered(preference, &registered, codec, direction)?;
        }
        let resolution = self.resolve(registered, preference, profile);
        let Some(profile) = profile else {
            return Ok(resolution.kept);
        };
        let what = format!(
            "{} of {} at profile {}",
            direction.name(),
            codec.name(),
            profile.token()
        );
        if let Some(pick) = resolution.rejected_pick {
            return Err(CodecError::unsupported(format!(
                "{what}: {}",
                pick.reason()
            )));
        }
        if resolution.kept.is_empty() {
            let removed: Vec<String> = resolution.removed.iter().map(ToString::to_string).collect();
            return Err(CodecError::unsupported(format!(
                "{what}: no candidate handles it; removed {}",
                removed.join(", ")
            )));
        }
        Ok(resolution.kept)
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
    ) -> Result<(T, CodecDescriptor), CodecError> {
        let mut tried: Vec<String> = Vec::new();
        for descriptor in candidates {
            let Some(provider) = self.provider(descriptor.provider()) else {
                return Err(CodecError::unsupported(format!(
                    "provider `{}` is not registered",
                    descriptor.provider()
                )));
            };
            match make(provider, descriptor.implementation()) {
                Ok(built) => return Ok((built, descriptor.clone())),
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

/// The candidates one request resolves to, and what the profile clause removed.
struct Resolution {
    kept: Vec<CodecDescriptor>,
    removed: Vec<ProfileRejection>,
    rejected_pick: Option<RejectedPick>,
}

/// Who picked a candidate the profile clause then removed.
enum PickedBy {
    Selector,
    Preference(String),
}

/// The candidate a selector or a preference picked, and that does not state the
/// stream's profile.
struct RejectedPick {
    by: PickedBy,
    provider: String,
    implementation: String,
}

impl RejectedPick {
    /// Why the pick was refused, naming the pick and who made it.
    fn reason(&self) -> String {
        let candidate = format!("{}:{}", self.provider, self.implementation);
        match &self.by {
            PickedBy::Selector => {
                format!("the selector chose `{candidate}`, which does not state the profile")
            }
            PickedBy::Preference(provider) => format!(
                "preference `{provider}` names `{candidate}`, which does not state the profile"
            ),
        }
    }
}

/// The candidates that handle `profile`, and the ones removed for not stating
/// it.
fn split(
    candidates: Vec<CodecDescriptor>,
    profile: Option<CodecProfile>,
) -> (Vec<CodecDescriptor>, Vec<ProfileRejection>) {
    let Some(profile) = profile else {
        return (candidates, Vec::new());
    };
    let (kept, removed): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|descriptor| handles(descriptor, profile));
    let removed = removed
        .into_iter()
        .map(|descriptor| ProfileRejection::new(descriptor, profile))
        .collect();
    (kept, removed)
}

/// Whether `descriptor` may take a stream in `profile`: it states no profiles,
/// or it states this one.
fn handles(descriptor: &CodecDescriptor, profile: CodecProfile) -> bool {
    descriptor.profiles().is_empty() || descriptor.profiles().contains(&profile)
}

//! A choice for one codec and direction, and its text form.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use crate::codec::{CodecId, OTHER_TOKEN, same_token, token};
use crate::direction::Direction;
use crate::preference_error::PreferenceError;
use crate::profile::{CodecProfile, TOKEN_SEPARATOR};
use crate::profile_error::ProfileError;
use crate::provider::offending_character;

/// Separates a codec from its provider, and a provider from its
/// implementation.
const SCOPE: char = ':';
/// Separates one preference from the next in a list.
const LIST_SEPARATOR: char = ',';
/// Separates the codec token from the provider.
const ASSIGNMENT: char = '=';

/// A choice of provider for one codec and direction.
///
/// A preference built with [`Preference::new`] applies to both directions; one
/// built with [`Self::with_direction`] applies to that direction only, and is
/// what the `dec:`/`enc:` text prefix produces. See
/// [`Registry::prefer`](crate::Registry::prefer) for how a registry uses it.
///
/// A preference for [`CodecId::Other`] spells its tag: `other:00000007=alpha`.
/// `other` alone names no codec and is [`PreferenceError::UnknownCodec`].
///
/// A preference built with [`Self::with_profile`] applies only to streams of
/// that profile (`h264/high10=libav`), and outranks a preference for the bare
/// codec when a stream of that profile is served.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preference {
    codec: CodecId,
    profile: Option<CodecProfile>,
    direction: Option<Direction>,
    provider: String,
    implementation: Option<String>,
}

impl Preference {
    /// A preference for `codec` in both directions, served by `provider`.
    #[must_use]
    pub fn new(codec: CodecId, provider: impl Into<String>) -> Self {
        Self {
            codec,
            profile: None,
            direction: None,
            provider: provider.into(),
            implementation: None,
        }
    }

    /// Narrow the preference to streams of `profile`.
    ///
    /// # Errors
    /// [`ProfileError::Mismatch`] when the profile belongs to another codec.
    pub fn with_profile(mut self, profile: CodecProfile) -> Result<Self, ProfileError> {
        if profile.codec() != self.codec {
            return Err(ProfileError::mismatch(profile, self.codec));
        }
        self.profile = Some(profile);
        Ok(self)
    }

    /// Narrow the preference to one direction.
    #[must_use]
    pub const fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Narrow the preference to one of the provider's implementations.
    #[must_use]
    pub fn with_implementation(mut self, implementation: impl Into<String>) -> Self {
        self.implementation = Some(implementation.into());
        self
    }

    /// Coded format the preference is about.
    #[must_use]
    pub const fn codec(&self) -> CodecId {
        self.codec
    }

    /// The one profile the preference is about, or `None` for every profile.
    #[must_use]
    pub const fn profile(&self) -> Option<CodecProfile> {
        self.profile
    }

    /// The one direction the preference is about, or `None` for both.
    #[must_use]
    pub const fn direction(&self) -> Option<Direction> {
        self.direction
    }

    /// Name of the preferred provider.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The provider implementation preferred, when one is named.
    #[must_use]
    pub fn implementation(&self) -> Option<&str> {
        self.implementation.as_deref()
    }
}

impl fmt::Display for Preference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(direction) = self.direction {
            write!(f, "{}:", direction.prefix())?;
        }
        match self.profile {
            Some(profile) => write!(f, "{}", profile.token())?,
            None => write!(f, "{}", token(self.codec))?,
        }
        write!(f, "={}", self.provider)?;
        if let Some(implementation) = &self.implementation {
            write!(f, ":{implementation}")?;
        }
        Ok(())
    }
}

impl FromStr for Preference {
    type Err = PreferenceError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (codec_side, provider_side) = text
            .split_once(ASSIGNMENT)
            .ok_or_else(|| PreferenceError::missing_equals(text))?;
        let (direction, codec_token) = match codec_side.split_once(SCOPE) {
            Some((prefix, rest)) => match Direction::from_prefix(prefix) {
                Some(direction) => (Some(direction), rest),
                None if same_token(prefix, OTHER_TOKEN) => (None, codec_side),
                None => return Err(PreferenceError::unknown_direction(prefix)),
            },
            None => (None, codec_side),
        };
        let (codec_token, profile_token) = match codec_token.split_once(TOKEN_SEPARATOR) {
            Some((codec, profile)) => (codec, Some(profile)),
            None => (codec_token, None),
        };
        let codec = parse_codec(codec_token)?;
        let profile = match profile_token {
            Some(token) => Some(parse_profile(codec, token)?),
            None => None,
        };
        let (provider, implementation) = match provider_side.split_once(SCOPE) {
            Some((provider, implementation)) => (provider, Some(implementation)),
            None => (provider_side, None),
        };
        check_name(provider, PreferenceError::EmptyProvider)?;
        let implementation = match implementation {
            Some(implementation) => {
                check_name(implementation, PreferenceError::EmptyImplementation)?;
                Some(implementation.into())
            }
            None => None,
        };
        let mut preference = Self::new(codec, provider);
        preference.profile = profile;
        preference.direction = direction;
        preference.implementation = implementation;
        Ok(preference)
    }
}

/// Parse a comma-separated list of preferences: `aac=oxideav,enc:h264=libav`.
///
/// Surrounding whitespace around an entry is ignored; a blank entry between
/// two separators is a [`PreferenceError::EmptyEntry`]. An empty string is an
/// empty list.
///
/// # Errors
/// Whatever the first entry that does not [`parse`](Preference::from_str)
/// reports.
pub fn parse_preferences(text: &str) -> Result<Vec<Preference>, PreferenceError> {
    let mut preferences = Vec::new();
    if text.trim().is_empty() {
        return Ok(preferences);
    }
    for (entry, piece) in text.split(LIST_SEPARATOR).enumerate() {
        let piece = piece.trim();
        if piece.is_empty() {
            return Err(PreferenceError::EmptyEntry { entry });
        }
        preferences.push(piece.parse()?);
    }
    Ok(preferences)
}

/// Whether a character may appear in a codec's text token.
fn codec_token_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ')
}

/// The codec a token names, refusing a stray character first so the error says
/// which character is wrong rather than "unknown codec".
fn parse_codec(token: &str) -> Result<CodecId, PreferenceError> {
    let other = token
        .split_once(SCOPE)
        .is_some_and(|(head, _)| same_token(head, OTHER_TOKEN));
    if !other && let Some(character) = token.chars().find(|c| !codec_token_character(*c)) {
        return Err(PreferenceError::invalid_character(token, character));
    }
    CodecId::from_name(token).ok_or_else(|| PreferenceError::unknown_codec(token))
}

/// The profile a token names for `codec`, refusing a stray character first so
/// the error says which character is wrong rather than "unknown profile".
fn parse_profile(codec: CodecId, token: &str) -> Result<CodecProfile, PreferenceError> {
    if let Some(character) = token.chars().find(|c| !codec_token_character(*c)) {
        return Err(PreferenceError::invalid_character(token, character));
    }
    CodecProfile::from_parts(codec, token)
        .ok_or_else(|| PreferenceError::unknown_profile(codec, token))
}

/// Refuse a name part that is empty or breaks the provider-name rule.
fn check_name(token: &str, empty: PreferenceError) -> Result<(), PreferenceError> {
    if token.is_empty() {
        return Err(empty);
    }
    match offending_character(token) {
        Some(character) => Err(PreferenceError::invalid_character(token, character)),
        None => Ok(()),
    }
}

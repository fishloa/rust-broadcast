//! Why a registry refused a provider.

use alloc::string::String;
use core::fmt;

/// Why a [`Registry`](crate::Registry) refused a provider.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A provider with this name is already registered.
    DuplicateName {
        /// The name both providers use.
        name: String,
    },
    /// A provider name breaks the naming rule
    /// [`CodecProvider::name`](crate::CodecProvider::name) states.
    InvalidName {
        /// The name the provider reported.
        name: String,
        /// The character that breaks the rule, or `None` when the name is
        /// empty.
        character: Option<char>,
    },
}

impl RegistryError {
    pub(crate) fn duplicate_name(name: &str) -> Self {
        Self::DuplicateName { name: name.into() }
    }

    pub(crate) fn invalid_name(name: &str, character: Option<char>) -> Self {
        Self::InvalidName {
            name: name.into(),
            character,
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName { name } => {
                write!(f, "provider `{name}` is already registered")
            }
            Self::InvalidName { name, character } => match character {
                Some(character) => write!(
                    f,
                    "provider name `{name}` contains `{character}`; use lower-case \
                     ASCII letters, digits, `-` and `_`"
                ),
                None => f.write_str("provider name is empty"),
            },
        }
    }
}

impl core::error::Error for RegistryError {}

//! Whether an implementation opens on this machine.

use alloc::string::String;

/// Whether a listed implementation opens here.
///
/// Listed means compiled in; available means it opens on this machine. A
/// hardware codec may be listed and unavailable.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Availability {
    /// The provider was not asked, or cannot tell.
    Unknown,
    /// The implementation opened here.
    Available,
    /// The implementation did not open here.
    Unavailable,
}

impl Availability {
    /// Label of the availability.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Available => "available",
            Self::Unavailable => "unavailable",
        }
    }
}

broadcast_core::impl_spec_display!(Availability);

/// One availability answer, with the provider's reason when it is unavailable.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Probe {
    availability: Availability,
    reason: Option<String>,
}

impl Probe {
    /// Not asked, or no answer.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            availability: Availability::Unknown,
            reason: None,
        }
    }

    /// It opens here.
    #[must_use]
    pub const fn available() -> Self {
        Self {
            availability: Availability::Available,
            reason: None,
        }
    }

    /// It does not open here, for `reason`, in the provider's words.
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            availability: Availability::Unavailable,
            reason: Some(reason.into()),
        }
    }

    /// The answer.
    #[must_use]
    pub const fn availability(&self) -> Availability {
        self.availability
    }

    /// Why it is unavailable, in the provider's words.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

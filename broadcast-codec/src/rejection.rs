//! One candidate the profile clause removed, and why.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::descriptor::CodecDescriptor;
use crate::profile::CodecProfile;

/// One candidate the registry removed from a stream's candidates because the
/// candidate states profiles and the stream's profile is not among them.
///
/// [`Registry::rejected_for`](crate::Registry::rejected_for) and its encoder
/// twin report these, so a caller can see what was removed and why instead of
/// only being told that nothing was left.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileRejection {
    descriptor: CodecDescriptor,
    profile: CodecProfile,
}

impl ProfileRejection {
    /// A rejection of `descriptor` for a stream in `profile`.
    #[must_use]
    pub const fn new(descriptor: CodecDescriptor, profile: CodecProfile) -> Self {
        Self {
            descriptor,
            profile,
        }
    }

    /// The candidate that was removed.
    #[must_use]
    pub const fn descriptor(&self) -> &CodecDescriptor {
        &self.descriptor
    }

    /// The stream profile the candidate does not state.
    #[must_use]
    pub const fn profile(&self) -> CodecProfile {
        self.profile
    }
}

impl fmt::Display for ProfileRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stated: Vec<String> = self
            .descriptor
            .profiles()
            .iter()
            .map(CodecProfile::token)
            .collect();
        write!(
            f,
            "{}:{} (states {})",
            self.descriptor.provider(),
            self.descriptor.implementation(),
            stated.join(", ")
        )
    }
}

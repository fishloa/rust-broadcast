//! Who provides an implementation, and under what terms.

use alloc::string::String;

/// One provider, described by the provider itself.
///
/// The application's inventory reports these, so whoever ships the binary can
/// see which adapter and which library a build linked, and under what terms.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderInfo {
    name: String,
    version: String,
    library: Option<String>,
    licence: Option<String>,
    notes: Option<String>,
}

impl ProviderInfo {
    /// Information for the provider called `name`, at the adapter's own
    /// `version`, with nothing known about the underlying library.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            library: None,
            licence: None,
            notes: None,
        }
    }

    /// State the underlying library and its version, for example
    /// `libavcodec 62.28.102`.
    #[must_use]
    pub fn with_library(mut self, library: impl Into<String>) -> Self {
        self.library = Some(library.into());
        self
    }

    /// State the terms of the linked library.
    ///
    /// This is information for whoever distributes the binary, not legal
    /// advice: an adapter must report what the library itself reports or what
    /// its manifest states, for example `LGPL version 2.1 or later` or
    /// `MPL-2.0`, and must not guess. `None` means the adapter cannot know.
    #[must_use]
    pub fn with_licence(mut self, licence: impl Into<String>) -> Self {
        self.licence = Some(licence.into());
        self
    }

    /// Add free text, such as a build configuration summary.
    #[must_use]
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// The provider's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The adapter's own version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The underlying library and its version, when the adapter knows them.
    #[must_use]
    pub fn library(&self) -> Option<&str> {
        self.library.as_deref()
    }

    /// The linked library's terms, when the adapter knows them.
    #[must_use]
    pub fn licence(&self) -> Option<&str> {
        self.licence.as_deref()
    }

    /// Free text about the build, when the adapter states any.
    #[must_use]
    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
    }
}

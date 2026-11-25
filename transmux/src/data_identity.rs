//! Carry an opaque data track across container families (Decision 7).
//!
//! A [`CodecConfig::Data`](crate::ir::CodecConfig::Data) track carries the
//! identity of the container it was demuxed from: a TS elementary-stream
//! `stream_type` and descriptor loop
//! ([`DataIdentity::Ts`](crate::ir::DataIdentity::Ts)), an ISOBMFF handler and
//! sample entry ([`DataIdentity::Mp4`](crate::ir::DataIdentity::Mp4)), or a
//! Matroska `CodecID` and `CodecPrivate`
//! ([`DataIdentity::Matroska`](crate::ir::DataIdentity::Matroska)). Each muxer
//! only places its own family's identity, so carrying a track to another
//! container needs the caller to say what the target identity is.
//!
//! [`DataIdentityMap`] is that caller-built table: a source [`TrackId`] and the
//! [`DataIdentity`] the track should carry in the output.
//! [`DataIdentityStage`] applies it to a [`MuxInput::Track`] announcement and
//! passes every other input through untouched; sample bytes are opaque and
//! are never altered. There is no library-chosen default mapping: an unmapped
//! data track reaches the muxer with its own identity, which the muxer then
//! accepts or refuses by family.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};

use broadcast_common::stage::{Stage, Timestamp};

use crate::error::Result;
use crate::ir::{CodecConfig, DataIdentity, TrackId, TrackSpec};
use crate::mux_stage::MuxInput;

/// A caller-built mapping from a source track to the [`DataIdentity`] it
/// should carry in the output, for carrying an opaque data track across
/// container families.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataIdentityMap {
    identities: BTreeMap<TrackId, DataIdentity>,
}

impl DataIdentityMap {
    /// An empty map: every data track keeps its own identity.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Map `track` to `identity` in the output, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, track: TrackId, identity: DataIdentity) -> Self {
        self.insert(track, identity);
        self
    }

    /// Map `track` to `identity` in the output.
    pub fn insert(&mut self, track: TrackId, identity: DataIdentity) {
        self.identities.insert(track, identity);
    }

    /// The identity `track` is mapped to, if any.
    #[must_use]
    pub fn get(&self, track: TrackId) -> Option<&DataIdentity> {
        self.identities.get(&track)
    }

    /// Whether no track is mapped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }
}

/// Rewrites the [`DataIdentity`] of a mapped data track's announcement to the
/// target identity and passes every other [`MuxInput`] through unchanged.
///
/// A [`MuxInput::Track`] whose track is listed in the [`DataIdentityMap`] and
/// whose config is a [`CodecConfig::Data`] track is announced with the mapped
/// identity. Samples are opaque bytes and are never altered. A track the map
/// does not name keeps its own identity.
#[derive(Debug, Clone)]
pub struct DataIdentityStage {
    map: DataIdentityMap,
    ready: VecDeque<MuxInput>,
}

impl DataIdentityStage {
    /// Apply `map` to every announcement this stage forwards.
    #[must_use]
    pub fn new(map: DataIdentityMap) -> Self {
        Self {
            map,
            ready: VecDeque::new(),
        }
    }

    /// The map this stage applies.
    #[must_use]
    pub fn map(&self) -> &DataIdentityMap {
        &self.map
    }

    fn rewrite(&self, mut spec: TrackSpec) -> MuxInput {
        if matches!(spec.config, CodecConfig::Data { .. })
            && let Some(identity) = self.map.get(spec.track_id)
        {
            spec.config = CodecConfig::Data {
                identity: identity.clone(),
            };
        }
        MuxInput::Track(Box::new(spec))
    }
}

impl Stage for DataIdentityStage {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = crate::error::Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => self.ready.push_back(self.rewrite(*spec)),
            other => self.ready.push_back(other),
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

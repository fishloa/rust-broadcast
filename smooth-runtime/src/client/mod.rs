//! The Smooth client engine: [`SmoothClient`] and the types it exchanges with a host.

mod action;
mod config;
mod engine;
mod output;
mod stream;

pub use action::{Action, FetchFailure, RequestId, StreamId};
pub use config::{QualitySelector, SmoothClientConfig};
pub use engine::SmoothClient;
pub use output::{Dropped, ManifestFault, Output, QualityInfo};

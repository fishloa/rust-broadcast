//! WHEP player (viewer/consumer) state machine: the [`WhepPlayer`] alias of
//! the shared [`crate::session::client::Client`] engine.

pub use crate::session::client::{Event, HttpRequest, HttpResponse, Method, State, WhepPlayer};

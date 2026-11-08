//! WHIP client (encoder/ingester) state machine: the [`WhipClient`] alias of
//! the shared [`crate::session::client::Client`] engine.

pub use crate::session::client::{Event, HttpRequest, HttpResponse, Method, State, WhipClient};

//! ICE + DTLS-SRTP media transport — the piece [`crate::whip`]/[`crate::whep`]
//! signalling hands off to once SDP has been exchanged.
//!
//! `whip`/`whep` only ever move opaque SDP/Trickle-ICE bytes over HTTP; they
//! have no socket, no ICE agent, no DTLS, no SRTP (see the crate README).
//! This module is the part that actually sends and receives media: it
//! terminates ICE connectivity checks (host + server-reflexive candidates —
//! TURN relay is out of scope for this cut), performs the DTLS handshake and
//! exports SRTP keying material per [RFC 5764] ("DTLS-SRTP"), and decrypts
//! inbound SRTP into RTP/RTCP packets typed by this workspace's own
//! [`rtp_packet`]/[`rtcp_packet`] crates — never `rtc-rtp`/`rtc-rtcp`.
//!
//! Still sans-IO where the underlying `rtc-ice`/`rtc-dtls` crates are:
//! [`MediaTransport`] owns no socket. The caller feeds it inbound datagrams
//! ([`MediaTransport::handle_datagram`]), drains outbound ones
//! ([`MediaTransport::poll_transmit`]), and drives its timers
//! ([`MediaTransport::handle_timeout`]) — the same push/pull shape as
//! [`crate::whip::client::WhipClient`], just for UDP instead of HTTP.
//!
//! SDP itself is out of scope here, exactly as it is in `whip`/`whep`: the
//! caller extracts `a=ice-ufrag`/`a=ice-pwd`/`a=fingerprint`/`a=candidate`
//! from the negotiated SDP and passes the values in through
//! [`MediaTransportConfig`]; [`MediaTransport::local_fingerprint`] and the
//! candidates gathered via [`MediaEvent::LocalCandidateGathered`] are what
//! the caller feeds back into its own SDP answer / Trickle-ICE fragment.
//!
//! # Demultiplexing a single UDP flow
//!
//! ICE (STUN), DTLS, and SRTP/SRTCP all arrive on the same local port. Per
//! [RFC 5764] §5.1.2 ("Reception"): a first byte of 0 or 1 is STUN, 20–63 is
//! DTLS, and 128–191 is RTP or RTCP. Once inside the SRTP/SRTCP band, [RFC
//! 5761] §4 further separates RTP from RTCP: it reserves RTCP packet types
//! in `[192, 223]` and requires senders to avoid RTP payload types `[64,
//! 95]` (whose value, with the RTP marker bit set, aliases into that same
//! `[192, 223]` band) specifically so that band unambiguously identifies
//! RTCP once `a=rtcp-mux` is negotiated. [`MediaTransport::handle_datagram`]
//! applies both rules via the named constants in this module.
//!
//! # MSRV
//!
//! This module (feature `media`) requires **rustc >= 1.88**. The rest of
//! the crate — the WHIP/WHEP signalling state machines — keeps the crate's
//! declared `rust-version = "1.86"`. The split exists because `rtc-dtls`
//! depends on `rcgen ^0.14.8`, which needs 1.88; enabling `media` on 1.86
//! fails with an opaque "package requires a newer version of Rust" error
//! from that transitive dependency, not from this crate, so it is
//! documented here and in the README rather than left to be discovered.
//!
//! [RFC 5764]: https://www.rfc-editor.org/rfc/rfc5764
//! [RFC 5761]: https://www.rfc-editor.org/rfc/rfc5761

mod gather;
mod transport;

pub use transport::{
    Datagram, DecryptedRtp, MediaEvent, MediaTransport, MediaTransportConfig, SetupRole,
};

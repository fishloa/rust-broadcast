mod arq_recovery;
mod crypto_handshake;
mod crypto_vectors;
mod error_variants;
mod filecc_congestion;
mod handshake_round_trip;
mod io_concurrent_listener;
mod io_config;
mod io_forwarder_abort_on_drop;
mod io_handshake;
mod io_isn_socket_id_randomization;
mod io_listener_lifecycle;
mod io_loopback;
mod io_loss_recovery;
mod io_rendezvous;
mod km_refresh;
mod libsrt_crypto_interop;
mod libsrt_fixtures;
mod libsrt_interop;
mod libsrt_loss;
mod libsrt_media;
mod libsrt_rendezvous;
mod livecc_pacing;
mod no_panic;
mod pacing_throughput;
mod rendezvous_round_trip;
mod rfc_vectors;
mod tokio_crypto_interop;
mod trait_round_trip;
mod tsbpd_delivery;

/// A default handshake that does not advertise TLPKTDROP, for tests whose
/// contract is that every loss is recovered by retransmission. Advertising it
/// licenses a receiver to skip a gap once its latency passes and, per libsrt
/// 1.5.5 `CUDT::sndDropTooLate` (gated on the *peer's* flag), a libsrt sender
/// to drop what it has buffered for over max(latency, 1000 ms) + 20 ms and
/// answer the NAK with a DROPREQ — both legitimate outcomes a host stall can
/// trigger, neither of which those tests are about.
#[cfg(feature = "tokio")]
fn without_tlpktdrop() -> srt_runtime::handshake_sm::HandshakeConfig {
    use srt_runtime::packet::HandshakeExtensionMessageFlags;
    use srt_runtime::packet::handshake::HS_MSG_FLAG_TLPKTDROP;
    let config = srt_runtime::handshake_sm::HandshakeConfig::default();
    srt_runtime::handshake_sm::HandshakeConfig {
        flags: HandshakeExtensionMessageFlags(config.flags.0 & !HS_MSG_FLAG_TLPKTDROP),
        ..config
    }
}

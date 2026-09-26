# rtcp-packet fixtures

## `chrome_rr_pli_remb.bin` (68 bytes)

A real, plaintext RTCP compound packet sent by a real Chromium
`RTCPeerConnection` (WHEP video viewer) to `multimux`, captured 2026-09-26
during a run of `multimux`'s own real-browser interop test
(`cargo test -p multimux --all-features --locked --test whep_egress --
--nocapture`, see `multimux/tests/whep_egress.rs`) — not synthetic, not
generated from a tool. Captured by a temporary one-line `eprintln!` of the
decrypted SRTCP plaintext in `webrtc-runtime/src/media/transport.rs`
(`decrypt_srtp`), removed again immediately after (see PR history for
issue #1071). Released under the workspace licence.

Wire layout (RFC 3550 §6 / RFC 4585 / the REMB draft
`draft-alvestrand-rmcat-remb-03`):

1. **RR** (RFC 3550 §6.4.2, PT 201), bytes `0..32`: reporter SSRC=1, one
   report block for SSRC `0x407E522F` (fraction_lost=0, cumulative_lost=0,
   ext_highest_seq=5146, jitter=482, lsr=0, dlsr=0).
2. **PSFB PLI** (RFC 4585 §6.3.1, PT 206 FMT 1), bytes `32..44`: sender
   SSRC=1, media source SSRC `0x407E522F`, no FCI.
3. **PSFB REMB** (`draft-alvestrand-rmcat-remb-03`, PT 206 FMT 15 —
   unregistered with IANA, not decoded by `rtcp-packet`, hence
   `RtcpPacket::Unknown`), bytes `44..68`: sender SSRC=1, media source
   SSRC=0, ASCII `"REMB"`, then the bit-rate/SSRC-list FCI opaque to this
   crate.

This is exactly the shape issue #1071 (r14-RTCP-C1) fixed:
`CompoundPacket::parse` used to reject the **whole** 68-byte datagram
because it walked into the PSFB packets and rejected their PT (206) as
"not an RFC 3550 §6 core packet type", discarding the leading RR along
with them. It now parses as one `CompoundPacket` of three packets: a typed
`RtcpPacket::ReceiverReport`, and two `RtcpPacket::Unknown { packet_type:
206, .. }` (this crate does not decode PSFB FCI — see the crate's own
`docs/rtcp.md` decode-completeness notes). See
`tests/real_browser_fixture.rs` for the parse + byte-identical round-trip
assertions.

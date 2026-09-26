# SRT wire fixtures

Real `libsrt` control-packet captures used by `tests/libsrt_fixtures.rs`
(issue #1060, the 4-byte zero-pad libsrt appends to CIF-less control types).

## `libsrt_keepalive.bin`, `libsrt_ackack.bin`

- **Tool:** `srt-live-transmit` (Homebrew `srt` formula)
- **libsrt version:** 1.5.5 (`srt-live-transmit -version` ->
  `SRT Library version: 1.5.5, clock type: MACH_ABSTIME`)
- **Command:** two local `srt-live-transmit` processes over loopback —
  a listener (`srt://:PORT` -> `udp://127.0.0.1:DISCARD_PORT`) and a caller
  (`file://con` -> `srt://127.0.0.1:PORT`), the caller fed synthetic
  188-byte MPEG-TS-shaped chunks (`bytes([0x47] + [0]*1315)`, sent as
  1316-byte writes) over its stdin via a Python `subprocess.Popen`, then left
  idle so the connection's own KEEPALIVE/ACKACK traffic was captured.
- **Capture:** raw UDP payloads sniffed on `lo0` with `scapy` (`sniff(iface="lo0",
  filter="udp port PORT")`), filtered to the SRT control channel by port, then
  the first observed KEEPALIVE (Control Type `0x0001`) and ACKACK (`0x0006`)
  payloads were saved verbatim as these two files. Every packet on the wire
  during that idle window round-tripped to the same 20-byte shape: a 16-byte
  SRT header (per `draft-sharabayko-srt-01` §3, Figure 2) followed by 4 zero
  bytes, matching libsrt's own `srtcore/packet.cpp` `CPacket::pack` (each of
  the `UMSG_KEEPALIVE`/`UMSG_ACKACK`/`UMSG_CGWARNING`/`UMSG_SHUTDOWN`/
  `UMSG_PEERERROR` arms calls
  `m_PacketVector[PV_DATA].set((void*)&m_extra_pad, 4)` — its own comment:
  "control info field should be none but writev does not allow this";
  `m_extra_pad` is a zero-initialized member, so the pad is always zero on
  the wire). Congestion Warning/Shutdown/Peer Error share that exact code
  path (same function, same 4-byte zero-pad write, only the `Control Type`
  differs) so they were not captured separately.

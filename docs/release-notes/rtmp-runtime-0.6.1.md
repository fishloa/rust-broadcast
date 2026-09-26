# rtmp-runtime 0.6.1

Security patch for RTMP chunk reassembly. **Upgrade if you accept RTMP from publishers you do not
control.** Drop-in for 0.6.x.

## Security

**GHSA-fjrp-rx2c-c9pw** — before this release, `ChunkAssembler` copied the whole message
received so far each time a chunk arrived, so reassembly cost grew with the square of the
message length:
- A peer could announce a chunk size of 1 before `connect` and send one large message, burning
  hours of CPU.
- Encoders at the default 128-byte chunk size paid about 0.5 s of CPU per 1 MiB keyframe.

## Changes

- Continuation chunks are appended to the message in place, and consumed input is compacted once
  per call. Reassembly is now linear in the message size at any chunk size. A regression test
  reassembles an 8 MiB message at chunk size 1 and bounds the bytes copied.
- Peer-announced chunk sizes from 1 up to the maximum are honoured exactly. A chunk size of 0,
  or one with the reserved bit set, is still rejected when the Set Chunk Size message is parsed.

MSRV 1.95.0.

# rtmp-runtime 0.6.1

Security patch for RTMP chunk reassembly and the `publish` stream-key check. **Upgrade if you
accept RTMP from publishers you do not control.** Drop-in for 0.6.x.

## Security

| Advisory | Before this release |
|---|---|
| GHSA-fjrp-rx2c-c9pw | `ChunkAssembler` copied the whole message received so far each time a chunk arrived, so reassembly cost grew with the square of the message length. A peer could announce a chunk size of 1 before `connect` and send one large message, burning hours of CPU; encoders at the default 128-byte chunk size paid about 0.5 s of CPU per 1 MiB keyframe. |
| GHSA-hgmf-qpx9-6gg2 | `publish` compared the offered stream key with a plain `!=`, which returns as soon as the first differing byte is found — timing a mismatch can leak the correct key one byte at a time. There was also no limit on failed `publish` attempts, so a connection could be retried against indefinitely. |

## Changes

- Continuation chunks are appended to the message in place, and consumed input is compacted once
  per call. Reassembly is now linear in the message size at any chunk size. A regression test
  reassembles an 8 MiB message at chunk size 1 and bounds the bytes copied.
- Peer-announced chunk sizes from 1 up to the maximum are honoured exactly. A chunk size of 0,
  or one with the reserved bit set, is still rejected when the Set Chunk Size message is parsed.
- `publish`'s stream-key check now runs in constant time, so a mismatch cannot be distinguished
  by comparison timing.
- A connection is closed after `MAX_FAILED_PUBLISH_ATTEMPTS` (3) `publish` attempts with a
  mismatched stream key, instead of allowing unlimited retries on the same connection.

MSRV 1.95.0.

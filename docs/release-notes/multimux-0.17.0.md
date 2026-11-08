# multimux 0.17.0

_Released 2026-11-08._

Breaking-class release (0.16 -> 0.17): two error types lose never-constructed variants, and the sibling epochs of `webrtc-runtime`, `rtsp-runtime` and `broadcast-client` move. Public constructors and builders are unchanged.

Version: 0.16.0 -> 0.17.0.

## Breaking changes and migration

What the CHANGELOG records as breaking (#1726):

- `MultimuxError::Depay`, never constructed: a malformed RTP datagram is dropped without failing the route, and demux and depayload failures keep their `transmux` source.
- `dashif_ingest::IngestError::Auth`, `UnknownPublishingPoint` and `UnsupportedMedia`, never constructed: the listener answers those three statuses directly (output auth challenge, path mismatch `404`, demux failure `415`).

```text
MultimuxError::Depay { reason }                  -> (removed; never constructed)
IngestError::Auth                                -> (removed; never constructed)
IngestError::UnknownPublishingPoint              -> (removed; never constructed)
IngestError::UnsupportedMedia                    -> (removed; never constructed)
```

A `match` that named any of these arms must drop them. To react to a demux or depayload failure, match `MultimuxError::Transmux` or `MultimuxError::Flv`, which carry the source error.

```rust
// before
match err {
    MultimuxError::Depay { reason } => warn!("depay: {reason}"),
    other => return Err(other),
}

// after
return Err(err);
```

## Other changes

- The RTSP, TS-HTTP, HLS-pull, DASH-pull and Smooth-pull inputs share one crate-private `spawn_pull::<K>` supervised spawn path and one `pull_route::<K>` constructor, replacing five per-input spawn closures (#1725). Behaviour is unchanged.

## Dependencies

Workspace sibling requirements move to the new epochs of this release:

- `webrtc-runtime` 0.5 -> 0.6 (WHIP and WHEP on one engine, #1725; see the webrtc-runtime 0.6.0 note)
- `rtsp-runtime` 0.9 -> 0.10 (`Error` carries source errors, #1726; see the rtsp-runtime 0.10.0 note)
- `broadcast-client` 0.3 -> 0.4

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `multimux-v0.17.0`.

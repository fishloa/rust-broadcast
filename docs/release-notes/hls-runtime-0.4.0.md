# hls-runtime 0.4.0

**Release date:** 2026-08-02

Renames the crate from `ll-hls-client` to `hls-runtime` to match the workspace convention (`*-runtime` = session engine). Adds a builder API for client configuration, a `Container` enum for output format selection, and fixes `#EXT-X-VERSION` derivation to comply with RFC 8216bis.

## What's new

- `HlsPullClientBuilder` — builder pattern for configuring the pull client.
- `Container` enum — select output format (fMP4, TS).
- Version derivation fix: `#EXT-X-VERSION` is now correctly derived from the highest-versioned tag present.

## What changed

- **Crate renamed** from `ll-hls-client` to `hls-runtime`.
- Requires `media-plane` 0.2.

## Migration

Breaking: rename your dependency from `ll-hls-client` to `hls-runtime`. All import paths change from `ll_hls_client::` to `hls_runtime::`.

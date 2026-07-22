# Security Policy

`rust-broadcast` is a family of **parsers for untrusted broadcast data** (transport
streams, SI/PSI sections, descriptors, BBFrames, SCTE-35). Parsing
attacker-influenced input safely is a primary goal.

## Supported versions

Security fixes are released for the **latest published minor** of the lockstep
core crates (`broadcast-common`, `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`,
`dvb-tools`) and the latest independently-versioned crates (`dvb-stream`,
`scte35-splice`, `mp4-emsg`, and others). Older versions are not patched —
upgrade to the latest release.

| Crate set | Supported |
|---|---|
| Core crates, latest minor (8.5.x) | ✅ |
| Anything older | ❌ |

## Reporting a vulnerability

Report privately via **GitHub Security Advisories**
(<https://github.com/fishloa/rust-broadcast/security/advisories/new>) — please do not
open a public issue for an unfixed vulnerability. Include a reproducer
(ideally a TS/section byte blob or a failing test) and the crate + version.

We aim to acknowledge within a few days and to ship a fix in a patch release.

## Hardening posture

- **No panics on malformed input.** Parsers validate tag/length before slicing
  and return structured `thiserror` errors (`BufferTooShort`, `InvalidDescriptor`,
  …) rather than panicking. A panic on any byte input is treated as a bug.
- **No `unsafe`** in the parsing paths.
- **`#![no_std]` + `alloc`**, MSRV 1.86 — usable in constrained/embedded targets.
- **Fuzzed.** `cargo-fuzz` targets exercise the section/descriptor/BBFrame
  parsers against arbitrary input; round-trip and real-broadcast-capture
  fixtures guard correctness.

If you find an input that panics, hangs, or reads out of bounds, that is a
security-relevant bug — please report it as above.

# dvb-ci-runtime

Pure-Rust **EN 50221 DVB Common Interface runtime** — the driver loop over the
[`dvb-ci`](https://crates.io/crates/dvb-ci) wire codecs.

`dvb-ci` is `no_std` and owns the **wire** layer (TPDU / SPDU / APDU
parse+serialize, CA_PMT building, CI Plus extensions). `dvb-ci-runtime` adds the
**runtime**: device I/O, the TPDU poll loop, SPDU session management, and the
per-resource state machines that drive a physical CAM (ETSI EN 50221, TS 101 699).

## Design

Everything is written against the `CaDevice` trait, so the runtime runs against
either a real Linux CA device (`/dev/dvb/adapterN/caM`, the `linux` feature) or an
in-memory `MockCaDevice`. The mock makes the state machines testable without
hardware and enables differential testing against an external reference — drive
both with the same scripted mock CAM, assert the emitted `write`/ioctl byte
sequences match.

Implemented from the EN 50221 specification.

## Status

Foundation: the `CaDevice` abstraction + mock. TPDU/SPDU/resource state machines
and the Linux device implementation land incrementally.

## License

MIT OR Apache-2.0.

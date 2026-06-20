# dvb-si-py

Python bindings for [`dvb-si`](https://crates.io/crates/dvb-si) — parse DVB SI/PSI
sections and demux MPEG-TS into Python dicts. Read-only by design (parse →
`serde_json` → Python objects), which is exactly what broadcast-ops scripting
(pcap-to-report, capture triage) needs.

Built with [PyO3](https://pyo3.rs) + [maturin](https://www.maturin.rs); ships as an
abi3 wheel (one wheel for CPython ≥ 3.9). Not part of the Rust workspace — it
consumes the published `dvb-si` / `dvb-t2mi` crates by version.

## Install

```console
$ pip install dvb-si-py
```

## Usage

```python
import dvb_si_py as dvb

# Parse a single SI/PSI section (table) from raw bytes → dict.
section = dvb.parse_section(section_bytes)
print(section["tableId"], section)

# Demux: feed aligned 188-byte TS packets, get the dicts of changed sections.
dm = dvb.Demux()
with open("capture.ts", "rb") as f:
    data = f.read()
for i in range(0, len(data) - 188, 188):
    for sec in dm.feed(data[i:i + 188]):
        print(sec)            # e.g. {"PatSection": {...}}, {"SdtSection": {...}}

# T2-MI: pump payloads off a PID.
t2 = dvb.T2miDemux(0x0040)
for i in range(0, len(data) - 188, 188):
    for payload in t2.feed(data[i:i + 188]):
        print(payload)
```

Keys mirror the Rust API (camelCase). The binding is read-only: it parses, it does
not build/serialize wire bytes — use the Rust crates for that.

## Build from source

```console
$ pip install maturin
$ maturin develop          # build + install into the active virtualenv
$ maturin build --release  # build a wheel
```

## License

MIT OR Apache-2.0.

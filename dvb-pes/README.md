# dvb-pes (DEPRECATED)

This crate has been renamed to [`mpeg-pes`](https://crates.io/crates/mpeg-pes).

PES (Packetised Elementary Stream) depacketisation + PTS/DTS is an ISO/IEC 13818-1
standard, not DVB-specific, so the crate was renamed accordingly.

**Please update your dependency:**

```toml
# Before:
dvb-pes = "0.1"

# After:
mpeg-pes = "0.1"
```

Replace all `dvb_pes::` references with `mpeg_pes::` in your source code.

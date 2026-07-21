# transmux 0.18.1 — 2026-07-21

Patch. Additive, opt-in RTCP wallclock A/V-sync for the streaming RTP demux. No breaking change; dependents on `^0.18` pick it up on rebuild.

## Added — RTCP SR wallclock A/V-sync (#722)

`RtpStreamDepacketiser` previously rebased each track's RTP timestamp to 0 **independently**, so two RTP streams that start at different timestamp bases (the normal case) lost cross-track A/V alignment. RTP timestamps alone cannot align tracks; RTCP **Sender Reports** carry the NTP-wallclock ↔ RTP-timestamp anchor that can (RFC 3550 §6.4.1).

- **`RtpStreamDepacketiser::push_sender_report(track_id, SenderReport)`** — anchor a track's NTP↔RTP correlation.
- **`push_rtcp(track_id, &[u8])`** — convenience wrapper parsing raw RTCP bytes via `rtcp_packet::SenderReport::parse`.
- **`sync_start_decode_times() -> Vec<(u32, u64)>`** — once ≥2 tracks have an SR anchor + a first sample, maps each anchored track's first sample onto one common wallclock (earliest = origin) and returns each track's `start_decode_time` in its own clock-rate ticks, preserving the real inter-track offset.

**Strictly additive / opt-in**: with no Sender Reports fed, `sync_start_decode_times` returns an empty `Vec` and existing callers keep the unchanged v1 (independent-rebase) behaviour.

## Validated

`transmux/tests/rtcp_av_sync.rs` — synthetic video (90 kHz, RTP base 1,000,000) + audio (48 kHz, RTP base 7,000,000) whose real-world captures start 1.0 s apart, each with a marker AU engineered to coincide in real time. Measured:

```
marker offset WITH SR:    0.000000 s   (bound < 0.001 s)
marker offset WITHOUT SR: 1.000000 s   (negative control — materially wrong)
```

The negative control proves the feature bites: without SRs, independent rebase-to-0 misaligns by the full 1.0 s start-time delta; with SRs, alignment is exact.

## Compatibility

MSRV 1.86. Additive API only — recompile.

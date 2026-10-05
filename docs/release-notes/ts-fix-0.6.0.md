# ts-fix 0.6.0

_Released 2026-10-05._

**Minor-epoch release (0.x); fixes only, no flag or output-format change.** Four defects that produced wrong or undecryptable output are fixed: `--service` dropped CA/ECM/EMM PIDs, a multi-operation run lost or bypassed the flush output of earlier operations, the CLI aborted on any capture that was not a clean multiple of 188 bytes, and `--regen-psi` regenerated a PAT with a resetting continuity counter, a fixed version and no network PID. Re-run any pipeline that used `--service` on a scrambled service or `--regen-psi`. The dependency epochs move with the wave: see [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md), [mpeg-pes 0.5.0](mpeg-pes-0.5.0.md), [dvb-si 11.0.0](dvb-si-11.0.0.md), [dvb-conformance 11.0.0](dvb-conformance-11.0.0.md) and [scte35-splice 3.0.0](scte35-splice-3.0.0.md).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-mpeg-ts          = { version = "0.4", default-features = false }
+mpeg-ts          = { version = "0.5", default-features = false }
-mpeg-pes         = { version = "0.4", default-features = false }
+mpeg-pes         = { version = "0.5", default-features = false }
-dvb-si           = { version = "10",  default-features = false }
+dvb-si           = { version = "11.0", default-features = false }
-dvb-conformance  = { version = "10",  default-features = false }
+dvb-conformance  = { version = "11.0", default-features = false }
-scte35-splice    = { version = "2.1", default-features = false }
+scte35-splice    = { version = "3.0", default-features = false }
```

## Fixes

- `--service` (service extract, `PidFilterOp`) (#1101). The keep-set was only the PMT's `pcr_pid` and `elementary_pid`s, so every ECM PID (from the CA descriptors in the PMT's program-info and ES-info loops, ISO/IEC 13818-1 §2.6.16), the CAT (PID 0x0001) and the EMM PIDs it references were dropped, and extracting a scrambled service produced undecryptable output. CA PIDs, the CAT and its EMM PIDs are now kept. The previously terminal `Resolved` state also keeps observing PAT/PMT version changes, so a programme whose PMT PID moves or whose ES PIDs are added mid-stream is no longer silently truncated.
- Multi-operation `finish` (#1101). Each operation's flush output is now fed through every later operation's `process`, as `push` already did for packets. Previously only the last operation drained the staging buffer, so the fallback PAT that `--regen-psi` emits from `PsiRegenOp::flush` when the input has no PAT slot bypassed `repair_continuity` and `stuffing`, or was dropped entirely when a later operation was configured.
- The CLI no longer aborts on a capture that is not a clean, sync-perfect multiple of 188 bytes (#1101). It now resynchronises with `mpeg_ts::resync::TsResync` (as `dvb-tools` does) instead of `chunks(188)`, so a capture starting mid-packet, a single corrupted sync byte, or a 204-byte RS-coded capture (parity stripped) is repaired, with the resync counts reported on stderr. Before, it failed with `missing TS sync byte` and wrote no output.
- `--regen-psi` (`PsiRegenOp`) (#1037). Every regenerated PAT packet used a fresh `SectionPacketiser`, so `continuity_counter` restarted at 0 on each emission; `version_number` was hard-coded to 0 even when the program mapping changed; and the original PAT's `network_pid` entry (`program_number == 0`, ISO/IEC 13818-1 §2.4.4.3), which cannot be derived from any PMT, was dropped. The packetiser is now reused across emissions, `version_number` increments only when the mapping actually changes, and the `network_pid` entry is preserved when present. The `--regen-psi` help text and `regen_psi` docs no longer say the PAT is rebuilt "on flush"; it is replaced in position.

---

Published from tag `ts-fix-v0.6.0`.

# ts-fix 0.6.0

_Released 2026-10-05._

### Fixed
- `--service` (service extract, `PidFilterOp`): the keep-set was built from the
  PMT's `pcr_pid` + `elementary_pid`s only, so every ECM PID (from the
  CA_descriptors in the PMT's program-info/ES-info loops, ISO/IEC 13818-1
  §2.6.16), the CAT (PID 0x0001) and its EMM PIDs were dropped — extracting a
  scrambled service produced an undecryptable output. CA PIDs, the CAT and the
  EMM PIDs it references are now kept, and the previously terminal `Resolved`
  state keeps re-observing PAT/PMT version changes, so a programme whose PMT
  PID moves or whose ES PIDs are added mid-stream is no longer silently
  truncated (#1101).
- Multi-op `finish`: each op's flush output is now fed forward through every
  later op's `process`, exactly as `push` chains packets. Previously only the
  last op drained the staging buffer, so `--regen-psi`'s fallback PAT (emitted
  from `PsiRegenOp::flush` when the input has no PAT slot) bypassed
  `repair_continuity` and `stuffing` — or, with a later op configured, was
  silently dropped from the output entirely (#1101).
- The `ts-fix` CLI no longer aborts the whole run on a capture that is not a
  clean, sync-perfect multiple of 188: it resynchronises the input byte stream
  with `mpeg_ts::resync::TsResync` (like `dvb-tools`) instead of slicing with
  `chunks(188)`, so a capture starting mid-packet, a single corrupted sync
  byte, or a 204-byte RS-coded capture (parity stripped) is now repaired with
  the resync counts reported on stderr, instead of failing with
  `missing TS sync byte` and writing no output. The `--regen-psi` help text
  and `regen_psi` docs also no longer claim the PAT is rebuilt "on flush" —
  it is replaced in-position (#1101).
- `--regen-psi` (`PsiRegenOp`): every regenerated PAT packet was built with a
  fresh `SectionPacketiser`, so `continuity_counter` restarted at 0 on every
  emission instead of incrementing across the PAT's repeat cycle; `PAT`
  `version_number` was hardcoded to 0 forever, even when the discovered
  program mapping changed; and the original PAT's `network_pid` entry
  (`program_number == 0`, ISO/IEC 13818-1 §2.4.4.3) — not derivable from any
  PMT — was silently dropped from the regenerated PAT. The packetiser is now
  reused across emissions, `version_number` bumps only when the mapping
  actually changes, and the `network_pid` entry is preserved from the
  original PAT when present (#1037).

---

Published from tag `ts-fix-v0.6.0`.

# ts-fix 0.11.0

_Released 2026-11-22._

Minor for a 0.x crate. One real fix: a PMT version change no longer carries the replaced PMT's PIDs into a service extract. The breaking part is the epoch move (`broadcast-common` 14, `dvb-conformance` 16, `dvb-si` 16, `mpeg-pes` 0.10, `mpeg-ts` 0.10, `scte35-splice` 8).

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Fixed

- **A PMT version change no longer carries the replaced PMT's PIDs into a service extract.** The keep-set rebuild treated every non-PSI PID as an EMM PID learned from the CAT, so the old elementary-stream, PCR and ECM PIDs stayed in the keep-set and a replaced stream leaked into the extracted service. The PMT's own PIDs are now tracked separately, so only the CAT-learned EMM PIDs survive the change.

## Tests

The service-extract keep-set is pinned from every side: a clear (non-CA) service carries neither the CAT nor a CAT-advertised EMM PID, a CA-signalling PMT keeps the CAT even with an empty CAT, no packet but the PAT may lead the output before the PAT is parsed, and a CAT version change is parsed while a repeat is ignored (audit 23 findings 133-136). PSI regeneration is pinned at the pointer_field, the flush chain and the unknown-`transport_stream_id` fallback (audit 23 findings 137-139), and the null-packet mask, inserted-null continuity counters, interpolated duplicate PCR and CLI resync diagnostic are pinned (audit 23 findings 130/132/140/141).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4022 (test bullets under a heading).

---

Published from tag `ts-fix-v0.11.0`.

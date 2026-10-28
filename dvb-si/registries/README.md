# Vendored registry data (TSDuck `.names`)

These data files are the **source of truth** for the living value→name registries
that have no fixed vendored ETSI/ISO standard (CA_system_id, private_data_specifier).
They are copied **verbatim** from [TSDuck](https://github.com/tsduck/tsduck) and
compiled into lookup functions at build time by ../../build.rs (issue #141).
Editing the data + rebuilding updates the lookups; no code changes needed.

**Do not hand-edit the values.** Re-sync by re-copying the upstream file (and
bumping the provenance below). Copying verbatim is deliberate: it removes any
chance of a transcription/codegen mistake inventing a registry value.

## Provenance

| File | Upstream path | Section(s) used |
|---|---|---|
| `tsCAS.names` | `src/libtsduck/dtv/cas/tsCAS.names` | `[CASystemId]` |
| `tsPDS.names` | `src/libtsduck/dtv/dvb/tsPDS.names` | `[PrivateDataSpecifier]` |

- **Source repository:** https://github.com/tsduck/tsduck
- **Commit:** `ab49cf29f239c337170e0ac8d4bf1db5aafe0aa4`
- **Synced:** 2026-06-12

(`tsCAS.names` also carries `[CASFamily]`/`[CASFamilyRange]` sections, which are
TSDuck-internal and unused here — kept only so the file stays byte-verbatim.)

## License / attribution

TSDuck is distributed under the **BSD 2-Clause License** (compatible with this
crate's `MIT OR Apache-2.0`). The required copyright notice and license text:

```
BSD 2-Clause License

Copyright (c) 2005-2026, Thierry Lelegard
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

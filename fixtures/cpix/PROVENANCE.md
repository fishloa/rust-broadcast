# Provenance

## CPIX key-exchange fixtures (`fixtures/cpix/`)

Prep work for issue #745 (multimux DRM key-server integration). See `private/specs-verified/dash/cpix/`
for the full transcription these fixtures support.

| File | Source | Date | Licence |
|---|---|---|---|
| `widevine-pssh-from-complex.bin` | Derived from the DASH-IF CPIX test vector `Complex.xml` (`https://github.com/Dash-Industry-Forum/cpix-test-vectors`, commit `e5cef98097a6115d6f561ace5eebba84ce950f3a`, MIT, © 2016 Axinom; the XML documents themselves are no longer vendored: nothing read them). Read by `fixtures/mp4/uuid-boxes/gen.py`. Derivation: the 56-byte raw `pssh` box, extracted by base64-decoding that document's Widevine `DRMSystem/ContentProtectionData` element (which itself wraps a base64 `<pssh>` element per the CPIX schema) with a one-off Python script (not committed — decode is: base64-decode `ContentProtectionData` text → parse the resulting `<pssh xmlns="urn:mpeg:cenc:2013">...</pssh>` XML fragment → base64-decode its text content). Verified against `private/specs-verified/mpeg/transmux/drm/public-notes/pssh.md` §2/§4 field-by-field: box header (`size`/`pssh`/`version 0`/`flags 0`), Widevine SystemID, protobuf field 2 (`key_id`, matches the `DRMSystem@kid` attribute) and field 9 (`protection_scheme` varint = `0x63656E63` = `"cenc"`) all decode exactly per the documented layout — see `private/specs-verified/dash/cpix/cpix-2.3-dashif.md` §11 for the full worked decode. Issue #745. | 2021-05 source / 2026-08 derived | MIT (source data); derived bytes are a mechanical extraction, not a new creative work |

Not vendored from the same repo (available upstream if a later implementation phase
needs them, but not needed for spec-transcription/fixture prep): the X.509
test-certificate `.cer`/`.pfx` pairs, and the intentionally-invalid
`Invalid_BadContentKeysSignature.xml` / `Invalid_BadDocumentSignature.xml` /
`Invalid_WrongMac.xml` negative fixtures.

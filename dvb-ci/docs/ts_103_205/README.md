# ETSI TS 103 205 — CI Plus extensions spec transcription

Render-verified Markdown transcription of the **freely-redistributable** ETSI
TS 103 205 v1.4.1 ("Extensions to the CI Plus Specification") syntax tables, for
the `dvb-ci` crate (issue #288, WP-B). Spec-md-first phase: docs only, no Rust.

Source PDF: `specs/etsi_ts_103_205_v01.04.01_dvb_ci_plus_extensions.pdf`
(178 pp). Every table was read from the **rendered PDF page** (Read with
`pages=`, never pdftotext) and transcribed exactly, with §/Table/PDF-page
citations and a `_Source: ... render-verified_` line per file.

> The proprietary CI Plus LLP spec (v1.4.3, all-rights-reserved) was NOT used.
> Where TS 103 205 defers an APDU layout to the proprietary CI Plus V1.3 [3]
> spec, that layout is NOT transcribed — only its tag/direction/reference noted.

## Files

- **`resource-ids.md`** — master registry: all TS 103 205 resources, their
  32-bit `resource_identifier` values, and every APDU `apdu_tag` with direction.
- **`multi-stream-resource.md`** — §6.4.2 Multistream resource (`0x00900041`):
  CICAM_multistream_capability, PID_select_req/reply.
- **`content-control.md`** — §6.4.3 Content Control multi-stream type
  (`0x008C1041`): cc_PIN_reply / cc_PIN_event APDU extensions + the §6.4.3.3 SAC
  protocol-extension tables (URI / Record Start / Record Stop / Change Operating
  Mode / License Exchange).
- **`ca-support.md`** — §6.4.4 CA Support multi-stream type (resource_type 2):
  ca_pmt / ca_pmt_reply with `LTS_id` (+ `PMT_PID`).
- **`multi-stream-host-control.md`** — §6.4.5 Multi-stream Host Control
  (`0x00200081`) tune APDUs + the §13.2 DVB Host Control v3 (`0x00200043`) base
  tune syntaxes and tags.
- **`sample-decryption.md`** — §7.4 Sample decryption resource (`0x00920041`):
  sd_info/sd_start/sd_update req+reply (+ DRM-metadata-source Table 34).
- **`cicam-player.md`** — §8.8 CICAM Player resource (`0x00930041`): the full
  CICAM_player_* APDU set.
- **`file-retrieval.md`** — §9 Auxiliary File System resource (`0x00910041`):
  FileSystemOffer / FileSystemAck (+ FileRequest / FileAcknowledge by reference).
- **`low-speed-comms-v4.md`** — §10 LSC v4: comms_info / comms_IP_config APDUs +
  hybrid/multicast descriptors + the hybrid TS-packet syntax (§10.12.4.1).
- **`usage-rules-v3.md`** — §11 URI v3 message syntax (+ trick_mode_control_info).
- **`ci-plus-descriptors.md`** — §7.5.5.4 IV descriptor (`0xD0`) + key-identifier
  descriptor (`0xD1`). **Closes the deferral** in `../ci_plus/fragment-header.md`.

## Out of scope / deferred by TS 103 205

These APDU bodies are referenced to the proprietary CI Plus V1.3 [3] spec and are
NOT printed in TS 103 205 (so NOT transcribed): the unextended Content Control
`cc_*` APDUs (§11.3.x); `tune_broadcast_req` / `tune_reply` / `ask_release(_reply)`
tags & bodies (§14.6.x / Table 14.30); FileRequest / FileAcknowledge bodies
(§14.5.1/.2); base LSC `comms_cmd`/`comms_reply`/`comms_send`/`comms_rcv` (§14.1);
PINcode/license blobs (opaque). Crypto/DRM/SAC/license payloads are transcribed as
wire structure only (field + length + mnemonic), bodies marked opaque.

## Re-check list (flagged ⚠, not invented)

- **`resource-ids.md`** — CA Support multi-stream type: §6.4.4.1 gives only
  "resource_type = 2, version = 1"; NO full 32-bit ID printed (likely
  `0x000C0041` but unconfirmed — do not encode without checking). LSC v4: no
  resource-summary table; base ID in proprietary CI Plus V1.3 §14.1.
- **`multi-stream-resource.md`** — Table 5 (`PID_select_reply`): the 7-bit
  `reserved` and `PID_selected_flag` mnemonics are printed `uimsbf` (not `bslbf`);
  transcribed as rendered.
- **`multi-stream-host-control.md`** — `tune_broadcast_req`/`tune_reply`/
  `ask_release(_reply)` tags not printed. Table 21 multi-stream `tune_ip_req` has a
  **1-bit** `reserved` prefix vs the §13.2.4 base (Table 100) **2-bit** reserved —
  divergent layouts, both transcribed as rendered. Table 98 first-row mnemonic
  printed `tune_lcn_triplet_tag` (label slip; authoritative tag `0x9F8409`).
- **`sample-decryption.md`** — `length_field()` shown as "16" bits in Tables
  32/33/38 (vs bare `length_field()` elsewhere); `sd_start_reply.drm_uuid` width
  printed `16*8` (=128) vs plain `128` in Tables 32/33. Transcribed as rendered.
- **`cicam-player.md`** — Table 69 (`CICAM_player_update_reply`) first syntax row
  literally prints `CICAM_player_start_reply_tag` (copy/paste slip); authoritative
  tag is `0x9FA00F`.
- **`low-speed-comms-v4.md`** — Table 80 `connection_state` Reserved range printed
  `0x10-0x11` but the field is 2 bits (evident typo, likely `0x02`–`0x03`).
  Tables 78/79 print tags without `0x` prefix (`9F8C09`/`9F8C0A`); Tables 76/77
  with `0x`. Same namespace.
- **`ci-plus-descriptors.md`** — IV/key-id descriptors are TLV variable-length
  (tag + length + opaque body), confirming the earlier `fragment-header.md`
  deferral; bodies are opaque crypto octets.

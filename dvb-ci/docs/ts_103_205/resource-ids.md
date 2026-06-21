# TS 103 205 resource identifiers & APDU tags

_Source: ETSI TS 103 205 v1.4.1, resource-summary Tables 2/6/30/71/75 + §6.4.4.1 / §6.4.5.1 / §13.1 prose, render-verified_

Master registry of the CI Plus extension resources defined (or extended) by
TS 103 205, their 32-bit `resource_identifier` values, and every APDU `apdu_tag`.
Resource IDs follow the EN 50221 §8.2.2 / Table 15 packing:
`resource_id_type` (2) + `resource_class` (14) + `resource_type` (10) +
`resource_version` (6), with `resource_id_type = 0` for public resources.

> CI Plus resources have their own apdu_tag space; tags here may collide with
> EN 50221 (`../en_50221/`) / TS 101 699 (`../ci_plus/`) tags — that is expected,
> these are a separate resource namespace.

## Resource identifiers

| Resource | Resource ID | Class | Type | Ver | Source |
|----------|-------------|-------|------|-----|--------|
| Multistream | `0x00900041` | 144 | 1 | 1 | §6.4.2.1 Table 2 (p. 31) |
| Content Control (multi-stream type) | `0x008C1041` | 140 | 65 | 1 | §6.4.3.1 Table 6 (p. 34) |
| Conditional Access Support (multi-stream type) | ⚠ NOT printed — defer, do not encode | — | 2 | 1 | §6.4.4.1 prose (p. 38) |
| Multi-stream Host Control | `0x00200081` | — | — | — | §6.4.5.1 prose (p. 40) |
| DVB Host Control v3 (base of above) | `0x00200043` | — | — | — | §13.1 prose (p. 126) |
| Sample Decryption | `0x00920041` | 146 | 1 | 1 | §7.4 Table 30 (p. 53) |
| CICAM Player | `0x00930041` | 147 | 1 | 1 | §8.8.19 Table 71 (p. 96) |
| Auxiliary File System | `0x00910041` | 145 | 1 | 1 | §9.1 prose + §9.6 Table 75 (pp. 96/99) |
| Low Speed Communication v4 | ⚠ no summary table | — | — | 4 | §10 (base ID in CI Plus V1.3 [3] §14.1) |

⚠ **CA Support multi-stream type — resource_id NOT printed in TS 103 205; defer
(do not encode).** §6.4.4.1 (p. 38, render-verified) states only "the Conditional
Access Support resource has a new resource_type defined (resource_type = 2,
version = 1), in which the ca_pmt() and ca_pmt_reply() APDU objects are extended".
No full 32-bit `resource_identifier` value appears anywhere in §6.4.4. (For
reference only — NOT to be encoded — the EN 50221 CA Support class is 3, which
*would* pack to `0x000C0041`, but the spec does not print this, so the constant is
unconfirmed and must not be hard-coded from TS 103 205.) The extended APDUs
themselves (`ca_pmt` `0x9F8032`, `ca_pmt_reply` `0x9F8033`, Tables 14/16) ARE fully
specified and encodable.

⚠ **LSC v4** — §10 prints no resource-summary table with a single 32-bit value;
the LSC resource identifier and base `comms_*` tags are in CI Plus V1.3 [3]
§14.1 (proprietary, not vendored). Only the new v4 APDU tags are established.

## APDU tag summary (per resource)

### Multistream (`0x00900041`) — Table 2
| APDU | Tag | Dir |
|------|-----|-----|
| CICAM_multistream_capability | `9F 92 00` | CICAM→Host |
| PID_select_req | `9F 92 01` | CICAM→Host |
| PID_select_reply | `9F 92 02` | Host→CICAM |

### Content Control multi-stream (`0x008C1041`) — Table 6
`cc_open_req` `9F9001` · `cc_open_cnf` `9F9002` · `cc_data_req` `9F9003` ·
`cc_data_cnf` `9F9004` · `cc_sync_req` `9F9005` · `cc_sync_cnf` `9F9006` ·
`cc_sac_data_req` `9F9007` · `cc_sac_data_cnf` `9F9008` · `cc_sac_sync_req`
`9F9009` · `cc_sac_sync_cnf` `9F9010` · `cc_PIN_capabilities_req` `9F9011` ·
`cc_PIN_capabilities_reply` `9F9012` · `cc_PIN_cmd` `9F9013` · `cc_PIN_reply`
`9F9014` · `cc_PIN_event` `9F9015` · `cc_PIN_playback` `9F9016` · `cc_PIN_MMI_req`
`9F9017`. (Only `cc_PIN_reply`/`cc_PIN_event` syntax is printed in TS 103 205; the
rest defer to CI Plus V1.3 [3] §11.3.x.)

### CA Support multi-stream (resource_type 2)
`ca_pmt` `9F8032` (Host→CICAM) · `ca_pmt_reply` `9F8033` (CICAM→Host) — same tags
as EN 50221, extended bodies (Tables 14/16).

### Multi-stream Host Control (`0x00200081`) / DVB Host Control v3 (`0x00200043`)
`tune_triplet_req` `9F8409` · `tune_lcn_req` `9F8407` · `tune_ip_req` `9F8408` ·
`tuner_status_req` `9F840A` · `tuner_status_reply` `9F840B`.
⚠ `tune_broadcast_req`, `tune_reply`, `ask_release`, `ask_release_reply` tags NOT
printed (defer to CI Plus V1.3 [3] §14.6.x / Table 14.30).

### Sample Decryption (`0x00920041`) — Table 30
`sd_info_req` `9F9800` · `sd_info_reply` `9F9801` · `sd_start` `9F9802` ·
`sd_start_reply` `9F9803` · `sd_update` `9F9804` · `sd_update_reply` `9F9805`.

### CICAM Player (`0x00930041`) — Table 71
`CICAM_player_verify_req` `9FA000` · `…_verify_reply` `9FA001` ·
`…_capabilities_req` `9FA002` · `…_capabilities_reply` `9FA003` · `…_start_req`
`9FA004` · `…_start_reply` `9FA005` · `…_play_req` `9FA006` · `…_status_error`
`9FA007` · `…_control_req` `9FA008` · `…_info_req` `9FA009` · `…_info_reply`
`9FA00A` · `…_stop` `9FA00B` · `…_end` `9FA00C` · `…_asset_end` `9FA00D` ·
`…_update_req` `9FA00E` · `…_update_reply` `9FA00F`.

### Auxiliary File System (`0x00910041`) — Table 75
`FileSystemOffer` `9F9400` (CICAM→Host) · `FileSystemAck` `9F9401` (Host→CICAM) ·
`FileRequest` `9F9402` (Host→CICAM) · `FileAcknowledge` `9F9403` (CICAM→Host).

### Low Speed Communication v4 — new APDUs only
`comms_info_req` `9F8C07` · `comms_info_reply` `9F8C08` · `comms_IP_config_req`
`9F8C09` · `comms_IP_config_reply` `9F8C0A`. (Base `comms_cmd`/`comms_reply`/
`comms_send`/`comms_rcv` tags defer to CI Plus V1.3 [3] §14.1.)

# Consolidated value registries — message_type, parameter_type, error_status

_Source: ETSI TS 103 197 V1.5.1 — Table 2 (p. 27), Table 3 (pp. 27-28), Table 5 (p. 31), Table 6 (p. 39), Table 7 (p. 42), Table 8 (p. 47), render-verified_

This file gathers the numeric registries in one place. Per-interface detail and
the message parameter lists live in `ecmg-scs.md`, `emmg-pdg-mux.md`, and (for
framing) `message-framing.md`. All values are big-endian.

## 1. protocol_version per interface (Table 2, §4.4.1 p. 27)

| Interface | protocol_version |
|-----------|------------------|
| ECMG ⇔ SCS | `0x03` |
| EMMG ⇔ MUX | `0x03` |
| C(P)SIG ⇔ (P)SIG | `0x03` |
| NMS ⇔ SIMF Agent | `0x03` |
| (P)SIG ⇔ MUX | `0x04` |
| EIS ⇔ SCS | `0x04` |

`0x05` on ECMG⇔SCS and EMMG⇔MUX = DVB-H (annex N). ACG⇔EIS and
SIMCOMP⇔MUXCONFIG versions not in Table 2 (defined in §11/§12, not transcribed).

## 2. message_type registry, grouped per interface (Table 3, §4.4.1 pp. 27-28)

### ECMG ⇔ SCS

| message_type | message |
|--------------|---------|
| `0x0001` | channel_setup |
| `0x0002` | channel_test |
| `0x0003` | channel_status |
| `0x0004` | channel_close |
| `0x0005` | channel_error |
| `0x0101` | stream_setup |
| `0x0102` | stream_test |
| `0x0103` | stream_status |
| `0x0104` | stream_close_request |
| `0x0105` | stream_close_response |
| `0x0106` | stream_error |
| `0x0201` | CW_provision |
| `0x0202` | ECM_response |

### EMMG ⇔ MUX

| message_type | message |
|--------------|---------|
| `0x0011` | channel_setup |
| `0x0012` | channel_test |
| `0x0013` | channel_status |
| `0x0014` | channel_close |
| `0x0015` | channel_error |
| `0x0111` | stream_setup |
| `0x0112` | stream_test |
| `0x0113` | stream_status |
| `0x0114` | stream_close_request |
| `0x0115` | stream_close_response |
| `0x0116` | stream_error |
| `0x0117` | stream_BW_request |
| `0x0118` | stream_BW_allocation |
| `0x0211` | data_provision |

### C(P)SIG ⇔ (P)SIG

| message_type | message |
|--------------|---------|
| `0x0301` | channel_setup |
| `0x0302` | channel_status |
| `0x0303` | channel_test |
| `0x0304` | channel_close |
| `0x0305` | channel_error |
| `0x0311` | stream_setup |
| `0x0312` | stream_status |
| `0x0313` | stream_test |
| `0x0314` | stream_close |
| `0x0315` | stream_close_request |
| `0x0316` | stream_close_response |
| `0x0317` | stream_error |
| `0x0318` | stream_service_change |
| `0x0319` | stream_trigger_enable_request |
| `0x031A` | stream_trigger_enable_response |
| `0x031B` | trigger |
| `0x031C` | table_request |
| `0x031D` | table_response |
| `0x031E` | descriptor_insert_request |
| `0x031F` | descriptor_insert_response |
| `0x0320` | PID_provision_request |
| `0x0321` | PID_provision_response |

### EIS ⇔ SCS

| message_type | message |
|--------------|---------|
| `0x0401` | channel_set-up |
| `0x0402` | channel_test |
| `0x0403` | channel_status |
| `0x0404` | channel_close |
| `0x0405` | channel_error |
| `0x0406` | channel_reset |
| `0x0408` | SCG_provision |
| `0x0409` | SCG_test |
| `0x040A` | SCG_status |
| `0x040B` | SCG_error |
| `0x040C` | SCG_list_request |
| `0x040D` | SCG_list_response |

> Note: `0x0407` is absent from Table 3 (the SCG block starts at `0x0408`,
> after the channel block `0x0401`–`0x0406`). Per the table as printed.

### (P)SIG ⇔ MUX

| message_type | message |
|--------------|---------|
| `0x0411` | channel_set_up |
| `0x0412` | channel_test |
| `0x0413` | channel_status |
| `0x0414` | channel_close |
| `0x0415` | channel_error |
| `0x0421` | stream_setup |
| `0x0422` | stream_test |
| `0x0423` | stream_status |
| `0x0424` | stream_close_request |
| `0x0425` | stream_close_response |
| `0x0426` | stream_error |
| `0x0431` | CiM_stream_section_provision (carousel in MUX) |
| `0x0432` | CiM_channel_reset (carousel in MUX) |
| `0x0441` | CiP_stream_BW_request (carousel in (P)SIG) |
| `0x0442` | CiP_stream_BW_allocation (carousel in (P)SIG) |
| `0x0443` | CiP_stream_data_provision (carousel in (P)SIG) |

### Reserved / user ranges (Table 3)

| range | meaning |
|-------|---------|
| `0x0000`, `0x0006`–`0x0010`, `0x0016`–`0x0100`, `0x0107`–`0x0110`, `0x0119`–`0x0200`, `0x0203`–`0x0210`, `0x0212`–`0x0300`, `0x0306`–`0x0310`, `0x0322`–`0x0400`, `0x040E`–`0x0410`, `0x0416`–`0x0420`, `0x0427`–`0x0430`, `0x0444`–`0x7FFF` | DVB reserved |
| `0x0433`–`0x0440` ⚠ | DVB reserved (printed `0x0433 to 0x040` — likely typo for `0x0440`; see message-framing.md) |
| `0x8000`–`0xFFFF` | User defined |

## 3. parameter_type registries

### ECMG ⇔ SCS — Table 5 (§5.2 p. 31)

| parameter_type | parameter | Type/units | Length |
|----------------|-----------|------------|--------|
| `0x0001` | Super_CAS_id | uimsbf | 4 |
| `0x0002` | section_TSpkt_flag | uimsbf | 1 |
| `0x0003` | delay_start | tcimsbf/ms | 2 |
| `0x0004` | delay_stop | tcimsbf/ms | 2 |
| `0x0005` | transition_delay_start | tcimsbf/ms | 2 |
| `0x0006` | transition_delay_stop | tcimsbf/ms | 2 |
| `0x0007` | ECM_rep_period | uimsbf/ms | 2 |
| `0x0008` | max_streams | uimsbf | 2 |
| `0x0009` | min_CP_duration | uimsbf/n×100ms | 2 |
| `0x000A` | lead_CW | uimsbf | 1 |
| `0x000B` | CW_per_msg | uimsbf | 1 |
| `0x000C` | max_comp_time | uimsbf/ms | 2 |
| `0x000D` | access_criteria | user defined | variable |
| `0x000E` | ECM_channel_id | uimsbf | 2 |
| `0x000F` | ECM_stream_id | uimsbf | 2 |
| `0x0010` | nominal_CP_duration | uimsbf/n×100ms | 2 |
| `0x0011` | access_criteria_transfer_mode | Boolean | 1 |
| `0x0012` | CP_number | uimsbf | 2 |
| `0x0013` | CP_duration | uimsbf/n×100ms | 2 |
| `0x0014` | CP_CW_combination (CP uimsbf 2B + CW variable) | --- | variable |
| `0x0015` | ECM_datagram | user defined | variable |
| `0x0016` | AC_delay_start | tcimsbf/ms | 2 |
| `0x0017` | AC_delay_stop | tcimsbf/ms | 2 |
| `0x0018` | CW_encryption | user defined | variable |
| `0x0019` | ECM_id | uimsbf | 2 |
| `0x7000` | error_status | see §5.6 | 2 |
| `0x7001` | error_information | user defined | variable |

Reserved: `0x0000`, `0x001A`–`0x6FFF`, `0x7002`–`0x7FFF` DVB reserved;
`0x8000`–`0xFFFF` user defined.

### EMMG/PDG ⇔ MUX — Table 7 (§6.2.2 p. 42)

| parameter_type | parameter | Type/units | Length |
|----------------|-----------|------------|--------|
| `0x0001` | client_id | uimsbf | 4 |
| `0x0002` | section_TSpkt_flag | uimsbf | 1 |
| `0x0003` | data_channel_id | uimsbf | 2 |
| `0x0004` | data_stream_id | uimsbf | 2 |
| `0x0005` | datagram | user defined | variable |
| `0x0006` | bandwidth | uimsbf/kbit/s | 2 |
| `0x0007` | data_type | uimsbf | 1 |
| `0x0008` | data_id | uimsbf | 2 |
| `0x7000` | error_status | see §6.2.6 | 2 |
| `0x7001` | error_information | user defined | variable |

Reserved: `0x0000`, `0x0009`–`0x6FFF`, `0x7002`–`0x7FFF` DVB reserved;
`0x8000`–`0xFFFF` user defined.

> The C(P)SIG⇔(P)SIG (§8.3.2.1 p. 102), EIS⇔SCS (§10.3 p. 148), (P)SIG⇔MUX
> (§9.3 p. 137) and ACG⇔EIS (§11.6 p. 167) parameter_type registries are
> **deferred** — see "Deferred" at the bottom.

## 4. error_status registries

### ECMG ⇔ SCS — Table 6 (§5.6 p. 39)

| error_status | Error type |
|--------------|-----------|
| `0x0001` | invalid message |
| `0x0002` | unsupported protocol version |
| `0x0003` | unknown message_type value |
| `0x0004` | message too long |
| `0x0005` | unknown Super_CAS_id value |
| `0x0006` | unknown ECM_channel_id value |
| `0x0007` | unknown ECM_stream_id value |
| `0x0008` | too many channels on this ECMG |
| `0x0009` | too many ECM streams on this channel |
| `0x000A` | too many ECM streams on this ECMG |
| `0x000B` | not enough control words to compute ECM |
| `0x000C` | ECMG out of storage capacity |
| `0x000D` | ECMG out of computational resources |
| `0x000E` | unknown parameter_type value |
| `0x000F` | inconsistent length for DVB parameter |
| `0x0010` | missing mandatory DVB parameter |
| `0x0011` | invalid value for DVB parameter |
| `0x0012` | unknown ECM_id value |
| `0x0013` | ECM_channel_id value already in use |
| `0x0014` | ECM_stream_id value already in use |
| `0x0015` | ECM_id value already in use |
| `0x7000` | unknown error |
| `0x7001` | unrecoverable error |

Reserved: `0x0000`, `0x0016`–`0x6FFF`, `0x7002`–`0x7FFF` DVB reserved;
`0x8000`–`0xFFFF` ECMG/CA-system/user defined.

### EMMG/PDG ⇔ MUX — Table 8 (§6.2.6 p. 47)

| error_status | Error type |
|--------------|-----------|
| `0x0001` | invalid message |
| `0x0002` | unsupported protocol version |
| `0x0003` | unknown message_type value |
| `0x0004` | message too long |
| `0x0005` | unknown data_stream_id value |
| `0x0006` | unknown data_channel_id value |
| `0x0007` | too many channels on this MUX |
| `0x0008` | too many data streams on this channel |
| `0x0009` | too many data streams on this MUX |
| `0x000A` | unknown parameter_type |
| `0x000B` | inconsistent length for DVB parameter |
| `0x000C` | missing mandatory DVB parameter |
| `0x000D` | invalid value for DVB parameter |
| `0x000E` | unknown client_id value |
| `0x000F` | exceeded bandwidth |
| `0x0010` | unknown data_id value |
| `0x0011` | data_channel_id value already in use |
| `0x0012` | data_stream_id value already in use |
| `0x0013` | data_id value already in use |
| `0x0014` | client_id value already in use |
| `0x7000` | unknown error |
| `0x7001` | unrecoverable error |

Reserved: `0x0000`, `0x0015`–`0x6FFF`, `0x7002`–`0x7FFF` DVB reserved;
`0x8000`–`0xFFFF` MUX/CA-system/user defined.

`error_information` (`0x7001`) is the optional companion to every
`error_status`: user-defined data (e.g. ASCII text or the faulty parameter ID).

## Deferred (with section refs for a follow-up)

These interfaces also have syntax/parameter-registry tables in TS 103 197 but
are **not** transcribed in this pass:

- **C(P)SIG ⇔ (P)SIG** — §8.3.2 message syntax & semantics (p. 102), §8.3.2.1
  parameter list (p. 102), §8.3.3–8.3.4 channel/stream message tables
  (pp. 107-116), §8.3.5 error status & error information (p. 117). Suggested
  file: `cpsig-psig.md`.
- **EIS ⇔ SCS** — §10.3 parameter_type values (p. 148), §10.4 parameter
  semantics (p. 148), §10.5–10.6 channel/SCG message tables (pp. 150-155),
  §10.7 error status (p. 156). Includes the SCG (Scrambling Control Group) and
  ECM_Group CompoundTLV (§10.6.7 p. 155). Suggested file: `eis-scs.md`.
- **(P)SIG ⇔ MUX** — §9.3 parameter_type values (p. 137), §9.4 semantics
  (p. 137), §9.5–9.8 message tables (pp. 139-143), §9.9 error status (p. 143).
- **ACG ⇔ EIS** — §11.6 parameter types (p. 167), §11.4 interface structure
  (p. 163), §11.8–11.9 message tables (pp. 168-172), §11.10 error status
  (p. 173).
- **SIMCOMP ⇔ MUXCONFIG** — §12.3 parameter semantics (p. 177), §12.4 interface
  structure (p. 179), §12.5–12.6 message tables (pp. 179-184).

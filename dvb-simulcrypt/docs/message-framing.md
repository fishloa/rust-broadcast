# Common message framing — generic TLV message structure

_Source: ETSI TS 103 197 V1.5.1 §4.4.1, Table 1b + Table 2 + Table 3 (PDF pp. 26-28); §4.4.2 Table 4 (PDF p. 29), render-verified_

> SCOPE NOTE: This crate parses **signalling** only. The control words carried
> in `CP_CW_combination` / `CW_encryption`, the ECMs in `ECM_datagram`, and the
> EMM/private data in `datagram` are **opaque payloads** — they are carried and
> framed, not interpreted. Likewise the actual scrambling keys are never
> decoded here.

All the connection-oriented Simulcrypt interfaces (ECMG⇔SCS, EMMG/PDG⇔MUX,
C(P)SIG⇔(P)SIG, (P)SIG⇔MUX, EIS⇔SCS, ACG⇔EIS, SIMCOMP⇔MUXCONFIG) share one
generic message structure: a fixed 5-byte header followed by zero or more TLV
parameters. All multi-byte fields are **big-endian** (most significant byte
first — Table 1b NOTE 1).

## Table 1b — generic_message structure (§4.4.1, p. 26)

| Syntax | Size | Notes |
|--------|------|-------|
| `generic_message {` | | |
| &nbsp;&nbsp;protocol_version | 1 byte | per Table 2 (interface-dependent) |
| &nbsp;&nbsp;message_type | 2 bytes | per Table 3 |
| &nbsp;&nbsp;message_length | 2 bytes | # bytes following this field |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | (parameters, any order) |
| &nbsp;&nbsp;&nbsp;&nbsp;parameter_type | 2 bytes | per interface-specific registry |
| &nbsp;&nbsp;&nbsp;&nbsp;parameter_length | 2 bytes | # bytes of parameter_value |
| &nbsp;&nbsp;&nbsp;&nbsp;parameter_value | `<parameter_length>` bytes | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (p. 26-27):

- **protocol_version** — 8-bit field, value per Table 2 (below).
- **message_type** — 16-bit field identifying the message type (Table 3).
  Unknown message types **shall be ignored** by the receiving entity.
- **message_length** — 16-bit field; number of bytes in the message
  *immediately following* the message_length field (i.e. the sum of all the
  parameter TLVs).
- **parameter_type** — 16-bit field. The list of values is defined per
  interface clause. Unknown parameters **shall be ignored**; their data is
  discarded and the rest of the message processed.
- **parameter_length** — 16-bit field; number of bytes of the following
  parameter_value field.
- **parameter_value** — variable-length; semantics specific to the
  parameter_type value.

NOTEs (p. 26):

- NOTE 1 — For parameters of two or more bytes, the first byte transmitted is
  the **most significant byte** (big-endian).
- NOTE 2 — Parameters need **not** be ordered within the generic message.
- NOTE 3 — Boolean `TRUE` = byte `0x01`, `FALSE` = byte `0x00`.

## Table 2 — Values for protocol_version parameter (p. 27)

| Interface | protocol_version |
|-----------|------------------|
| ECMG ⇔ SCS | `0x03` |
| EMMG ⇔ MUX | `0x03` |
| C(P)SIG ⇔ (P)SIG | `0x03` |
| NMS ⇔ SIMF Agent | `0x03` |
| (P)SIG ⇔ MUX | `0x04` |
| EIS ⇔ SCS | `0x04` |

- NOTE 4 — Compliance between protocol versions is described in annex I.
- NOTE 5 — protocol_version `0x05` on the ECMG⇔SCS and EMMG⇔MUX interfaces is
  specifically for "IP Datacasting over DVB-H" and is described in annex N.

> ⚠ p. 27 — Table 2 does **not** list ACG⇔EIS or SIMCOMP⇔MUXCONFIG
> protocol_version values; those interfaces carry their version in their own
> clauses (§11 / §12) — not transcribed here.

## The channel / stream concept

Every connection-oriented interface multiplexes work over **channels** and
**streams** carried on one transport connection (one channel per TCP
connection). The message_type registry (Table 3) is partitioned into:

- **channel-level** messages (setup / test / status / close / error), and
- **stream-level** messages (setup / test / status / close_request /
  close_response / error, plus interface-specific extras such as
  CW_provision, ECM_response, data_provision, BW_request/allocation).

Channel and stream identifiers are interface-specific (e.g.
`ECM_channel_id`/`ECM_stream_id` on ECMG⇔SCS, `data_channel_id`/
`data_stream_id` on EMMG/PDG⇔MUX) and are themselves carried as TLV
parameters.

## Table 3 — message_type value registry (§4.4.1, pp. 27-28)

The consolidated message_type registry for all command/response-based
interfaces. See `parameter-types.md` for this same registry re-grouped per
interface alongside the parameter_type registries.

| message_type | interface | message |
|--------------|-----------|---------|
| `0x0000` | DVB reserved | DVB reserved |
| `0x0001` | ECMG ⇔ SCS | channel_setup |
| `0x0002` | ECMG ⇔ SCS | channel_test |
| `0x0003` | ECMG ⇔ SCS | channel_status |
| `0x0004` | ECMG ⇔ SCS | channel_close |
| `0x0005` | ECMG ⇔ SCS | channel_error |
| `0x0006`–`0x0010` | DVB reserved | DVB reserved |
| `0x0011` | EMMG ⇔ MUX | channel_setup |
| `0x0012` | EMMG ⇔ MUX | channel_test |
| `0x0013` | EMMG ⇔ MUX | channel_status |
| `0x0014` | EMMG ⇔ MUX | channel_close |
| `0x0015` | EMMG ⇔ MUX | channel_error |
| `0x0016`–`0x0100` | DVB reserved | DVB reserved |
| `0x0101` | ECMG ⇔ SCS | stream_setup |
| `0x0102` | ECMG ⇔ SCS | stream_test |
| `0x0103` | ECMG ⇔ SCS | stream_status |
| `0x0104` | ECMG ⇔ SCS | stream_close_request |
| `0x0105` | ECMG ⇔ SCS | stream_close_response |
| `0x0106` | ECMG ⇔ SCS | stream_error |
| `0x0107`–`0x0110` | DVB reserved | DVB reserved |
| `0x0111` | EMMG ⇔ MUX | stream_setup |
| `0x0112` | EMMG ⇔ MUX | stream_test |
| `0x0113` | EMMG ⇔ MUX | stream_status |
| `0x0114` | EMMG ⇔ MUX | stream_close_request |
| `0x0115` | EMMG ⇔ MUX | stream_close_response |
| `0x0116` | EMMG ⇔ MUX | stream_error |
| `0x0117` | EMMG ⇔ MUX | stream_BW_request |
| `0x0118` | EMMG ⇔ MUX | stream_BW_allocation |
| `0x0119`–`0x0200` | DVB reserved | DVB reserved |
| `0x0201` | ECMG ⇔ SCS | CW_provision |
| `0x0202` | ECMG ⇔ SCS | ECM_response |
| `0x0203`–`0x0210` | DVB reserved | DVB reserved |
| `0x0211` | EMMG ⇔ MUX | data_provision |
| `0x0212`–`0x0300` | DVB reserved | DVB reserved |
| `0x0301` | C(P)SIG ⇔ (P)SIG | channel_setup |
| `0x0302` | C(P)SIG ⇔ (P)SIG | channel_status |
| `0x0303` | C(P)SIG ⇔ (P)SIG | channel_test |
| `0x0304` | C(P)SIG ⇔ (P)SIG | channel_close |
| `0x0305` | C(P)SIG ⇔ (P)SIG | channel_error |
| `0x0306`–`0x0310` | DVB reserved | DVB reserved |
| `0x0311` | C(P)SIG ⇔ (P)SIG | stream_setup |
| `0x0312` | C(P)SIG ⇔ (P)SIG | stream_status |
| `0x0313` | C(P)SIG ⇔ (P)SIG | stream_test |
| `0x0314` | C(P)SIG ⇔ (P)SIG | stream_close |
| `0x0315` | C(P)SIG ⇔ (P)SIG | stream_close_request |
| `0x0316` | C(P)SIG ⇔ (P)SIG | stream_close_response |
| `0x0317` | C(P)SIG ⇔ (P)SIG | stream_error |
| `0x0318` | C(P)SIG ⇔ (P)SIG | stream_service_change |
| `0x0319` | C(P)SIG ⇔ (P)SIG | stream_trigger_enable_request |
| `0x031A` | C(P)SIG ⇔ (P)SIG | stream_trigger_enable_response |
| `0x031B` | C(P)SIG ⇔ (P)SIG | trigger |
| `0x031C` | C(P)SIG ⇔ (P)SIG | table_request |
| `0x031D` | C(P)SIG ⇔ (P)SIG | table_response |
| `0x031E` | C(P)SIG ⇔ (P)SIG | descriptor_insert_request |
| `0x031F` | C(P)SIG ⇔ (P)SIG | descriptor_insert_response |
| `0x0320` | C(P)SIG ⇔ (P)SIG | PID_provision_request |
| `0x0321` | C(P)SIG ⇔ (P)SIG | PID_provision_response |
| `0x0322`–`0x0400` | DVB reserved | DVB reserved |
| `0x0401` | EIS ⇔ SCS | channel_set-up |
| `0x0402` | EIS ⇔ SCS | channel_test |
| `0x0403` | EIS ⇔ SCS | channel_status |
| `0x0404` | EIS ⇔ SCS | channel_close |
| `0x0405` | EIS ⇔ SCS | channel_error |
| `0x0406` | EIS ⇔ SCS | channel_reset |
| `0x0408` | EIS ⇔ SCS | SCG_provision |
| `0x0409` | EIS ⇔ SCS | SCG_test |
| `0x040A` | EIS ⇔ SCS | SCG_status |
| `0x040B` | EIS ⇔ SCS | SCG_error |
| `0x040C` | EIS ⇔ SCS | SCG_list_request |
| `0x040D` | EIS ⇔ SCS | SCG_list_response |
| `0x040E`–`0x0410` | DVB reserved | DVB reserved |
| `0x0411` | (P)SIG ⇔ MUX | channel_set_up |
| `0x0412` | (P)SIG ⇔ MUX | channel_test |
| `0x0413` | (P)SIG ⇔ MUX | channel_status |
| `0x0414` | (P)SIG ⇔ MUX | channel_close |
| `0x0415` | (P)SIG ⇔ MUX | channel_error |
| `0x0416`–`0x0420` | Reserved | Reserved |
| `0x0421` | (P)SIG ⇔ MUX | stream_setup |
| `0x0422` | (P)SIG ⇔ MUX | stream_test |
| `0x0423` | (P)SIG ⇔ MUX | stream_status |
| `0x0424` | (P)SIG ⇔ MUX | stream_close_request |
| `0x0425` | (P)SIG ⇔ MUX | stream_close_response |
| `0x0426` | (P)SIG ⇔ MUX | stream_error |
| `0x0427`–`0x0430` | DVB reserved | DVB reserved |
| `0x0431` | (P)SIG ⇔ MUX (carousel in MUX – CiM) | CiM_stream_section_provision |
| `0x0432` | (P)SIG ⇔ MUX (carousel in MUX – CiM) | CiM_channel_reset |
| `0x0433`–`0x040` | DVB reserved | DVB reserved ⚠ (see note below) |
| `0x0441` | (P)SIG ⇔ MUX (carousel in (P)SIG – CiP) | CiP_stream_BW_request |
| `0x0442` | (P)SIG ⇔ MUX (carousel in (P)SIG – CiP) | CiP_stream_BW_allocation |
| `0x0443` | (P)SIG ⇔ MUX (carousel in (P)SIG – CiP) | CiP_stream_data_provision |
| `0x0444`–`0x7FFF` | DVB reserved | DVB reserved |
| `0x8000`–`0xFFFF` | User defined | User defined |

> ⚠ p. 28 — Table 3 reserved range printed as `0x0433 to 0x040`. The lower
> bound is clearly `0x0433`; the upper bound digit string `0x040` is
> almost certainly a typo for **`0x0440`** (the gap between the CiM block
> ending `0x0432` and the CiP block starting `0x0441`). Candidates:
> `0x0440` (most likely) or as-printed `0x0040`. Owner re-check.

## Table 4 — Message Structure for XML-based protocols (§4.4.2, p. 29)

A separate framing used by the SIMF/XML transaction-based variant (only
available for C(P)SIG⇔(P)SIG and for control of the UDP-based EMMG/PDG⇔MUX
protocol, per §4.4.3). UTF-8 encoded.

| Syntax | Bytes | Type |
|--------|-------|------|
| `xml_message {` | | |
| &nbsp;&nbsp;message_length | 4 | uimsbf |
| &nbsp;&nbsp;message_data | * | Varies (XML) |
| `}` | | |

- **message_length** — total size in bytes from message_length through the end
  of message_data.
- **message_data** — an XML document of varying size.

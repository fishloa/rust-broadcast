# ECMG ⇔ SCS interface — message & parameter syntax

_Source: ETSI TS 103 197 V1.5.1 §5, Tables 5/6 + §5.4–5.6 message parameter tables (PDF pp. 31-39), render-verified_

The ECMG⇔SCS interface (clause 5) connects the **ECM Generator** (server) and
the **Simulcrypt Synchronizer** (client). The SCS opens one channel per TCP
connection to an ECMG (selected by `Super_CAS_id`), then opens ECM streams; it
sends `CW_provision` messages and receives `ECM_response` messages carrying the
computed ECM.

protocol_version = `0x03` (Table 2; `0x05` for DVB-H per annex N).

> SIGNALLING ONLY: the control words inside `CP_CW_combination` and
> `CW_encryption`, and the ECM inside `ECM_datagram`, are **opaque payloads** —
> carried/framed but not interpreted by this crate. The CW on this interface is
> the clear scrambling key (§5.7) — never decoded.

## message_type values (subset of Table 3, §4.4.1 pp. 27-28)

| message_type | message | direction |
|--------------|---------|-----------|
| `0x0001` | channel_setup | ECMG ⇐ SCS |
| `0x0002` | channel_test | ECMG ⇔ SCS |
| `0x0003` | channel_status | ECMG ⇔ SCS |
| `0x0004` | channel_close | ECMG ⇐ SCS |
| `0x0005` | channel_error | ECMG ⇔ SCS |
| `0x0101` | stream_setup | ECMG ⇐ SCS |
| `0x0102` | stream_test | ECMG ⇔ SCS |
| `0x0103` | stream_status | ECMG ⇔ SCS |
| `0x0104` | stream_close_request | ECMG ⇐ SCS |
| `0x0105` | stream_close_response | ECMG ⇒ SCS |
| `0x0106` | stream_error | ECMG ⇔ SCS |
| `0x0201` | CW_provision | ECMG ⇐ SCS |
| `0x0202` | ECM_response | ECMG ⇒ SCS |

(Direction arrows from the §5.4/§5.5 clause titles, pp. 34-38: `⇐` = toward
ECMG, `⇒` = toward SCS, `⇔` = either.)

## Table 5 — ECMG protocol parameter_type values (§5.2, p. 31)

| parameter_type | parameter | Type/units | Length (bytes) |
|----------------|-----------|------------|----------------|
| `0x0000` | DVB Reserved | - | - |
| `0x0001` | Super_CAS_id | uimsbf | 4 |
| `0x0002` | section_TSpkt_flag | uimsbf | 1 |
| `0x0003` | delay_start | tcimsbf/ms | 2 |
| `0x0004` | delay_stop | tcimsbf/ms | 2 |
| `0x0005` | transition_delay_start | tcimsbf/ms | 2 |
| `0x0006` | transition_delay_stop | tcimsbf/ms | 2 |
| `0x0007` | ECM_rep_period | uimsbf/ms | 2 |
| `0x0008` | max_streams | uimsbf | 2 |
| `0x0009` | min_CP_duration | uimsbf/n × 100ms | 2 |
| `0x000A` | lead_CW | uimsbf | 1 |
| `0x000B` | CW_per_msg | uimsbf | 1 |
| `0x000C` | max_comp_time | uimsbf/ms | 2 |
| `0x000D` | access_criteria | user defined | variable |
| `0x000E` | ECM_channel_id | uimsbf | 2 |
| `0x000F` | ECM_stream_id | uimsbf | 2 |
| `0x0010` | nominal_CP_duration | uimsbf/n × 100ms | 2 |
| `0x0011` | access_criteria_transfer_mode | Boolean | 1 |
| `0x0012` | CP_number | uimsbf | 2 |
| `0x0013` | CP_duration | uimsbf/n × 100ms | 2 |
| `0x0014` | CP_CW_combination | --- | variable |
| &nbsp;&nbsp;↳ CP | uimsbf | 2 | (sub-field of `0x0014`) |
| &nbsp;&nbsp;↳ CW | uimsbf | variable | (sub-field of `0x0014`) |
| `0x0015` | ECM_datagram | user defined | variable |
| `0x0016` | AC_delay_start | tcimsbf/ms | 2 |
| `0x0017` | AC_delay_stop | tcimsbf/ms | 2 |
| `0x0018` | CW_encryption | user defined | variable |
| `0x0019` | ECM_id | uimsbf | 2 |
| `0x001A`–`0x6FFF` | DVB reserved | - | - |
| `0x7000` | error_status | see clause 5.6 | 2 |
| `0x7001` | error_information | user defined | variable |
| `0x7002`–`0x7FFF` | DVB reserved | - | - |
| `0x8000`–`0xFFFF` | User defined | - | - |

`CP_CW_combination` (`0x0014`) is a compound value: a 2-byte CP (crypto-period
number) concatenated with a variable-length CW (control word). It is "typically
10 bytes long" (§5.3). The CW portion is opaque to this crate.

## Channel-specific messages (§5.4, pp. 34-35)

Each table lists parameters and the number of instances per message.

### channel_setup — `0x0001` — ECMG ⇐ SCS (Table, §5.4.1 p. 34)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| Super_CAS_id | 1 |

### channel_test — `0x0002` — ECMG ⇔ SCS (§5.4.2 p. 34)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |

### channel_status — `0x0003` — ECMG ⇔ SCS (§5.4.3 p. 34)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| section_TSpkt_flag | 1 |
| AC_delay_start | 0/1 |
| AC_delay_stop | 0/1 |
| delay_start | 1 |
| delay_stop | 1 |
| transition_delay_start | 0/1 |
| transition_delay_stop | 0/1 |
| ECM_rep_period | 1 |
| max_streams | 1 |
| min_CP_duration | 1 |
| lead_CW | 1 |
| CW_per_msg | 1 |
| max_comp_time | 1 |

The ECMG returns its operating parameters. A reply to channel_setup carries the
values requested by the ECMG (valid for the channel lifetime); a reply to
channel_test carries the values currently valid.

### channel_close — `0x0004` — ECMG ⇐ SCS (§5.4.4 p. 35)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |

### channel_error — `0x0005` — ECMG ⇔ SCS (§5.4.5 p. 35)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| error_status | 1 to n |
| error_information | 0 to n |

## Stream-specific messages (§5.5, pp. 35-38)

### stream_setup — `0x0101` — ECMG ⇐ SCS (§5.5.1 p. 35)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |
| ECM_id | 1 |
| nominal_CP_duration | 1 |

### stream_test — `0x0102` — ECMG ⇔ SCS (§5.5.2 p. 35)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |

### stream_status — `0x0103` — ECMG ⇔ SCS (§5.5.3 p. 35)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |
| ECM_id | 1 |
| access_criteria_transfer_mode | 1 |

### stream_close_request — `0x0104` — ECMG ⇐ SCS (§5.5.4 p. 36)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |

### stream_close_response — `0x0105` — ECMG ⇒ SCS (§5.5.5 p. 36)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |

### stream_error — `0x0106` — ECMG ⇔ SCS (§5.5.6 p. 36)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |
| error_status | 1 to n |
| error_information | 0 to n |

### CW_provision — `0x0201` — ECMG ⇐ SCS (§5.5.7 p. 36)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |
| CP_number | 1 |
| CW_encryption | 0 to 1 |
| CP_CW_combination | CW_per_msg |
| CP_duration | 0 to 1 |
| access_criteria | 0 to 1 |

The number of `CP_CW_combination` instances equals the `CW_per_msg` value
agreed at channel setup. Sent by SCS as a request to the ECMG to compute an
ECM. The SCS shall not send a new CW_provision before receiving the
ECM_response (or an error) for the previous crypto-period.

### ECM_response — `0x0202` — ECMG ⇒ SCS (§5.5.8 p. 38)

| Parameter | Instances |
|-----------|-----------|
| ECM_channel_id | 1 |
| ECM_stream_id | 1 |
| CP_number | 1 |
| ECM_datagram | 1 |

Reply to CW_provision, carrying the computed `ECM_datagram` (opaque). The
`CP_number` echoes the incoming CW_provision's. Time-out derives from
`max_comp_time` + network delay.

## Table 6 — ECMG protocol error values (error_status, §5.6, p. 39)

| error_status | Error type |
|--------------|-----------|
| `0x0000` | DVB Reserved |
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
| `0x0016`–`0x6FFF` | DVB Reserved |
| `0x7000` | unknown error |
| `0x7001` | unrecoverable error |
| `0x7002`–`0x7FFF` | DVB Reserved |
| `0x8000`–`0xFFFF` | ECMG specific / CA system specific / User defined |

`error_status` may appear 1..n times in a channel_error/stream_error message;
each may be accompanied by an optional `error_information` (user-defined, e.g.
ASCII text or a faulty parameter ID). "unrecoverable error" means the channel
or stream (per the message used) must be closed.

## Security (§5.7, p. 39)

The control words in `CP_CW_combination` are the clear scrambling keys. The
spec requires CW confidentiality on this interface — e.g. an inherently secure
network or the CW encryption scheme of annex D (`CW_encryption` parameter). This
crate does not decrypt; it carries `CW_encryption`/`CP_CW_combination` as opaque
bytes.

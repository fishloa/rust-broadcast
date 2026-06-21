# EMMG/PDG ⇔ MUX interface — message & parameter syntax

_Source: ETSI TS 103 197 V1.5.1 §6, Tables 7/8 + §6.2.4–6.2.6 message parameter tables + §6.3 UDP (PDF pp. 42-49), render-verified_

The EMMG/PDG⇔MUX interface (clause 6) connects the **EMM Generator / Private
Data Generator** (client) and the **Multiplexer** (server). The EMMG/PDG opens
a channel (identified by `client_id`), opens data streams, then sends
`data_provision` messages carrying EMMs / private data into the MUX.

protocol_version = `0x03` (Table 2; `0x05` for DVB-H per annex N). Both TCP
(§6.2) and UDP (§6.3) transports are defined; both use the same generic message
format. Under UDP, only `data_provision` is sent over UDP (§6.3.1); channel/
stream management still uses TCP (or SIMF).

> SIGNALLING ONLY: the EMM / private data inside the `datagram` parameter is an
> **opaque payload** — carried/framed, not interpreted by this crate.

## message_type values (subset of Table 3, §4.4.1 pp. 27-28)

| message_type | message | direction |
|--------------|---------|-----------|
| `0x0011` | channel_setup | EMMG/PDG ⇒ MUX |
| `0x0012` | channel_test | EMMG/PDG ⇔ MUX |
| `0x0013` | channel_status | EMMG/PDG ⇔ MUX |
| `0x0014` | channel_close | EMMG/PDG ⇒ MUX |
| `0x0015` | channel_error | EMMG/PDG ⇔ MUX |
| `0x0111` | stream_setup | EMMG/PDG ⇒ MUX |
| `0x0112` | stream_test | EMMG/PDG ⇔ MUX |
| `0x0113` | stream_status | EMMG/PDG ⇔ MUX |
| `0x0114` | stream_close_request | EMMG/PDG ⇒ MUX |
| `0x0115` | stream_close_response | EMMG/PDG ⇐ MUX |
| `0x0116` | stream_error | EMMG/PDG ⇔ MUX |
| `0x0117` | stream_BW_request | EMMG/PDG ⇒ MUX |
| `0x0118` | stream_BW_allocation | EMMG/PDG ⇐ MUX |
| `0x0211` | data_provision | EMMG/PDG ⇒ MUX |

(Directions from §6.2.4/§6.2.5 clause titles, pp. 43-46.)

## Table 7 — EMMG/PDG protocol parameter_type values (§6.2.2, p. 42)

| parameter_type | parameter | Type/units | Length (bytes) |
|----------------|-----------|------------|----------------|
| `0x0000` | DVB Reserved | - | - |
| `0x0001` | client_id | uimsbf | 4 |
| `0x0002` | section_TSpkt_flag | uimsbf | 1 |
| `0x0003` | data_channel_id | uimsbf | 2 |
| `0x0004` | data_stream_id | uimsbf | 2 |
| `0x0005` | datagram | user defined | variable |
| `0x0006` | bandwidth | uimsbf/kbit/s | 2 |
| `0x0007` | data_type | uimsbf | 1 |
| `0x0008` | data_id | uimsbf | 2 |
| `0x0009`–`0x6FFF` | DVB Reserved | - | - |
| `0x7000` | error_status | see clause 6.2.6 | 2 |
| `0x7001` | error_information | user defined | variable |
| `0x7002`–`0x7FFF` | DVB reserved | - | - |
| `0x8000`–`0xFFFF` | user defined | - | - |

### data_type values (§6.2.3, p. 42)

| data_type | meaning |
|-----------|---------|
| `0x00` | EMM |
| `0x01` | private data |
| `0x02` | DVB reserved (ECM) |
| other | DVB reserved |

### section_TSpkt_flag values (§6.2.3, p. 43)

| section_TSpkt_flag | meaning |
|--------------------|---------|
| `0x00` | EMMs / private datagrams in MPEG-2 section format |
| `0x01` | MPEG-2 TS packet format (all TS packets 188 bytes) |
| `0x02` | arbitrary-length EMMs/KMMs per IP Datacast SPP [26] (annex N) |
| other | DVB reserved |

## Channel-specific messages (§6.2.4, pp. 43-44)

### channel_setup — `0x0011` — EMMG/PDG ⇒ MUX (§6.2.4.1 p. 43)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| section_TSpkt_flag | 1 |

### channel_test — `0x0012` — EMMG/PDG ⇔ MUX (§6.2.4.2 p. 43)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |

### channel_status — `0x0013` — EMMG/PDG ⇔ MUX (§6.2.4.3 p. 43)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| section_TSpkt_flag | 1 |

### channel_close — `0x0014` — EMMG/PDG ⇒ MUX (§6.2.4.4 p. 44)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |

### channel_error — `0x0015` — EMMG/PDG ⇔ MUX (§6.2.4.5 p. 44)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| error_status | 1 to n |
| error_information | 0 to n |

## Stream-specific messages (§6.2.5, pp. 44-46)

### stream_setup — `0x0111` — EMMG/PDG ⇒ MUX (§6.2.5.1 p. 44)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |
| data_id | 1 |
| data_type | 1 |

### stream_test — `0x0112` — EMMG/PDG ⇔ MUX (§6.2.5.2 p. 44)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |

### stream_status — `0x0113` — EMMG/PDG ⇔ MUX (§6.2.5.3 p. 44)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |
| data_id | 1 |
| data_type | 1 |

### stream_close_request — `0x0114` — EMMG/PDG ⇒ MUX (§6.2.5.4 p. 45)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |

### stream_close_response — `0x0115` — EMMG/PDG ⇐ MUX (§6.2.5.5 p. 45)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |

### stream_error — `0x0116` — EMMG/PDG ⇔ MUX (§6.2.5.6 p. 45)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |
| error_status | 1 to n |
| error_information | 0 to n |

### stream_BW_request — `0x0117` — EMMG/PDG ⇒ MUX (§6.2.5.7 p. 45)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |
| bandwidth | 0 to 1 |

If `bandwidth` present → request that amount; if absent → query current
allocation. The MUX always replies with stream_BW_allocation.

### stream_BW_allocation — `0x0118` — EMMG/PDG ⇐ MUX (§6.2.5.8 p. 46)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 1 |
| data_stream_id | 1 |
| bandwidth | 0 to 1 |

If `bandwidth` absent → allocated bandwidth is not known. The allocated value
may differ (be less) than requested.

### data_provision — `0x0211` — EMMG/PDG ⇒ MUX (TCP form, §6.2.5.9 p. 46)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 0 to 1 |
| data_stream_id | 0 to 1 |
| data_id | 1 |
| datagram | 1 to n |

In the **TCP** protocol the message **shall include** `data_channel_id` and
`data_stream_id`. In the **UDP** protocol (§6.3) those two parameters **shall
not** be included. `datagram` carries the opaque EMM / private data.

### data_provision — `0x0211` — EMMG/PDG ⇒ MUX (UDP form, §6.3.1.1 p. 48)

| Parameter | Instances |
|-----------|-----------|
| client_id | 1 |
| data_channel_id | 0 to 1 |
| data_stream_id | 0 to 1 |
| data_id | 1 |
| datagram | 1 to n |

The only message sent over UDP/IP; may be broadcast to several multiplexers.
The `client_id`/`data_id` pair uniquely identifies the EMM/private data stream
across the system.

## Table 8 — EMMG/PDG protocol error values (error_status, §6.2.6, p. 47)

| error_status | Error type |
|--------------|-----------|
| `0x0000` | DVB Reserved |
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
| `0x0015`–`0x6FFF` | DVB Reserved |
| `0x7000` | unknown error |
| `0x7001` | unrecoverable error |
| `0x7002`–`0x7FFF` | DVB Reserved |
| `0x8000`–`0xFFFF` | MUX specific / CA system specific / User defined |

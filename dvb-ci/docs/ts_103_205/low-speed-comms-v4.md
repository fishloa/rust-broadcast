# Low Speed Communication resource version 4 (CI Plus)

_Source: ETSI TS 103 205 v1.4.1 §10, Tables 76-89 (PDF pp. 100-110), render-verified_

LSC version 4 extends LSC version 3 (CI Plus V1.3 [3] §14.1) to add: source-specific
multicast (IGMPv3); delivery of response data across the TS interface (hybrid
connections); a new `comms_info()` APDU (LSC session info Host→CICAM); a new
`comms_IP_config()` APDU (Host IP-adapter info → CICAM); and a `source_port` field
in the connection_descriptor. New APDU tags live in the CI Plus `0x9F8Cxx`
namespace.

⚠ No "LSC v4 resource summary" table (with a single resource_identifier value) is
printed in §10. The existing LSC resource identifier and the base `comms_*` tags
(`comms_cmd`, `comms_reply`, `comms_send`, `comms_rcv`) are defined in CI Plus
V1.3 [3] §14.1 (proprietary, not reproduced). §10 prints only the *new* v4 APDUs
and the device-type / descriptor extensions below.

## New v4 APDU tags

| APDU | Tag value | Direction | Source |
|------|-----------|-----------|--------|
| comms_info_req       | `0x9F8C07` | CICAM → Host | §10.9.2 Table 76 |
| comms_info_reply     | `0x9F8C08` | Host → CICAM | §10.9.3 Table 77 |
| comms_IP_config_req  | `0x9F8C09` | CICAM → Host | §10.10.2 Table 78 |
| comms_IP_config_reply| `0x9F8C0A` | Host → CICAM | §10.10.3 Table 79 |

⚠ Table 78 prints the tag as `9F8C09` (no `0x` prefix) and Table 79 as `9F8C0A`;
Tables 76/77 print `0x9F8C07` / `0x9F8C08`. Same hex namespace, transcribed as
rendered.

## §10.9.2 — comms_info_req APDU — Table 76 (PDF p. 102)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_info_req () {` | | |
| &nbsp;&nbsp;comms_info_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| `}` | | |

- **comms_info_req_tag** — `0x9F8C07`. (Note: `length_field() = 1` is as printed; the APDU carries no payload fields.)

## §10.9.3 — comms_info_reply APDU — Table 77 (PDF p. 102)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_info_reply () {` | | |
| &nbsp;&nbsp;comms_info_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 22 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;status | 1 | bslbf |
| &nbsp;&nbsp;source_IPaddress | 128 | uimsbf |
| &nbsp;&nbsp;source_port | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;inputDeliveryPID | 13 | uimsbf |
| `}` | | |

Field semantics (§10.9.3, p. 103):
- **comms_info_reply_tag** — `0x9F8C08`.
- **LTS_id** (8) — identifier of the Local TS.
- **status** (1) — `0b1` = a connection has been established; `0b0` = not valid (CICAM shall ignore fields following `status`).
- **source_IPaddress** (128) — IP source address used by the Host for this LSC session, in IPv6 format (IPv4 prefixed `::ffff:0:0/96` or `::0:0/96`). All zeros if not determinable.
- **source_port** (16) — source port for this LSC session; `0x0000` if not aware.
- **inputDeliveryPID** (13) — PID for delivery of the TCP/UDP payload across the TS interface for hybrid connections; range `0x0020`–`0x1FFE`. `0x0000` if not a hybrid connection.

## §10.10.2 — comms_IP_config_req APDU — Table 78 (PDF p. 103)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_IP_config_req () {` | | |
| &nbsp;&nbsp;comms_IP_config_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

- **comms_IP_config_req_tag** — `9F8C09`.

## §10.10.3 — comms_IP_config_reply APDU — Table 79 (PDF p. 104)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_IP_config_reply () {` | | |
| &nbsp;&nbsp;comms_IP_config_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;connection_state | 2 | uimsbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;physical_address | 48 | uimsbf |
| &nbsp;&nbsp;`if (connection_state == 01) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;IP_address | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;network_mask | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;default_gateway | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;DHCP_server_address | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;num_DNS_servers | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (int i=0; i<num_DNSservers; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;DNS_server_address | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§10.10.3, p. 104):
- **comms_IP_config_reply_tag** — `9F8C0A`.
- **connection_state** (2) — state of the IP adapter; see Table 80.
- **physical_address** (48) — MAC address for this IP network adapter.
- **IP_address / network_mask / default_gateway / DHCP_server_address / DNS_server_address** — all 128-bit, IPv6 format (IPv4 prefixed `::ffff:0:0/96` or `::0:0/96`). DHCP address all-zeros if no DHCP server.
- **num_DNS_servers** (8) — number of DNS servers.

### Table 80 — connection_state values (PDF p. 104)

| Connection_state | Type value | Description |
|------------------|------------|-------------|
| Disconnected | `0x00` | Network interface is inactive or disconnected |
| Connected | `0x01` | Network interface is active, connected and has a valid IP address |
| Reserved | `0x10`–`0x11` | |

⚠ Table 80 prints the Reserved range as `0x10-0x11` — given `connection_state` is
only 2 bits, this is an evident spec typo (likely intended `0x02`–`0x03`).
Transcribed as rendered; flagged for re-check.

## §10.11.1 — Connection descriptor APDU — Table 81 (PDF p. 105)

The connection_descriptor (CI Plus V1.3 [3] Table 14.5) is modified to add the
hybrid and source-specific-multicast descriptor types and a `source_port`. The
MSB of `connection_descriptor_type` is repurposed as `source_port_flag`
(backward-compatible with V1.3).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `connection_descriptor () {` | | |
| &nbsp;&nbsp;connection_descriptor_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;source_port_flag | 1 | bslbf |
| &nbsp;&nbsp;connection_descriptor_type | 7 | uimsbf |
| &nbsp;&nbsp;`if (source_port_flag == 0b1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;source_port | 16 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == SI_telephone_descriptor) { telephone_descriptor () }` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == cable_return_channel_descriptor) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;channel_id | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == IP_descriptor) { IP_descriptor () }` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == hostname_descriptor) { hostname_descriptor () }` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == hybrid_descriptor) { hybrid_descriptor () }` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == multicast_descriptor) { multicast_descriptor () }` | | |
| `}` | | |

- **source_port_flag** (1) — `0b1` signals a `source_port` is specified.
- **source_port** (16) — source port the CICAM requests; shall be one already in use by a concurrent LSC session (per `comms_info_reply()`).

### Table 82 — Connection descriptor type (PDF p. 105)

| connection_descriptor_type | Type value |
|----------------------------|------------|
| SI_telephone_descriptor | `0x01` |
| cable_return_channel_descriptor | `0x02` |
| IP_descriptor | `0x03` |
| hostname_descriptor | `0x04` |
| hybrid_descriptor | `0x05` |
| multicast_descriptor | `0x06` |
| reserved for future use | `0x07`–`0x7F` |

## §10.11.2 — Comms Cmd hybrid_descriptor — Table 83 (PDF p. 106)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `hybrid_descriptor() {` | | |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;IP_connection_type | 8 | uimsbf |
| &nbsp;&nbsp;`if (IP_connection_type == IP_descriptor) { IP_descriptor() }` | | |
| &nbsp;&nbsp;`if (IP_connection_type == multicast_descriptor) { multicast_descriptor() }` | | |
| &nbsp;&nbsp;`if (IP_connection_type == hostname_descriptor) { hostname_descriptor() }` | | |
| `}` | | |

Field semantics (§10.11.2, p. 106):
- **descriptor_tag** (8) — value `0x05` for the hybrid_descriptor.
- **descriptor_length** (8) — bytes of the data portion following this field.
- **LTS_id** (8) — Local TS on which the TCP/UDP payload shall be delivered (returned in `CICAM_player_start_reply()`).
- **IP_connection_type** (8) — type of hybrid connection; see Table 84.
- **IP_descriptor()** / **hostname_descriptor()** — same as CI Plus V1.3 [3] §14.2.1.1 / §14.2.1.2 (not reproduced).

### Table 84 — IP connection type (PDF p. 106)

| IP_connection_type | Type value |
|--------------------|------------|
| reserved | `0x01` |
| reserved | `0x02` |
| IP_descriptor | `0x03` |
| hostname_descriptor | `0x04` |
| reserved | `0x05` |
| multicast_descriptor | `0x06` |
| reserved for future use | `0x07`–`0xFF` |

## §10.11.3 — Comms Cmd multicast_descriptor — Table 85 (PDF p. 107)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `multicast_descriptor () {` | | |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;IP_protocol_version | 8 | uimsbf |
| &nbsp;&nbsp;IP_address | 128 | uimsbf |
| &nbsp;&nbsp;multicast_port | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;include_sources | 1 | bslbf |
| &nbsp;&nbsp;num_source_addresses | 8 | uimsbf |
| &nbsp;&nbsp;`for ( i=0; i<num_source_addresses; i++ ) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;source_address | 128 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§10.11.3, p. 107):
- **descriptor_tag** (8) — value `0x06`.
- **descriptor_length** (8) — bytes of data portion following this field.
- **IP_protocol_version** (8) — see Table 86.
- **IP_address** (128) — multicast service address. For IPv4 the first 12 bytes are `0x00`.
- **multicast_port** (16) — multicast port for the host.
- **include_sources** (1) — `0b1` = receive flows from the listed source addresses; `0b0` = receive from all sources except those listed. Only relevant when `num_source_addresses > 0`.
- **num_source_addresses** (8) — number of multicast source addresses. `0` = not source-specific (any source).
- **source_address** (128) — a source address for the multicast.

### Table 86 — IP protocol version (PDF p. 107)

| IP_protocol_version | Type value |
|---------------------|------------|
| reserved | `0x00` |
| IPv4 | `0x01` |
| IPv6 | `0x02` |
| reserved for future use | `0x03`–`0xFF` |

## §10.12 — LSC resource types modification

### Table 87 — Communications Device types (LSC v4) (PDF p. 108)

| Description | Value |
|-------------|-------|
| Modems | `0x00`–`0x3F` |
| Serial ports | `0x40`–`0x4F` |
| Cable return channel | `0x50` |
| reserved | `0x51`–`0x5F` |
| IP connection | `0x60` |
| reserved | `0x61`–`0x6F` |
| Hybrid connection | `0x70` |
| reserved | `0x71`–`0xFF` |

The device number shall be zero for IP connection and Hybrid connection types.

### Table 88 — comms reply return values (PDF p. 108)

| Description | Value |
|-------------|-------|
| OK | `0x00` |
| Reserved | `0x01`–`0x7F` |
| Private errors | `0x80`–`0xFD` |
| Connection protocol not supported | `0xFE` |
| Non-specific error | `0xFF` |

## §10.12.4.1 — TS packet syntax for hybrid data transfer — Table 89 (PDF p. 109)

When using a hybrid connection, data received by the Host is delivered across the
TS interface by encapsulating the TCP/UDP payload in TS packets.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `transport_packet () {` | | |
| &nbsp;&nbsp;sync_byte | 8 | bslbf |
| &nbsp;&nbsp;transport_error_indicator | 1 | bslbf |
| &nbsp;&nbsp;payload_unit_start_indicator | 1 | bslbf |
| &nbsp;&nbsp;transport_priority | 1 | bslbf |
| &nbsp;&nbsp;PID | 13 | uimsbf |
| &nbsp;&nbsp;transport_scrambling_control | 2 | bslbf |
| &nbsp;&nbsp;adaptation_field_control | 2 | bslbf |
| &nbsp;&nbsp;continuity_counter | 4 | uimsbf |
| &nbsp;&nbsp;`if(adaptation_field_control == '11') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptation_field() | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`for(i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;data_byte | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Fields are as MPEG-2 Systems [4] except (§10.12.4.1, p. 109):
- **sync_byte** (8) — used to identify the Local TS; set to the `LTS_id` allocated by the Host for the player session (NOT the usual `0x47`).
- **transport_error_indicator** — `0b0`.
- **payload_unit_start_indicator** — `1` for the TS packet containing the first byte of UDP payload; `0` otherwise.
- **transport_priority** — `0b0`.
- **PID** (13) — assigned by the Host so it is unique within the Local TS (each hybrid LSC session uses a different PID).
- **transport_scrambling_control** — `0b00`.
- **data_byte** — the payload of the TCP or UDP packet.

§10.12.4.2 (Adaptation field usage): the adaptation field is used only to stuff TS
packets when the last section of IP datagram data does not fill the packet
(`adaptation_field_length` set to the remaining byte count, all flags `0b0`,
`stuffing_byte` = `0xFF`). One TS packet shall contain bytes from only one UDP
packet.

# Low-Speed Communications resource (comms_cmd / connection_descriptor / comms_reply / comms_send / comms_rcv)

_Source: EN 50221 §8.7.1, Tables 52-56 + value tables (PDF pp. 50-54), render-verified_

The Low-Speed Communication resource class (resource identifier `0060xxx1`, see
Table 57 and Figure 14) provides a bi-directional (full-duplex) channel over, for
example, a telephone line or cable network return channel. Data is split into
segments for transfer; flow control is applied in both directions. Buffers of up to
**254 bytes** shall be accepted in both directions (EN 50221 §8.7.1.2, p. 50).

apdu_tag values (cross-ref Table 58, `apdu-tag-values.md`):

| apdu_tag | tag value | Direction (host <-> app) |
|----------|-----------|--------------------------|
| Tcomms_cmd             | `9F 8C 00` | `<---` |
| Tconnection_descriptor | `9F 8C 01` | `<---` |
| Tcomms_reply           | `9F 8C 02` | `--->` |
| Tcomms_send_last       | `9F 8C 03` | `<---` |
| Tcomms_send_more       | `9F 8C 04` | `<---` |
| Tcomms_rcv_last        | `9F 8C 05` | `--->` |
| Tcomms_rcv_more        | `9F 8C 06` | `--->` |

**Transcription note on object count.** §8.7.1.3 (p. 51) states that *four* objects
are defined — Comms Cmd, Comms Reply, Comms Send and Comms Rcv (plus the Connection
Descriptor component carried inside Comms Cmd). The spec gives one syntax table each
for `comms_send` (Table 55) and `comms_rcv` (Table 56). The `..._last` / `..._more`
tag pairs in Table 58 (`9F 8C 03/04` for send, `9F 8C 05/06` for receive) are the
APDU-chaining `L_apdu_tag` / `M_apdu_tag` variants of those same two object bodies —
the last block vs. more-to-follow framing of the APDU chaining mechanism (see
`apdu-coding.md`). The send/receive object body is identical for the `_last` and
`_more` tag; only the tag (and hence "more to follow") differs.

## Table 52 — Comms Cmd object coding (comms_cmd)

apdu_tag `Tcomms_cmd` = `9F 8C 00`, Direction `<---` (EN 50221 §8.7.1.4, Table 52, PDF p. 51).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_cmd() {` | | |
| &nbsp;&nbsp;comms_cmd_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;comms_command_id | 8 | uimsbf |
| &nbsp;&nbsp;`if (comms_command_id == Connect_on_Channel) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;connection_descriptor() | | |
| &nbsp;&nbsp;&nbsp;&nbsp;retry_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;timeout | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (comms_command_id == Set_Params) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;buffer_size | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;timeout | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (comms_command_id == Get_Next_Buffer) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;comms_phase_id | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

### comms_command_id values (EN 50221 §8.7.1.4, Table p. 52)

| comms_command_id | id value |
|------------------|----------|
| Connect_on_Channel    | `01` |
| Disconnect_on_Channel | `02` |
| Set_Params            | `03` |
| Enquire_Status        | `04` |
| Get_Next_Buffer       | `05` |
| reserved              | other values |

- `Connect_on_Channel` — establishes communication on the comms resource. The
  `connection_descriptor` carries the information needed to establish the connection
  (e.g. the telephone number to dial). `retry_count` allows one or more retries to be
  attempted. `timeout` (in seconds) aborts a connection attempt if no positive state
  indication is received in time; a timeout of zero means wait indefinitely.
- `Disconnect_on_Channel` — terminates the connection on the comms resource.
- `Set_Params` — `buffer_size` is the maximum buffer size in bytes (min 1, max 254);
  `timeout` is an input time-out in units of 10 ms (if at least one byte has been
  received and a gap of `timeout` elapses, the buffer is given to the application as a
  Comms Rcv object; if the buffer fills to `buffer_size` with no timeout it is
  returned immediately).
- `Enquire_Status` — no parameters; generates a Comms Reply with a parameter giving
  the current connection status.
- `Get_Next_Buffer` — operates receive-side flow control; `comms_phase_id` alternates
  0,1,0,1… and must be 0 for the first one (the host enforces this). Issuing it elicits
  an immediate Comms Reply acknowledgement; the received buffer is sent later via the
  Comms Rcv object.

## Table 53 — Connection Descriptor object coding (connection_descriptor)

apdu_tag `Tconnection_descriptor` = `9F 8C 01`, Direction `<---` (EN 50221 §8.7.1.4, Table 53, PDF p. 51).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `connection_descriptor() {` | | |
| &nbsp;&nbsp;connection_descriptor_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;connection_descriptor_type | 8 | uimsbf |
| &nbsp;&nbsp;`if (connection_descriptor_type == SI_Telephone_Descriptor) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;telephone_descriptor()&nbsp;&nbsp;/* see DVB/SI specification [4] */ | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (connection_descriptor_type == Cable_Return_Channel_Descriptor) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;channel_id | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

### connection_descriptor_type values (EN 50221 §8.7.1.4, Table p. 52)

| connection_descriptor_type | type value |
|----------------------------|------------|
| SI_Telephone_Descriptor          | `01` |
| Cable_Return_Channel_Descriptor  | `02` |
| reserved                         | other values |

- `telephone_descriptor()` — the SI telephone descriptor defined by reference [4]
  (DVB/SI); its internal layout is not reproduced in EN 50221.
- `channel_id` — selects the cable return channel.

## Table 54 — Comms Reply object coding (comms_reply)

apdu_tag `Tcomms_reply` = `9F 8C 02`, Direction `--->` (EN 50221 §8.7.1.5, Table 54, PDF p. 52).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_reply() {` | | |
| &nbsp;&nbsp;comms_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;comms_reply_id | 8 | uimsbf |
| &nbsp;&nbsp;return_value | 8 | uimsbf |
| `}` | | |

### comms_reply_id values (EN 50221 §8.7.1.5, Table p. 53)

| comms_reply_id | id value |
|----------------|----------|
| Connect_Ack         | `01` |
| Disconnect_Ack      | `02` |
| Set_Params_Ack      | `03` |
| Status_Reply        | `04` |
| Get_Next_Buffer_Ack | `05` |
| Send_Ack            | `06` |
| reserved            | other values |

- `return_value` — in general positive values are OK and negative values are errors.
  Zero is the standard OK return value and `-1` is the non-specific error. For
  `Status_Reply`: 0 = Disconnected, 1 = Connected. For `Send_Ack` the return value
  tells which buffer was successfully sent (alternating phase 0 / 1, matching the
  `comms_phase_id` of the acknowledged Comms Send). A Comms Reply can be sent
  unsolicited on an error; the only error currently signalled is a disconnection
  (`comms_reply_id` = Status_Reply, `return_value` = 0).

## Table 55 — Comms Send object coding (comms_send)

apdu_tag pair: `Tcomms_send_last` = `9F 8C 03` / `Tcomms_send_more` = `9F 8C 04`,
Direction `<---` (EN 50221 §8.7.1.6, Table 55, PDF p. 53).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_send() {` | | |
| &nbsp;&nbsp;comms_send_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;comms_phase_id | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0;i<n;i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;message_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- `comms_phase_id` — takes the values 0 or 1. The first Comms Send after a Connect
  Comms Cmd must set it to 0; subsequent Comms Sends alternate 1,0,1,0… The host
  checks this and returns a Comms Reply with a Send_Ack error if the sequence is
  broken. The two-phase scheme lets the host keep the comms continuously fed at
  maximum speed.
- The maximum number of `message_byte`s is 254.

## Table 56 — Comms Rcv object coding (comms_rcv)

apdu_tag pair: `Tcomms_rcv_last` = `9F 8C 05` / `Tcomms_rcv_more` = `9F 8C 06`,
Direction `--->` (EN 50221 §8.7.1.7, Table 56, PDF p. 54).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `comms_rcv() {` | | |
| &nbsp;&nbsp;comms_rcv_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;comms_phase_id | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0;i<n;i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;message_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- `comms_phase_id` — indicates which phase of the Get_Next_Buffer cycle this data
  belongs to.

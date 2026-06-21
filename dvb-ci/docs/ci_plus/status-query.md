# Status Query resource

_Source: ETSI TS 101 699 V1.1.1 §6.2, Tables 35–51 (PDF pp. 42–50), render-verified_

The Status Query resource allows modules to interrogate the status of the host
(§6.2). It is a Module-ID-derived resource: one resource instance is created per
Module ID, with the Module ID placed in the `resource_instance` field
(resource_class = Status Query, `resource_type` = 1, `resource_version` = 1).
The resource_identifier is therefore `0x00211ii1` (type 1\*, where `ii` = Module
ID) — e.g. `0x00211041` for module_id 1, `0x00211081` for module_id 2.

The host provides a StatusQuery resource supporting a session on each
host-module transport connection. Some of the information (Audience metering)
is private to the consumer; the host must ensure the consumer is aware of and
authorizes data collection, using a module-specific authenticated session
(§6.2.1, §6.2.3.1).

## Table 35 — Messages of the status query resource (p. 42)

| Message          | Direction (see note)  | Description |
|------------------|-----------------------|-------------|
| StatusQuery (N)  | M → H | Requests the host to return the status of status item N. |
| Trap (N)         | M → H | Requests the host to return the status of status item N whenever its value changes. |
| GetNextItemReq   | M → H | The dialogue supported by these calls can be used by the module to explore the set of status items that the host supports. |
| GetNextItemAck   | H → M | |
| StatusAck        | H → M | Returns the status of requested status item as a variable length array of bytes. The format of these bytes will depend on the status item. |

NOTE: M = module resident process, H = host's status query resource. M → H means from module to host.

## Table 36 — List of status items that can be interrogated (p. 42)

| Status Item Number | Name | Description |
|--------------------|------|-------------|
| 0 | Reserved | |
| 1 | Selection Information | Used to provide Audience Metering Information by describing the inputs and outputs of the host. See "Selection information". |
| 2 | Port Profile | Also used in Audience Metering, provides a description of the various host ports. See "Port profile". |
| 3 | Viewed Service | Used to allow an auxiliary decoder (e.g. Audio Description) to track the service being viewed on the host. See "Port profile". |
| 4 | Activation Status | Describes the power status of the host to the module. See "Activation status". |

## Table 37 — StatusQueryReq syntax (§6.2.2.1, p. 43)

apdu_tag `StatusQueryReqTag` = `0x9F 80 00`, Direction M → H (module → host).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `StatusQueryReq() {` | | |
| &nbsp;&nbsp;StatusQueryReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;StatusItem | 32 | uimsbf |
| `}` | | |

Field notes:
- `StatusQueryReqTag` — this 24 bit field with value `0x9F8000` identifies this message.
- `StatusItem` — this 32 bit unsigned integer identifies the status item queried. The allowed values, and their definitions are listed in Table 36.

## Table 38 — TrapReq syntax (§6.2.2.2, p. 43)

apdu_tag `TrapReqTag` = `0x9F 80 01`, Direction M → H (module → host).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `TrapReq() {` | | |
| &nbsp;&nbsp;TrapReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;StatusItem | 32 | uimsbf |
| `}` | | |

Field notes:
- `TrapReqTag` — this 24 bit field with value `0x9F8001` identifies this message.
- `StatusItem` — this 32 bit unsigned integer identifies the status item to be monitored. The allowed values, and their definitions are listed in Table 36.

## Table 39 — GetNextItemReq syntax (§6.2.2.3, p. 43)

apdu_tag `GetNextItemReqTag` = `0x9F 80 02`, Direction M → H (module → host).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `GetNextItemReq() {` | | |
| &nbsp;&nbsp;GetNextItemReqTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;StartStatusItem | 32 | uimsbf |
| `}` | | |

Field notes:
- `GetNextItemReqTag` — this 24 bit field with value `0x9F8002` identifies this message.
- `StartStatusItem` — this 32 bit unsigned integer identifies a start point for a search through the set of supported status items. This value is not required to be one of the status items supported by the host. Typically a module will use the value zero when starting a search.

## Table 40 — GetNextItemAck syntax (§6.2.2.4, p. 44)

apdu_tag `GetNextItemAckTag` = `0x9F 80 03`, Direction H → M (host → module).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `GetNextItemAck() {` | | |
| &nbsp;&nbsp;GetNextItemAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;NextStatusItem | 32 | uimsbf |
| `}` | | |

Field notes:
- `GetNextItemAckTag` — this 24 bit field with value `0x9F8003` identifies this message.
- `NextStatusItem` — this 32 bit unsigned integer identifies status item number of the first supported status item greater than the StartStatusItem specified in the request. The value 0 is returned if StartStatusItem is greater than or equal to the status item number of the highest numbered item supported by the host.

## Table 41 — StatusAck syntax (§6.2.2.5, p. 44)

apdu_tag `StatusAckTag` = `0x9F 80 04`, Direction H → M (host → module).

> Note: the spec's Table 41 heading reads "DeliverySystemInfoReq syntax" — an apparent editorial error; the body and field notes describe the StatusAck object.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `StatusAck () {` | | |
| &nbsp;&nbsp;StatusAckTag | 24 | bslbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;StatusItem | 32 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;StatusBytes | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `StatusAckTag` — this 24 bit field with value `0x9F8004` identifies this message.
- `StatusItem` — this 32 bit unsigned integer is the StatusItem value from the StatusQuery or Trap request that lead to this reply.
- `StatusBytes` — this set of bytes conveys the status information corresponding to the StatusItem. The coding of this information will depend on the status item interrogated. If the host does not support the status item requested there shall be an immediate reply with no status byte information.

## Table 42 — List of status item byte formats (p. 45)

| Status Item Number | Definition of status bytes |
|--------------------|----------------------------|
| 0 | None allowed |
| 1 | See Table 43, "Selection information status data" |
| 2 | See Table 48, "Port profile status data" |
| 3 | See Table 49, "Viewed service status data" |
| 4 | See Table 50, "Activation status data" |

## §6.2.3 Audience metering

To support Audience Metering for the purpose of market analysis, hosts support
the following status items: selection information; port profile (§6.2.3).

### Table 43 — Selection information status data (§6.2.3.2, p. 46)

Status item 1. A list of descriptions of signal sources with their associated
destinations.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| time | 40 | bslbf |
| `while (there is data in the object) {` | | |
| &nbsp;&nbsp;in_port_id | 8 | bslbf |
| &nbsp;&nbsp;length_in_signal_desc | 8 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < length_in_signal_desc; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;in_signal_desc | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;length_outputs | 12 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < length_outputs; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;out_port_id | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;length_out_signal_desc | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < length_out_signal_desc; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;out_signal_desc | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `time` — time encoded as in the UTC_time field of the DVB SI Time and Date Table.
- `in_port_id` — identifier of the source of a signal (see Table 44).
- `length_in_signal_desc` — the number of bytes in the description of the input signal.
- `in_signal_desc` — a block of bytes describing the input signal. The format of this block depends on the input port (see Table 45).
- `reserved` — this 4 bit field should be set to '0'.
- `length_outputs` — the number of bytes in the description of the output signal(s).
- `out_port_id` — identifier of the destination of the signal (see Table 46).
- `length_out_signal_desc` — the number of bytes in the description of the input signal.
- `out_signal_desc` — a block of bytes describing the output signal. The format of this block depends on the output port (see Table 47).

### Table 44 — In port values (p. 46)

| in_port_id | description |
|------------|-------------|
| 0 - 7     | RF Modulated digital source 0 to 7 |
| 8 - 15    | IEEE 1394 [9] port 0 to 7 |
| 16 - 23   | SCART port 0 to 7 |
| 24 - 31   | CI input module sources 0 to 7 |
| 32 - 126  | Reserved for future use |
| 127       | No source |
| 128 - 255 | Manufacturer specific ports |

### Table 45 — In signal description blocks (p. 47)

| in_port_id | signal source description | no bits | mnemonic |
|------------|---------------------------|---------|----------|
| 0 - 7   | DVB SI style delivery system description: | | |
|         | &nbsp;&nbsp;original_network_id | 16 | uimsbf |
|         | &nbsp;&nbsp;network_id | 16 | uimsbf |
|         | &nbsp;&nbsp;transport_stream_id | 16 | uimsbf |
|         | &nbsp;&nbsp;service_id | 16 | uimsbf |
|         | &nbsp;&nbsp;Video component tag (0 x FF if not found) | 8 | uimsbf |
|         | &nbsp;&nbsp;Audio component tag (0 x FF if not found) | 8 | uimsbf |
| 8 - 15  | \<TBD\> | | |
| 16 - 23 | Empty | | |
| 24 - 31 | CI input module sources 0 to 7. | | |
|         | When type 'A': | | |
|         | &nbsp;&nbsp;TuningInformationMessage | 11 x 8 | bslbf |
|         | &nbsp;&nbsp;service_id | 16 | uimsbf |
|         | &nbsp;&nbsp;Video component tag (0 x FF if not found) | 8 | uimsbf |
|         | &nbsp;&nbsp;Audio component tag (0 x FF if not found) | 8 | uimsbf |
|         | When type 'B': | | |
|         | &nbsp;&nbsp;original_network_id | 16 | uimsbf |
|         | &nbsp;&nbsp;service_id | 16 | uimsbf |
|         | &nbsp;&nbsp;Video component tag (0 x FF if not found) | 8 | uimsbf |
|         | &nbsp;&nbsp;Audio component tag (0 x FF if not found) | 8 | uimsbf |
| 32 - 126  | Reserved for future use | | |
| 127       | | | |
| 128 - 255 | Manufacturer specific string of bytes | | |

### Table 46 — Out port values (p. 47)

| out_port_id | description |
|-------------|-------------|
| 0 - 7     | Display 0 to 7 |
| 8 - 15    | IEEE 1394 [9] port 0 to 7 |
| 16 - 23   | SCART port 0 to 7 |
| 24 - 31   | RF Modulator 0 to 7 |
| 32 - 126  | Reserved for future use |
| 127       | No output |
| 128 - 255 | Manufacturer specific ports |

### Table 47 — Out signal description blocks (p. 48)

| out_port_id | signal destination description | no bits | mnemonic |
|-------------|--------------------------------|---------|----------|
| 0 - 7   | Visibility measure | 8 | bslbf |
|         | &nbsp;&nbsp;0 → obscured | | |
|         | &nbsp;&nbsp;1 → partially obscured | | |
|         | &nbsp;&nbsp;2 → fully visible | | |
|         | &nbsp;&nbsp;\> 2 reserved | | |
| 8 - 15  | \<TBD\> | | |
| 16 - 23 | Empty | | |
| 24 - 127  | Reserved for future use | | |
| 128 - 255 | Manufacturer specific string of bytes | | |

### Table 48 — Port profile status data (§6.2.3.3, p. 48)

Status item 2. Provides a textual definition of the host and each input and
output port.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| receiver_identification_length | 8 | uimsbf |
| `for (i = 0; i < receiver_identification_length; i++){` | | |
| &nbsp;&nbsp;receiver_identification_char | 8 | uimsbf |
| `}` | | |
| `for (j = 0; j < N; j++){` | | |
| &nbsp;&nbsp;in_port_id | 8 | bslbf |
| &nbsp;&nbsp;length_in_port_desc | 8 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < length_in_port_desc; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;in_port_desc | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;out_port_id | 8 | bslbf |
| &nbsp;&nbsp;length_out_port_desc | 8 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < length_out_port_desc; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;out_signal_desc | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `receiver_identification_length` — length of the following string in bytes.
- `receiver_identification_char` — a string of characters (coded according to annex A of DVB SI) uniquely describing the receiver manufacturer, model and version.
- `in_port_id` — see Table 44.
- `length_in_port_desc` — length of the following string in bytes.
- `in_port_desc` — a string of characters (coded according to annex A of DVB SI) describing the input port.
- `out_port_id` — see Table 46.
- `length_out_port_desc` — length of the following string in bytes.
- `out_signal_desc` — a string of characters (coded according to annex A of DVB SI) describing the output port.
- NOTE 1: If an input port type defines more than one form of coding of the signal description in Table 45 then the port description shall identify the particular form of the encoding used (e.g. a CI input module distinguishing type 'A' vs type 'B').
- NOTE 2: If an output port type defines more than one form of coding of the signal description in Table 47 then the port description shall identify the particular form of the encoding used.

### Table 49 — Viewed service status data (§6.2.3.4 Auxiliary decoder, p. 49)

Status item 3. Indicates the program or components selected by the consumer to
be most significant on the display of the host.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| service_id | 16 | bslbf |
| number_components | 8 | uimsbf |
| `for (i = 0; i < number_components; i++) {` | | |
| &nbsp;&nbsp;component_tag | 8 | uimsbf |
| `}` | | |

Field notes:
- `service_id` — corresponds to the service_id/program_number of the program currently selected for display by the host. The program number `0x0000` should be used to indicate that the source of the signal applied to the display is not in the Transport Stream available to the module (e.g. an analogue VCR connected to a SCART interface).
- `number_components` — number of components tags that follow.
- `component_tag` — the component tag of the component currently selected for decoding by the consumer, if a component tag is provided for this component in the PMT by a stream identifier descriptor.

## §6.2.4 Activation status

The Activation status data describes the power status of the host. Status item 4.

### Table 50 — Activation status data (p. 50)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| reserved | 4 | bslbf |
| event_activated | 1 | bslbf |
| activation_state | 3 | bslbf |

Field notes:
- `reserved` — these 4 bits are reserved for future use and shall be set to '0'.
- `event_activated` — this 1 bit field, when set to '1', indicates that the host was activated by an event from the event manager (see "Event Management") rather than by user action (which is indicated when this bit is set to '0'). When the host has been event activated it is likely that a user is available to respond to dialogues generated by the module.
- `activation_state` — this value identifies the power-up state of the host (see Table 51).

### Table 51 — Activation state status values (p. 50)

| activation state | current power mode |
|------------------|--------------------|
| 0     | Reserved |
| 1     | Standby-active (note 1) |
| 2     | On (note 2) |
| 2 - 7 | Reserved for future use |

NOTE 1: Corresponds to the EACEM defined power mode "Standby-active".
NOTE 2: Corresponds to the EACEM defined power modes "On (play)" and "On (record)".

> Note: the value-range overlap (row "2" and row "2 - 7") is transcribed exactly as printed in the spec table.

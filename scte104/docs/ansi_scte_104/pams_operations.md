# ANSI/SCTE 104 2023 — §10 PAMS to the Automation System Communications (machine transcription)

_Source: ANSI/SCTE 104 2023 (specs/ansi_scte_104_2023.pdf), pp. 69-81 (PDF pages 69-81
printed page numbers), transcribed locally via `pdf2md` (text-layer engine, `--report`
verification against the PDF's embedded text layer, exit code 0 — every `0x…` token in
the tables below matched the source text layer byte-for-byte). This file covers the
eight Table 8-3 `opID`s (`0x0009`-`0x0012`, minus the reserved `0x000D`-`0x000E` range)
whose data-structure layouts live in §10, outside the pp. 26-66 range already
transcribed in `operations.md`.

ANSI/SCTE 104 2023

## 10.4.1. config_request message AS ==> PAMS

Table 10-1: config_request_data

| Syntax | Bytes | Type |
|---|---|---|
| config_request_data(){ |  |  |
| AS_IP_address | 4 | uimsbf |
| AS_socket_number | 2 | uimsbf |
| activeflag | 1 | uimsbf |
| protocol_version | 1 | uimsbf |
| last_AS_index | 1 | uimsbf |
| last_injectorcount | 2 | uimsbf |
| permanent_connection_requested | 1 | uimsbf |
| } |  |  |

Total: 12 bytes.

### 10.4.1.1. Semantics

- **AS_IP_address** – IP address of the Automation System. Zero in a bi-directional
  serial communications architecture.
- **AS_socket_number** – TCP port of the Automation System. Zero in a bi-directional
  serial communications architecture.
- **activeflag** – Boolean; zero = backup AS, non-zero = primary AS.
- **protocol_version** – 8-bit unsigned integer, shall be `0x00`.
- **last_AS_index** – `AS_index` from a previous system initialization (1-255), or
  zero if unused / first initialization.
- **last_injectorcount** – `injectorcount` last provided by the PAMS; zero on first
  initialization.
- **permanent_connection_requested** – Non-zero requests a permanent TCP/IP link.

## 10.4.2. config_response message PAMS ==> AS

Table 10-2: config_response_data

| Syntax | Bytes | Type |
|---|---|---|
| config_response_data(){ |  |  |
| AS_index | 1 | uimsbf |
| permanent_connection_requested | 1 | uimsbf |
| } |  |  |

Total: 2 bytes.

### 10.4.2.1. Semantics

- **AS_index** – Index provided by the PAMS (0-255); see §8.2.1.
- **permanent_connection_requested** – Non-zero: a permanent TCP/IP link has been
  provisioned on the PAMS side.

## 10.5.1. provisioning_request message PAMS ==> AS

Table 10-3: provisioning_request_data

| Syntax | Bytes | Type |
|---|---|---|
| provisioning_request_data(){ |  |  |
| service_count | 1 | uimsbf |
| for (i=0; i<service_count; i++) { |  |  |
| &nbsp;&nbsp;injector_IP_address | 4 | uimsbf |
| &nbsp;&nbsp;injector_socket_number | 2 | uimsbf |
| &nbsp;&nbsp;service_name | 32 | bslbf |
| &nbsp;&nbsp;number_of_DPI_PIDs | 1 | uimsbf |
| &nbsp;&nbsp;for (i=0; i<number_of_DPI_PIDs; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;DPI_PID_index | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;shared_PID | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;event_id_compliance_flag | 1 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;component_mode | 1 | uimsbf |
| &nbsp;&nbsp;if (component_mode != 0){ |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;injector_component_list() | * | Varies |
| &nbsp;&nbsp;} |  |  |
| } |  |  |
| } |  |  |

### 10.5.1.1. Semantics

- **service_count** – number of services in the following loop.
- **injector_IP_address** – 32-bit IP address; zero if not using TCP/IP.
- **injector_socket_number** – 16-bit socket number; zero if not using TCP/IP.
- **service_name** – case-sensitive string, NUL-terminated, fixed 32-byte field.
- **number_of_DPI_PIDs** – count of DPI PIDs provisioned (>= 1).
- **DPI_PID_index** – PID index for a specific DPI service (§8.2.1).
- **shared_PID** – zero = unique `DPI_PID_index`; one = intentionally duplicated
  (e.g. multi-language audio sharing one video).
- **event_id_compliance_flag** – one = all `splice_event_id` values comply with the
  SCTE 35 "Constraints on Event Id" section.
- **component_mode** – acts as the presence flag for `injector_component_list()`
  in the syntax table's `if (component_mode != 0)` condition; the prose additionally
  (and inconsistently with the syntax table) glosses it as "Length of the
  injector_services_list()" — the syntax table's condition is what the wire format
  follows here.
- **injector_component_list()** – present only if `component_mode != 0`; see
  Table 10-9 below.

NOTE: §10.5.1.1's prose also defines `cue_stream_type` ("Identifies the type of cue
stream. The values are taken from Table 6-3 of SCTE 35") but **no such field appears
in Table 10-3's syntax** — confirmed by two independent renders (`pdftotext -layout`
and `pdf2md` text-layer, which agree byte-for-byte and both omit it from the syntax
rows). This is a spec-internal inconsistency (prose describes a field the syntax
table doesn't carry), not a transcription drop; the implementation follows the syntax
table (the wire truth) and does not carry a `cue_stream_type` field.

## 10.5.2. provisioning_response message AS ==> PAMS

Table 10-4: provisioning_response_data

| Syntax | Bytes | Type |
|---|---|---|
| provisioning_response_data(){ |  |  |
| } |  |  |

Empty body (0 bytes). Contains no data; may carry a result code in the wrapping
`single_operation_message()`.

## 10.6.1. fault_request message AS ==> PAMS

Table 10-5: fault_request_data

| Syntax | Bytes | Type |
|---|---|---|
| fault_request_data(){ |  |  |
| injector_IP_address | 4 | uimsbf |
| injector_socket_number | 2 | uimsbf |
| injector_service_name | 32 | bslbf |
| DPI_PID_index | 2 | uimsbf |
| } |  |  |

Total: 40 bytes.

### 10.6.1.1. Semantics

- **injector_IP_address** – 32-bit IP address; zero if not using TCP/IP.
- **injector_socket_number** – 16-bit socket number; zero if not using TCP/IP.
- **injector_service_name** – NUL-terminated string, fixed 32-byte field; must match
  the `service_name` sent by the PAMS in `provisioning_request_data()`.
- **DPI_PID_index** – PID index of the specific DPI service that appears to have
  failed; may be zero if the other three fields unambiguously identify the Injector.

## 10.6.2. fault_response message PAMS ==> AS

Table 10-6: fault_response_data

| Syntax | Bytes | Type |
|---|---|---|
| fault_response_data(){ |  |  |
| } |  |  |

Empty body (0 bytes).

## 10.7.1. AS_alive_request PAMS ==> AS

Table 10-7: AS_alive_request_data

| Syntax | Bytes | Type |
|---|---|---|
| AS_alive_request_data(){ |  |  |
| } |  |  |

Empty body (0 bytes). Sent if there has been no activity on the connection for the
preceding 60 seconds.

## 10.7.2. AS_alive_response AS ==> PAMS

Table 10-8: AS_alive_response_data

| Syntax | Bytes | Type |
|---|---|---|
| AS_alive_response_data(){ |  |  |
| } |  |  |

Empty body (0 bytes).

## 10.8.1. injector_component_list() Definition

Table 10-9: injector_component_list()

| Syntax | Bytes | Type |
|---|---|---|
| injector_component_list { |  |  |
| video_component_tag | 1 | uimsbf |
| number_of_audio_component_tags | 1 | uimsbf |
| for (i=0; i<number_of_audio_component_tags; i++) { |  |  |
| &nbsp;&nbsp;audio_component_tag | 1 | uimsbf |
| } |  |  |
| number_of_data_component_tags | 1 | uimsbf |
| for (i=0; i<number_of_data_component_tags; i++) { |  |  |
| &nbsp;&nbsp;data_component_tag | 1 | uimsbf |
| } |  |  |
| } |  |  |

### 10.8.1.1. Semantics

- **video_component_tag** – `component_tag` value of the video stream.
- **number_of_audio_component_tags** – count of audio `component_tag`s that follow.
- **audio_component_tag** – `component_tag` value of each specific audio stream.
- **number_of_data_component_tags** – count of data `component_tag`s that follow.
- **data_component_tag** – `component_tag` value of each specific data service.

Only used when referenced conditionally from `provisioning_request_data()`
(Table 10-3) with `component_mode != 0`.
